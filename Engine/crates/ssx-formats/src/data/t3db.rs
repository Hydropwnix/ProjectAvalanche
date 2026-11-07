use std::collections::HashMap;

use crate::reader::Reader;

pub mod format;
mod write;
use crate::{Error, Result};
pub use write::FLAG_WRITTEN;

pub fn crc32(data: &[u8], mut crc: u32) -> u32 {
    for &b in data {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 { (crc << 1) ^ 0x04C1_1DB7 } else { crc << 1 };
        }
    }
    crc
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub kind: u32,
    pub bit_offset: u32,
    pub name: [u8; 4],
    pub bits: u32,
}

pub const KIND_INTEGER: u32 = 3;
pub const KIND_STRING: u32 = 13;
pub const KIND_LONG_STRING: u32 = 14;

impl Field {
    pub fn name_str(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    pub fn in_blob(&self) -> bool {
        matches!(self.kind, KIND_STRING | KIND_LONG_STRING)
    }

    pub fn row_bits(&self) -> u32 {
        if self.in_blob() { 32 } else { self.bits }
    }

    pub fn word(&self, rows: &[u8], row_bytes: u32, row: usize) -> Option<u32> {
        let at = row * row_bytes as usize;
        self.get(rows.get(at..at + row_bytes as usize)?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: [u8; 4],
    pub flags: u32,
    pub row_bytes: u32,
    pub deleted_bit: u32,
    pub blob_bytes: u32,
    pub row_count: u16,
    pub rows_used: u16,
    pub free_count: u16,
    pub free_head: u16,
    pub index_count: u8,
    pub unk_tail: [u8; 6],
    pub fields: Vec<Field>,
    pub rows: Vec<u8>,
    pub blob: Vec<u8>,
}

impl Table {
    pub fn name_str(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }

    pub fn field(&self, short: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == short.as_bytes())
    }

    pub fn data_bits(&self) -> u32 {
        self.fields.iter().map(|f| f.bit_offset + f.row_bits()).max().unwrap_or(0)
    }

    pub fn blob_from(&self, offset: u32) -> Option<&[u8]> {
        self.blob.get(offset as usize..)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Database {
    pub unk_header: [u8; 3],
    pub size: u32,
    pub unk_0c: u32,
    pub tables: Vec<Table>,
}

fn check_crc(r: &mut Reader, block: &[u8], what: &str) -> Result<()> {
    let at = r.pos();
    let stored = r.u32()?;
    let crc = crc32(block, 0xFFFF_FFFF);
    if stored != crc {
        return Err(Error::invalid(format!("t3db: {what} CRC {stored:#010x} at {at:#x}, computed {crc:#010x}")));
    }
    Ok(())
}

impl Database {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(b"DB")?;
        let version: [u8; 3] = r.array()?;
        if version != [0, 8, 1] {
            return Err(Error::invalid(format!("t3db: header bytes 2..5 {version:02x?}, the loader wants 00 08 01")));
        }
        let unk_header: [u8; 3] = r.array()?;
        let size = r.u32()?;
        let unk_0c = r.u32()?;
        let count = r.u32()? as usize;
        check_crc(&mut r, &data[..0x14], "header")?;
        let index_start = r.pos();
        let mut index = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            index.push((r.array::<4>()?, r.u32()? as usize));
        }
        let index_end = r.pos();
        check_crc(&mut r, &data[index_start..index_end], "table index")?;
        let base = r.pos();
        let mut tables = Vec::with_capacity(index.len());
        for (name, offset) in index {
            r.seek(base + offset)?;
            tables.push(Self::read_table(&mut r, data, name)?);
        }
        if r.pos() != data.len() {
            return Err(Error::invalid(format!("t3db: {} bytes after the last table", data.len() - r.pos())));
        }
        if size as usize != data.len() {
            return Err(Error::invalid(format!("t3db: header size {size:#x}, stream {:#x}", data.len())));
        }
        Ok(Database { unk_header, size, unk_0c, tables })
    }

    fn read_table(r: &mut Reader, data: &[u8], name: [u8; 4]) -> Result<Table> {
        let head = r.pos();
        let flags = r.u32()?;
        let row_bytes = r.u32()?;
        let deleted_bit = r.u32()?;
        let blob_bytes = r.u32()?;
        let row_count = r.u16()?;
        let (rows_used, free_count, free_head) = (r.u16()?, r.u16()?, r.u16()?);
        let field_count = r.u8()?;
        let index_count = r.u8()?;
        let unk_tail: [u8; 6] = r.array()?;
        check_crc(r, &data[head..head + 32], "table header")?;
        let body = r.pos();
        let mut fields = Vec::with_capacity(usize::from(field_count));
        for _ in 0..field_count {
            fields.push(Field { kind: r.u32()?, bit_offset: r.u32()?, name: r.array()?, bits: r.u32()? });
        }
        let rows = r.bytes(usize::from(row_count) * row_bytes as usize)?.to_vec();
        let mut blob = Vec::new();
        if flags & 0x60 != 0 && blob_bytes != 0 {
            blob = r.bytes(blob_bytes as usize)?.to_vec();
            if !blob_bytes.is_multiple_of(8) {
                r.skip(8 - blob_bytes as usize % 8)?;
            }
        }
        if index_count != 0 {
            return Err(Error::invalid(format!("t3db: table {name:?} has {index_count} indices (not decoded)")));
        }
        let end = r.pos();
        check_crc(r, &data[body..end], "table body")?;
        let t = Table {
            name,
            flags,
            row_bytes,
            deleted_bit,
            blob_bytes,
            row_count,
            rows_used,
            free_count,
            free_head,
            index_count,
            unk_tail,
            fields,
            rows,
            blob,
        };
        for f in &t.fields {
            let known = matches!(f.kind, KIND_INTEGER | KIND_STRING | KIND_LONG_STRING);
            if !known || f.bits == 0 || (!f.in_blob() && f.bits > 32) || f.bit_offset + f.row_bits() > row_bytes * 8 {
                return Err(Error::invalid(format!(
                    "t3db: field {} of {} outside its row",
                    f.name_str(),
                    t.name_str()
                )));
            }
        }
        Ok(t)
    }

    pub fn table(&self, short: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == short.as_bytes())
    }
}

// TODO(guess): how the game's T3DB library bounds the tree (a size word, or the flag 0x40 vs 0x20) is not traced; the port takes the reachable nodes; settle: the string field reader under the query 0x83021C80.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringTree {
    nodes: Vec<[std::result::Result<u8, u8>; 2]>,
}

