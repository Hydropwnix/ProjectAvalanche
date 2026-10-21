use super::{Database, Field, KIND_INTEGER, MetaSchema, Table, crc32};
use crate::{Error, Result};

pub const FLAG_WRITTEN: u32 = 2;

impl Field {
    fn window(&self) -> (usize, u32) {
        let word = (self.bit_offset / 32 * 4) as usize;
        let shift = 64 - (self.bit_offset % 32) - self.row_bits();
        (word, shift)
    }

    fn mask(&self) -> u64 {
        (1u64 << self.row_bits()) - 1
    }

    pub fn get(&self, row: &[u8]) -> Option<u32> {
        if ((self.bit_offset + self.row_bits()) as usize).div_ceil(8) > row.len() {
            return None;
        }
        let (word, shift) = self.window();
        let mut w = [0u8; 8];
        let avail = row.len().saturating_sub(word).min(8);
        w[..avail].copy_from_slice(&row[word..word + avail]);
        Some(((u64::from_be_bytes(w) >> shift) & self.mask()) as u32)
    }

    pub fn put(&self, row: &mut [u8], value: u32) -> bool {
        if ((self.bit_offset + self.row_bits()) as usize).div_ceil(8) > row.len() {
            return false;
        }
        let (word, shift) = self.window();
        let mut w = [0u8; 8];
        let avail = row.len().saturating_sub(word).min(8);
        w[..avail].copy_from_slice(&row[word..word + avail]);
        let mask = self.mask() << shift;
        let merged = (u64::from(value) << shift) & mask | u64::from_be_bytes(w) & !mask;
        row[word..word + avail].copy_from_slice(&merged.to_be_bytes()[..avail]);
        true
    }
}

impl Table {
    pub fn row(&self, i: usize) -> Option<&[u8]> {
        let n = self.row_bytes as usize;
        self.rows.get(i * n..(i + 1) * n)
    }

    fn row_mut(&mut self, i: usize) -> Option<&mut [u8]> {
        let n = self.row_bytes as usize;
        self.rows.get_mut(i * n..(i + 1) * n)
    }

    pub fn is_deleted(&self, i: usize) -> bool {
        let bit = self.deleted_bit as usize;
        self.row(i).and_then(|r| r.get(bit / 8)).is_some_and(|b| b & (0x80 >> (bit % 8)) != 0)
    }

