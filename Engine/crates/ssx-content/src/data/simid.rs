use std::collections::BTreeMap;
use std::fmt;

use crate::hash::ContentHash;

pub const DOMAIN: &[u8] = b"project-avalanche/sim-content-id/1\n";

pub const MAX_ROWS: usize = 1 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimRow {
    pub id: String,
    pub class: String,
    pub hash: ContentHash,
    pub layer: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SimContentId(pub [u8; 32]);

impl SimContentId {
    pub fn of(rows: &[SimRow]) -> Result<SimContentId, String> {
        if rows.len() > MAX_ROWS {
            return Err(format!("{} inputs (at most {MAX_ROWS})", rows.len()));
        }
        let mut sorted: BTreeMap<(&str, &str), &ContentHash> = BTreeMap::new();
        for r in rows {
            if let Some(prev) = sorted.insert((&r.id, &r.class), &r.hash)
                && *prev != r.hash
            {
                return Err(format!("{} ({}) has two values", r.id, r.class));
            }
        }
        let mut h = blake3::Hasher::new();
        h.update(DOMAIN);
        h.update(&(sorted.len() as u64).to_le_bytes());
        for ((id, class), hash) in sorted {
            field(&mut h, id.as_bytes());
            field(&mut h, class.as_bytes());
            h.update(&hash.0);
        }
        Ok(SimContentId(*h.finalize().as_bytes()))
    }

    pub fn short(&self) -> String {
        self.0[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
}

fn field(h: &mut blake3::Hasher, bytes: &[u8]) {
    h.update(&(bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

impl fmt::Display for SimContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for SimContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SimContentId({self})")
    }
}

impl std::str::FromStr for SimContentId {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let b = super::check::parse_hex(s).filter(|b| b.len() == 32).ok_or("64 lower-case hex digits expected")?;
        let mut out = [0; 32];
        out.copy_from_slice(&b);
        Ok(SimContentId(out))
    }
}

pub fn record_hash(table: &toml::Table) -> ContentHash {
    let mut out = Vec::new();
    canonical_table(table, &mut out);
    ContentHash::of(&out)
}

fn canonical_table(t: &toml::Table, out: &mut Vec<u8>) {
    let mut keys: Vec<&String> = t.keys().collect();
    keys.sort();
    out.push(b'{');
    out.extend((keys.len() as u64).to_le_bytes());
    for k in keys {
        out.extend((k.len() as u64).to_le_bytes());
        out.extend(k.as_bytes());
        if let Some(v) = t.get(k) {
            canonical(v, out);
        }
    }
}

fn canonical(v: &toml::Value, out: &mut Vec<u8>) {
    match v {
        toml::Value::String(s) => {
            out.push(b's');
            out.extend((s.len() as u64).to_le_bytes());
            out.extend(s.as_bytes());
        }
        toml::Value::Integer(i) => {
            out.push(b'i');
            out.extend(i.to_le_bytes());
        }
        toml::Value::Float(x) => {
            out.push(b'f');
            out.extend(x.to_bits().to_le_bytes());
        }
        toml::Value::Boolean(b) => out.extend([b'b', u8::from(*b)]),
        toml::Value::Datetime(d) => {
            let s = d.to_string();
            out.push(b'd');
            out.extend((s.len() as u64).to_le_bytes());
            out.extend(s.as_bytes());
        }
        toml::Value::Array(a) => {
            out.push(b'[');
            out.extend((a.len() as u64).to_le_bytes());
            a.iter().for_each(|x| canonical(x, out));
        }
        toml::Value::Table(t) => canonical_table(t, out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, class: &str, b: u8) -> SimRow {
        SimRow { id: id.into(), class: class.into(), hash: ContentHash([b; 32]), layer: "L0".into() }
    }

    #[test]
    fn order_and_layer_do_not_matter_values_do() {
        let a = SimContentId::of(&[row("ssx:tuning/a", "simulation", 1), row("ssx:tuning/b", "simulation", 2)]);
        let mut swapped = vec![row("ssx:tuning/b", "simulation", 2), row("ssx:tuning/a", "simulation", 1)];
        swapped[0].layer = "L2".into();
        assert_eq!(a, SimContentId::of(&swapped));
        assert_ne!(a, SimContentId::of(&[row("ssx:tuning/a", "simulation", 1), row("ssx:tuning/b", "simulation", 3)]));
        assert_ne!(SimContentId::of(&[row("ab", "c", 0)]), SimContentId::of(&[row("a", "bc", 0)]));
        assert!(SimContentId::of(&[row("x", "s", 1), row("x", "s", 2)]).is_err());
        assert!(SimContentId::of(&[row("x", "s", 1), row("x", "s", 1)]).is_ok(), "the same row twice is one input");
        let id = a.unwrap();
        assert_eq!(id.to_string().parse::<SimContentId>(), Ok(id));
        assert_eq!(id.short().len(), 16);
    }

    #[test]
    fn records_hash_by_value_not_by_spelling() {
        let a: toml::Table = toml::from_str("b = 1.5\na = [1, 2]\n").unwrap();
        let b: toml::Table = toml::from_str("a = [ 1,2 ]   # x\nb = 1.50\n").unwrap();
        assert_eq!(record_hash(&a), record_hash(&b));
    }
}