impl StringTree {
    pub fn parse(blob: &[u8]) -> Result<Self> {
        let entry = |node: usize, bit: usize| -> Result<std::result::Result<u8, u8>> {
            let at = node * 4 + bit * 2;
            let b = blob
                .get(at..at + 2)
                .ok_or_else(|| Error::invalid(format!("t3db: string tree node {node} past the blob")))?;
            match (b[0], b[1]) {
                (0, c) => Ok(Err(c)),
                (n, 0) => Ok(Ok(n)),
                (n, x) => Err(Error::invalid(format!("t3db: string tree node {node} entry {n:#x} {x:#x}"))),
            }
        };
        let mut nodes: Vec<Option<[std::result::Result<u8, u8>; 2]>> = Vec::new();
        let mut stack = vec![0usize];
        while let Some(n) = stack.pop() {
            if nodes.get(n).is_some_and(Option::is_some) {
                continue;
            }
            let pair = [entry(n, 0)?, entry(n, 1)?];
            if nodes.len() <= n {
                nodes.resize(n + 1, None);
            }
            nodes[n] = Some(pair);
            stack.extend(pair.iter().filter_map(|e| e.ok()).map(usize::from));
        }
        let nodes = nodes
            .into_iter()
            .enumerate()
            .map(|(i, n)| n.ok_or_else(|| Error::invalid(format!("t3db: string tree node {i} unreachable"))))
            .collect::<Result<Vec<_>>>()?;
        Ok(StringTree { nodes })
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn decode(&self, data: &[u8], len: usize) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len);
        let (mut node, mut bit) = (0usize, 0usize);
        while out.len() < len {
            let byte = *data.get(bit / 8).ok_or_else(|| Error::invalid("t3db: coded string past the blob"))?;
            let b = usize::from((byte >> (7 - bit % 8)) & 1);
            bit += 1;
            match self.nodes[node][b] {
                Ok(n) => node = usize::from(n),
                Err(c) => {
                    out.push(c);
                    node = 0;
                }
            }
        }
        Ok(out)
    }
}