    pub fn live_rows(&self) -> impl Iterator<Item = usize> + '_ {
        (0..usize::from(self.rows_used)).filter(|&i| !self.is_deleted(i))
    }

    pub fn get(&self, i: usize, short: &str) -> Option<u32> {
        self.field(short)?.get(self.row(i)?)
    }

    pub fn put(&mut self, i: usize, short: &str, value: u32) -> Result<()> {
        let f = self.field(short).cloned().ok_or_else(|| Error::invalid(format!("t3db: no field {short}")))?;
        let name = self.name_str();
        let row = self.row_mut(i).ok_or_else(|| Error::invalid(format!("t3db: {name} has no row {i}")))?;
        f.put(row, value);
        Ok(())
    }

    pub fn reserve(&mut self, capacity: u16) {
        if capacity > self.row_count {
            self.row_count = capacity;
            self.rows.resize(usize::from(capacity) * self.row_bytes as usize, 0);
        }
    }

    pub fn insert_row(&mut self) -> Result<usize> {
        let i = if self.free_count != 0 {
            let head = usize::from(self.free_head);
            let next = self.row(head).map(|r| u16::from_be_bytes([r[0], r[1]])).unwrap_or(0xFFFF);
            self.free_count -= 1;
            self.free_head = next;
            head
        } else if self.rows_used < self.row_count {
            self.rows_used += 1;
            usize::from(self.rows_used - 1)
        } else {
            return Err(Error::invalid(format!("t3db: table {} is full ({} rows)", self.name_str(), self.row_count)));
        };
        if let Some(r) = self.row_mut(i) {
            r.fill(0);
        }
        Ok(i)
    }

    pub fn delete_row(&mut self, i: usize) -> Result<()> {
        let (bit, head) = (self.deleted_bit as usize, self.free_head);
        let name = self.name_str();
        let r = self.row_mut(i).ok_or_else(|| Error::invalid(format!("t3db: {name} has no row {i}")))?;
        r.fill(0);
        r[bit / 8] |= 0x80 >> (bit % 8);
        r[..2].copy_from_slice(&head.to_be_bytes());
        self.free_head = i as u16;
        self.free_count += 1;
        Ok(())
    }

    fn header(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(32);
        for w in [self.flags, self.row_bytes, self.deleted_bit, self.blob_bytes] {
            h.extend(w.to_be_bytes());
        }
        for w in [self.row_count, self.rows_used, self.free_count, self.free_head] {
            h.extend(w.to_be_bytes());
        }
        h.push(self.fields.len() as u8);
        h.push(self.index_count);
        h.extend(self.unk_tail);
        h
    }

    fn body(&self) -> Vec<u8> {
        let mut b = Vec::new();
        for f in &self.fields {
            b.extend(f.kind.to_be_bytes());
            b.extend(f.bit_offset.to_be_bytes());
            b.extend(f.name);
            b.extend(f.bits.to_be_bytes());
        }
        b.extend(&self.rows);
        if self.flags & 0x60 != 0 && self.blob_bytes != 0 {
            b.extend(&self.blob);
            b.resize(b.len() + (8 - self.blob.len() % 8) % 8, 0);
        }
        b
    }

    pub fn from_meta(meta: &super::MetaTable) -> Result<Self> {
        let mut fields = Vec::with_capacity(meta.fields.len());
        for mf in &meta.fields {
            let name: [u8; 4] = mf
                .short
                .as_bytes()
                .try_into()
                .map_err(|_| Error::invalid(format!("t3db meta: short name {} is not 4 bytes", mf.short)))?;
            if mf.kind != "FIELDTYPE_INTEGER" || !(1..=32).contains(&mf.depth) {
                return Err(Error::invalid(format!("t3db meta: field {} ({} {})", mf.name, mf.kind, mf.depth)));
            }
            fields.push(Field { kind: KIND_INTEGER, bit_offset: 0, name, bits: mf.depth });
        }
        fields.sort_by_key(|f| f.name);
        let mut order: Vec<usize> = (0..fields.len()).collect();
        order.sort_by_key(|&i| (fields[i].bits != 32, fields[i].name));
        let mut at = 0;
        for i in order {
            fields[i].bit_offset = at;
            at += fields[i].bits;
        }
        let row_bytes = (at + 1).div_ceil(32) * 4;
        let name = meta.short.as_bytes().try_into().map_err(|_| Error::invalid("t3db meta: table short name"))?;
        Ok(Table {
            name,
            flags: FLAG_WRITTEN,
            row_bytes,
            deleted_bit: row_bytes * 8 - 1,
            blob_bytes: 0,
            row_count: 0,
            rows_used: 0,
            free_count: 0,
            free_head: 0xFFFF,
            index_count: 0,
            unk_tail: [0; 6],
            fields,
            rows: Vec::new(),
            blob: Vec::new(),
        })
    }
}

impl Database {
    pub fn write(&self) -> Vec<u8> {
        let tables: Vec<&Table> = self.tables.iter().filter(|t| t.flags & FLAG_WRITTEN != 0).collect();
        let mut blocks = Vec::with_capacity(tables.len());
        for t in &tables {
            let (head, body) = (t.header(), t.body());
            let mut b = head.clone();
            b.extend(crc32(&head, !0).to_be_bytes());
            b.extend(&body);
            b.extend(crc32(&body, !0).to_be_bytes());
            blocks.push(b);
        }
        let index_len = tables.len() * 8;
        let total = 0x18 + index_len + 4 + blocks.iter().map(Vec::len).sum::<usize>();
        let mut out = Vec::with_capacity(total);
        out.extend(b"DB\0\x08\x01");
        out.extend(self.unk_header);
        out.extend((total as u32).to_be_bytes());
        out.extend(self.unk_0c.to_be_bytes());
        out.extend((tables.len() as u32).to_be_bytes());
        out.extend(crc32(&out.clone(), !0).to_be_bytes());
        let mut index = Vec::with_capacity(index_len);
        let mut offset = 0u32;
        for (t, b) in tables.iter().zip(&blocks) {
            index.extend(t.name);
            index.extend(offset.to_be_bytes());
            offset += b.len() as u32;
        }
        out.extend(&index);
        out.extend(crc32(&index, !0).to_be_bytes());
        for b in blocks {
            out.extend(b);
        }
        out
    }

