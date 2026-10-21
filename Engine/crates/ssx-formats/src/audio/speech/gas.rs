use super::{Coverage, u16_at, u32_at};
use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub a: i16,
    pub op: i16,
    pub kind: i16,
    pub index: i16,
    pub value: i32,
}

#[derive(Clone, Debug)]
pub struct Statement {
    pub records: Vec<Record>,
}

#[derive(Clone, Debug)]
pub struct Script {
    pub id: u32,
    pub unk_04: u32,
    pub params: Vec<(u32, u32)>,
    pub locals: Vec<(u32, u32)>,
    pub unk_10: u32,
    pub statements: Vec<Statement>,
}

#[derive(Clone, Debug)]
pub struct GasFile {
    pub version: (u16, u16),
    pub chunks: Vec<([u8; 4], u32)>,
    pub css_unk: (u32, u32),
    pub scripts: Vec<Script>,
}

impl GasFile {
    pub fn parse(d: &[u8]) -> Result<Self> {
        let (z, nch) = (u32_at(d, 0)?, u32_at(d, 4)? as usize);
        let version = (u16_at(d, 8)?, u16_at(d, 10)?);
        if z != 0 || nch == 0 || nch > 16 {
            return Err(Error::invalid(format!("GAS header {z:#x}, {nch} chunks")));
        }
        let mut chunks = Vec::with_capacity(nch);
        for i in 0..nch {
            let f: [u8; 4] = d
                .get(0xC + 8 * i..0x10 + 8 * i)
                .and_then(|b| b.try_into().ok())
                .ok_or(Error::invalid("GAS chunk table"))?;
            chunks.push((f, u32_at(d, 0x10 + 8 * i)?));
        }
        let css = chunks
            .iter()
            .find(|c| &c.0 == b"CSS_")
            .map(|c| c.1 as usize)
            .ok_or(Error::invalid("GAS: no CSS_ chunk"))?;
        let mut cov = Coverage::default();
        cov.claim(0, 0xC + 8 * nch);
        let (u0, u4, n, first) =
            (u32_at(d, css)?, u32_at(d, css + 4)?, u32_at(d, css + 8)? as usize, u32_at(d, css + 12)?);
        cov.claim(css, css + 16 + 4 * n);
        if first as usize != css + 16 + 4 * n {
            return Err(Error::invalid(format!("GAS: first script at {first:#x}")));
        }
        let mut scripts = Vec::with_capacity(n);
        for i in 0..n {
            let o = u32_at(d, css + 16 + 4 * i)? as usize;
            scripts.push(Self::script(d, o, &mut cov)?);
        }
        let mut ids: Vec<u32> = scripts.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err(Error::invalid("GAS: duplicate script ids"));
        }
        cov.check(d.len(), "GAS")?;
        Ok(Self { version, chunks, css_unk: (u0, u4), scripts })
    }

    fn script(d: &[u8], o: usize, cov: &mut Coverage) -> Result<Script> {
        let w = (0..7).map(|k| u32_at(d, o + 4 * k)).collect::<Result<Vec<_>>>()?;
        let (id, unk_04, np, nl, unk_10, ns, size) =
            (w[0], w[1], w[2] as usize, w[3] as usize, w[4], w[5] as usize, w[6]);
        let pair = |p: usize| Ok::<_, Error>((u32_at(d, p)?, u32_at(d, p + 4)?));
        let params = (0..np).map(|k| pair(o + 0x1C + 8 * k)).collect::<Result<Vec<_>>>()?;
        let locals = (0..nl).map(|k| pair(o + 0x1C + 8 * (np + k))).collect::<Result<Vec<_>>>()?;
        let mut p = o + 0x1C + 8 * (np + nl);
        let mut statements = Vec::with_capacity(ns);
        let mut records = 0usize;
        for _ in 0..ns {
            let (sz, nr) = (u32_at(d, p)? as usize, u32_at(d, p + 4)? as usize);
            if sz != 8 + 12 * nr || nr == 0 {
                return Err(Error::invalid(format!("GAS script {id}: statement at {p:#x} size {sz} for {nr} records")));
            }
            let mut recs = Vec::with_capacity(nr);
            for r in 0..nr {
                let q = p + 8 + 12 * r;
                let h = |k| u16_at(d, q + k).map(|v| v as i16);
                recs.push(Record { a: h(0)?, op: h(2)?, kind: h(4)?, index: h(6)?, value: u32_at(d, q + 8)? as i32 });
            }
            records += nr;
            statements.push(Statement { records: recs });
            p += sz;
        }
        let decl = |i: usize, op: i16, n: usize| {
            statements.get(i).is_some_and(|s| s.records[0].a == -1 && s.records[0].op == op && s.records.len() == 1 + n)
        };
        if size as usize != 12 * records
            || !decl(0, 5, np)
            || !decl(1, 7, nl)
            || unk_10 as usize != o + 0x1C + 4 * np + 8 * nl
        {
            return Err(Error::invalid(format!(
                "GAS script {id} at {o:#x}: size {size}, {records} records, declarations"
            )));
        }
        cov.claim(o, p);
        Ok(Script { id, unk_04, params, locals, unk_10, statements })
    }

    pub fn script_by_id(&self, id: u32) -> Option<&Script> {
        self.scripts.iter().find(|s| s.id == id)
    }
}