impl Table {
    pub fn string(&self, tree: &StringTree, field: &Field, row: usize) -> Result<Option<Vec<u8>>> {
        let Some(off) = field.word(&self.rows, self.row_bytes, row) else {
            return Err(Error::invalid(format!("t3db: field {} is not a 32-bit word", field.name_str())));
        };
        if off == u32::MAX {
            return Ok(None);
        }
        let at = off as usize;
        let (len, start) = match field.kind {
            KIND_STRING => (self.blob.get(at).map(|&b| usize::from(b)), at + 1),
            KIND_LONG_STRING => {
                (self.blob.get(at..at + 2).map(|b| usize::from(u16::from_be_bytes([b[0], b[1]]))), at + 2)
            }
            k => return Err(Error::invalid(format!("t3db: field kind {k} is not a string"))),
        };
        let len = len.ok_or_else(|| Error::invalid(format!("t3db: string offset {off:#x} past the blob")))?;
        let data = self.blob.get(start..).unwrap_or_default();
        tree.decode(data, len).map(Some)
    }
}

pub fn loc_hash(key: &str) -> u32 {
    let mut crc = 0u32;
    for &c in key.as_bytes() {
        let mut x = (crc ^ u32::from(c & !0x20)) & 0xFF;
        for _ in 0..8 {
            x = if x & 1 != 0 { (x >> 1) ^ 0xEDB8_8320 } else { x >> 1 };
        }
        crc = x ^ (crc >> 8);
    }
    crc
}

#[derive(Clone, Debug, Default)]
pub struct LocStrings {
    pub by_hash: HashMap<u32, (String, Option<String>)>,
    pub tree_bytes: usize,
    pub hash_mismatches: usize,
}