    pub fn from_meta(meta: &MetaSchema, order: &[&str]) -> Result<Self> {
        let mut tables = Vec::with_capacity(order.len());
        for short in order {
            let mt = meta.tables.iter().find(|t| t.short == *short);
            let mt = mt.ok_or_else(|| Error::invalid(format!("t3db meta: no table {short}")))?;
            tables.push(Table::from_meta(mt)?);
        }
        let mut db = Database { unk_header: [0; 3], size: 0, unk_0c: 0, tables };
        db.size = db.write().len() as u32;
        Ok(db)
    }

    pub fn table_mut(&mut self, short: &str) -> Option<&mut Table> {
        self.tables.iter_mut().find(|t| t.name == short.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(bit_offset: u32, bits: u32) -> Field {
        Field { kind: KIND_INTEGER, bit_offset, name: *b"test", bits }
    }

    #[test]
    fn fields_pack_most_significant_bit_first() {
        let mut row = [0u8; 8];
        let f = field(0, 3);
        assert!(f.put(&mut row, 0b101));
        assert_eq!(row[0], 0b1010_0000);
        let g = field(5, 7);
        g.put(&mut row, 0x7F);
        assert_eq!(&row[..2], &[0b1010_0111, 0b1111_0000]);
        assert_eq!((f.get(&row), g.get(&row)), (Some(5), Some(0x7F)));
        let h = field(30, 4);
        h.put(&mut row, u32::MAX);
        assert_eq!(h.get(&row), Some(15));
        assert_eq!((row[3] & 3, row[4] >> 6), (3, 3));
        let w = field(32, 32);
        w.put(&mut row, 0x1234_5678);
        assert_eq!(&row[4..], &[0x12, 0x34, 0x56, 0x78]);
        assert!(!field(60, 8).put(&mut row, 1));
    }

    #[test]
    fn rows_insert_delete_and_reuse_the_free_list() {
        let meta = super::super::MetaTable {
            name: "attribs".into(),
            short: "GbQU".into(),
            max_insert: Some(4),
            save_groups: None,
            fields: ["XfbS", "syIv"]
                .iter()
                .map(|s| super::super::MetaField {
                    name: s.to_string(),
                    short: s.to_string(),
                    kind: "FIELDTYPE_INTEGER".into(),
                    depth: 32,
                    range: (0, 0),
                    key: false,
                })
                .collect(),
        };
        let mut t = Table::from_meta(&meta).unwrap();
        assert_eq!((t.row_bytes, t.deleted_bit), (12, 95));
        t.reserve(3);
        let a = t.insert_row().unwrap();
        let b = t.insert_row().unwrap();
        t.put(b, "syIv", 7).unwrap();
        t.delete_row(a).unwrap();
        assert!(t.is_deleted(a));
        assert_eq!(t.live_rows().collect::<Vec<_>>(), vec![b]);
        assert_eq!((t.free_count, t.free_head), (1, a as u16));
        assert_eq!(t.insert_row().unwrap(), a);
        assert_eq!((t.free_count, t.free_head, t.rows_used), (0, 0xFFFF, 2));
        assert_eq!(t.get(b, "syIv"), Some(7));
        t.insert_row().unwrap();
        assert!(t.insert_row().is_err());
    }

    #[test]
    fn written_streams_parse_back() {
        let xml = "<database name=\"x\" shortname=\"y\" version=\"6\"><table name=\"equipment\" shortname=\"MpJj\" \
                   maxinsert=\"2\"><fields><field name=\"NAME\" shortname=\"syIv\" type=\"FIELDTYPE_INTEGER\" \
                   depth=\"32\" rangelow=\"0\" rangehigh=\"1\" key=\"True\" /><field name=\"N\" shortname=\"aaaa\" \
                   type=\"FIELDTYPE_INTEGER\" depth=\"5\" rangelow=\"0\" rangehigh=\"20\" /></fields></table></database>";
        let meta = MetaSchema::parse(xml).unwrap();
        let mut db = Database::from_meta(&meta, &["MpJj"]).unwrap();
        assert!(super::super::check_schema(&db, &meta).is_empty());
        let t = db.table_mut("MpJj").unwrap();
        t.reserve(2);
        let r = t.insert_row().unwrap();
        t.put(r, "syIv", 0xDEAD_BEEF).unwrap();
        t.put(r, "aaaa", 17).unwrap();
        let bytes = db.write();
        let back = Database::parse(&bytes).unwrap();
        assert_eq!(back.write(), bytes);
        assert_eq!(back.table("MpJj").unwrap().get(r, "aaaa"), Some(17));
        assert_eq!(back.table("MpJj").unwrap().get(r, "syIv"), Some(0xDEAD_BEEF));
    }
}