impl LocStrings {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let db = Database::parse(data)?;
        let t = db.table("GJCv").ok_or_else(|| Error::invalid("t3db: no LanguageStrings table (GJCv)"))?;
        if t.flags & 0x40 == 0 {
            return Err(Error::invalid(format!("t3db: LanguageStrings flags {:#x} (coded strings expected)", t.flags)));
        }
        let field = |n: &str| t.field(n).ok_or_else(|| Error::invalid(format!("t3db: LanguageStrings has no {n}")));
        let (hash, id, text) = (field("jKhj")?, field("VhAs")?, field("bYbZ")?);
        let tree = StringTree::parse(&t.blob)?;
        let tree_bytes = tree.node_count() * 4;
        let mut out = LocStrings { by_hash: HashMap::new(), tree_bytes, hash_mismatches: 0 };
        let utf8 =
            |b: Vec<u8>| String::from_utf8(b).map_err(|e| Error::invalid(format!("t3db: string not UTF-8: {e}")));
        for row in 0..usize::from(t.row_count) {
            let h = hash.word(&t.rows, t.row_bytes, row).ok_or_else(|| Error::invalid("t3db: hashid"))?;
            let key = t.string(&tree, id, row)?.map(utf8).transpose()?.unwrap_or_default();
            let value = t.string(&tree, text, row)?.map(utf8).transpose()?;
            if loc_hash(&key) != h {
                out.hash_mismatches += 1;
            }
            out.by_hash.insert(h, (key, value));
        }
        Ok(out)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.by_hash.get(&loc_hash(key)).map(|(_, t)| t.as_deref().unwrap_or(""))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaField {
    pub name: String,
    pub short: String,
    pub kind: String,
    pub depth: u32,
    pub range: (i64, i64),
    pub key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaTable {
    pub name: String,
    pub short: String,
    pub max_insert: Option<i64>,
    pub save_groups: Option<String>,
    pub fields: Vec<MetaField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaSchema {
    pub name: String,
    pub short: String,
    pub version: i64,
    pub tables: Vec<MetaTable>,
    pub indices: Vec<(String, String, Vec<String>)>,
}

impl MetaSchema {
    pub fn parse(text: &str) -> Result<Self> {
        let root = crate::xml::parse(text.trim_start_matches('\u{feff}'))?;
        let attr = |e: &crate::xml::Element, k: &str| -> Result<String> {
            e.attr(k).map(str::to_string).ok_or_else(|| Error::invalid(format!("t3db meta: <{}> without {k}", e.name)))
        };
        let int = |e: &crate::xml::Element, k: &str| -> Result<i64> {
            attr(e, k)?.parse().map_err(|_| Error::invalid(format!("t3db meta: {k} is not an integer")))
        };
        if root.name != "database" {
            return Err(Error::invalid("t3db meta: root is not <database>"));
        }
        let mut tables = Vec::new();
        for t in root.children_named("table") {
            let mut fields = Vec::new();
            for fs in t.children_named("fields") {
                for f in fs.children_named("field") {
                    fields.push(MetaField {
                        name: attr(f, "name")?,
                        short: attr(f, "shortname")?,
                        kind: attr(f, "type")?,
                        depth: int(f, "depth")? as u32,
                        range: (int(f, "rangelow")?, int(f, "rangehigh")?),
                        key: f.attr("key") == Some("True"),
                    });
                }
            }
            tables.push(MetaTable {
                name: attr(t, "name")?,
                short: attr(t, "shortname")?,
                max_insert: t.attr("maxinsert").map(|_| int(t, "maxinsert")).transpose()?,
                save_groups: t.attr("savegroups").map(str::to_string),
                fields,
            });
        }
        let mut indices = Vec::new();
        for list in root.children_named("indices") {
            for ix in list.children_named("index") {
                let fields = ix
                    .children_named("indexfields")
                    .flat_map(|fs| fs.children_named("indexfield"))
                    .map(|f| attr(f, "shortname"))
                    .collect::<Result<Vec<_>>>()?;
                indices.push((attr(ix, "shortname")?, attr(ix, "tableshortname")?, fields));
            }
        }
        Ok(MetaSchema {
            name: attr(&root, "name")?,
            short: attr(&root, "shortname")?,
            version: int(&root, "version")?,
            tables,
            indices,
        })
    }

    pub fn table(&self, name: &str) -> Option<&MetaTable> {
        self.tables.iter().find(|t| t.name == name)
    }
}

pub fn check_schema(db: &Database, meta: &MetaSchema) -> Vec<String> {
    let mut out = Vec::new();
    for mt in &meta.tables {
        let Some(t) = db.table(&mt.short) else {
            out.push(format!("table {} ({}) missing", mt.name, mt.short));
            continue;
        };
        if t.fields.len() != mt.fields.len() {
            out.push(format!("{}: {} fields, schema {}", mt.name, t.fields.len(), mt.fields.len()));
        }
        for mf in &mt.fields {
            match t.field(&mf.short) {
                None => out.push(format!("{}.{} ({}) missing", mt.name, mf.name, mf.short)),
                Some(f) if f.bits != mf.depth => {
                    out.push(format!("{}.{}: {} bits, schema {}", mt.name, mf.name, f.bits, mf.depth))
                }
                Some(f)
                    if (f.kind == KIND_INTEGER) != (mf.kind == "FIELDTYPE_INTEGER")
                        || f.in_blob() != (mf.kind == "FIELDTYPE_STRING") =>
                {
                    out.push(format!("{}.{}: type {} / {}", mt.name, mf.name, f.kind, mf.kind))
                }
                _ => {}
            }
        }
        let mut order: Vec<&Field> = t.fields.iter().collect();
        order.sort_by(|a, b| (a.row_bits() != 32).cmp(&(b.row_bits() != 32)).then(a.name.cmp(&b.name)));
        let mut at = 0;
        for f in order {
            if f.bit_offset != at {
                out.push(format!(
                    "{}.{}: bit {} where the layout rule gives {at}",
                    mt.name,
                    f.name_str(),
                    f.bit_offset
                ));
            }
            at += f.row_bits();
        }
        if t.row_bytes != (t.data_bits() + 1).div_ceil(32) * 4 {
            out.push(format!("{}: row {} bytes for {} bits", mt.name, t.row_bytes, t.data_bits()));
        }
        if t.deleted_bit != t.row_bytes * 8 - 1 {
            out.push(format!("{}: header word 2 = {}", mt.name, t.deleted_bit));
        }
    }
    if db.tables.len() != meta.tables.len() {
        out.push(format!("{} tables, schema {}", db.tables.len(), meta.tables.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(row_count: u16, rows: &[u8]) -> Vec<u8> {
        let mut table = Vec::new();
        let mut head = Vec::new();
        for w in [2u32, 8, 63, 0] {
            head.extend(w.to_be_bytes());
        }
        head.extend(row_count.to_be_bytes());
        head.extend([0u8, 0, 0, 0, 0xFF, 0xFF, 1, 0, 0, 0, 0, 0, 0, 0]);
        table.extend(&head);
        table.extend(crc32(&head, !0).to_be_bytes());
        let mut body = Vec::new();
        body.extend(3u32.to_be_bytes());
        body.extend(0u32.to_be_bytes());
        body.extend(*b"syIv");
        body.extend(32u32.to_be_bytes());
        body.extend(rows);
        table.extend(&body);
        table.extend(crc32(&body, !0).to_be_bytes());
        let mut out = b"DB\0\x08\x01\0\0\0".to_vec();
        let total = 16 + 4 + 4 + 8 + 4 + table.len();
        out.extend((total as u32).to_be_bytes());
        out.extend(0u32.to_be_bytes());
        out.extend(1u32.to_be_bytes());
        out.extend(crc32(&out.clone(), !0).to_be_bytes());
        let index = [*b"MpJj", 0u32.to_be_bytes()].concat();
        out.extend(&index);
        out.extend(crc32(&index, !0).to_be_bytes());
        out.extend(table);
        out
    }

    #[test]
    fn crc_matches_mpeg2_check_value() {
        assert_eq!(crc32(b"123456789", 0xFFFF_FFFF), 0x0376_E6E7);
    }

    #[test]
    fn parses_and_checks_crcs() {
        let s = stream(1, &[0, 0, 0, 42, 0, 0, 0, 0]);
        let db = Database::parse(&s).unwrap();
        let t = db.table("MpJj").unwrap();
        assert_eq!((t.row_bytes, t.row_count, t.rows.len()), (8, 1, 8));
        assert_eq!(t.field("syIv").map(|f| f.bits), Some(32));
        let mut bad = s.clone();
        let n = bad.len();
        bad[n - 9] ^= 1;
        assert!(Database::parse(&bad).is_err());
        assert!(Database::parse(&s[..s.len() - 1]).is_err());
    }

    #[test]
    fn a_null_text_is_found_and_empty() {
        let mut s = LocStrings::default();
        s.by_hash.insert(loc_hash("TXT_A"), ("TXT_A".into(), None));
        s.by_hash.insert(loc_hash("TXT_B"), ("TXT_B".into(), Some("b".into())));
        assert_eq!(s.get("TXT_A"), Some(""));
        assert_eq!(s.get("txt_b"), Some("b"));
        assert_eq!(s.get("TXT_C"), None);
    }

    #[test]
    fn loc_hash_folds_case() {
        assert_eq!(loc_hash(""), 0);
        assert_eq!(loc_hash("TXT_GO_THERE"), loc_hash("txt_go_there"));
        assert_eq!(loc_hash("TXT_GO_THERE"), 0xE78E_5DFA);
    }

    #[test]
    fn string_tree_decodes_msb_first() {
        let blob = [0, b'a', 1, 0, 0, b'b', 0, b'c'];
        let tree = StringTree::parse(&blob).unwrap();
        assert_eq!(tree.node_count(), 2);
        assert_eq!(tree.decode(&[0b0111_0000], 4).unwrap(), b"acba");
        assert!(tree.decode(&[0b0111_0000], 9).is_err());
        assert!(StringTree::parse(&[2, 0, 0, b'x']).is_err());
    }

    #[test]
    fn schema_check() {
        let db = Database::parse(&stream(0, &[])).unwrap();
        let xml = "\u{feff}<?xml version=\"1.0\"?><database name=\"x\" shortname=\"y\" version=\"6\"><table name=\"equipment\" \
                   shortname=\"MpJj\" maxinsert=\"150\" savegroups=\"SSXV_PROFILE\"><fields><field name=\"NAME\" \
                   shortname=\"syIv\" type=\"FIELDTYPE_INTEGER\" depth=\"32\" rangelow=\"0\" rangehigh=\"1\" key=\"True\" />\
                   </fields></table></database>";
        let meta = MetaSchema::parse(xml).unwrap();
        assert_eq!(meta.tables[0].fields[0].name, "NAME");
        assert!(check_schema(&db, &meta).is_empty(), "{:?}", check_schema(&db, &meta));
    }
}
