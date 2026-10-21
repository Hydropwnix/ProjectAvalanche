use super::collection::{self, ENTRY_ARRAY, ENTRY_INLINE, ENTRY_REFERENCE};
use super::container::{FILE_BIN, FILE_VLT, PtrTarget};
use super::*;

const DATN_PAYLOAD: usize = 88;

struct Builder {
    strings: Vec<u8>,
    datn: Vec<u8>,
    bin_data: Vec<u8>,
    exports: Vec<[u32; 4]>,
    fixups: Vec<(u16, u32, u32)>,
}

impl Builder {
    fn new(strings: &[&str]) -> Self {
        let mut s: Vec<u8> = strings.iter().flat_map(|x| x.bytes().chain([0])).collect();
        s.resize(s.len().next_multiple_of(16), 0);
        Builder { strings: s, datn: Vec::new(), bin_data: Vec::new(), exports: Vec::new(), fixups: Vec::new() }
    }
    fn bin_base(&self) -> u32 {
        (8 + self.strings.len() + 16) as u32
    }
    fn string_at(&self, s: &str) -> u32 {
        let pos = self.strings.windows(s.len() + 1).position(|w| &w[..s.len()] == s.as_bytes() && w[s.len()] == 0);
        8 + pos.unwrap() as u32
    }
    fn export(&mut self, key: u32, ty: u32, bytes: &[u8]) -> u32 {
        self.datn.resize(self.datn.len().next_multiple_of(8), 0);
        let off = (DATN_PAYLOAD + self.datn.len()) as u32;
        self.datn.extend_from_slice(bytes);
        self.exports.push([key, ty, bytes.len() as u32, off]);
        off
    }
    fn bin(&mut self, bytes: &[u8]) -> u32 {
        self.bin_data.resize(self.bin_data.len().next_multiple_of(16), 0);
        let off = self.bin_base() + self.bin_data.len() as u32;
        self.bin_data.extend_from_slice(bytes);
        off
    }
    fn ptr(&mut self, file: u16, slot: u32, target: u32) {
        self.fixups.push((file, slot, target));
    }
    fn build(&self) -> (Vec<u8>, Vec<u8>) {
        let mut v = Vec::new();
        let chunk = |v: &mut Vec<u8>, tag: &[u8], payload: &[u8]| {
            v.extend_from_slice(tag);
            v.extend_from_slice(&(payload.len() as u32 + 8).to_be_bytes());
            v.extend_from_slice(payload);
        };
        chunk(&mut v, b"Vers", &[0x2c, 0x82, 0x1e, 0x4b, 0x12, 0x0c, 0x4f, 0xd6]);
        chunk(&mut v, b"StrN", &[0; 8]);
        let mut dep = be(&[2, fnv1(b"t.vlt"), fnv1(b"t.bin"), 0, 6]);
        dep.extend_from_slice(b"t.vlt\0t.bin\0");
        dep.resize(40, 0);
        chunk(&mut v, b"DepN", &dep);
        assert_eq!(v.len() + 8, DATN_PAYLOAD);
        let mut datn = self.datn.clone();
        datn.resize(datn.len().next_multiple_of(16), 0);
        chunk(&mut v, b"DatN", &datn);
        let mut exp = be(&[self.exports.len() as u32]);
        self.exports.iter().for_each(|e| exp.extend(be(e)));
        exp.resize((exp.len() + 8).next_multiple_of(16) - 8, 0);
        chunk(&mut v, b"ExpN", &exp);
        let mut ptr = Vec::new();
        for file in [FILE_BIN, FILE_VLT] {
            ptr.extend(be(&[0, (2 << 16) | file as u32, 0]));
            for &(_, slot, target) in self.fixups.iter().filter(|x| x.0 == file) {
                ptr.extend(be(&[slot, (3 << 16) | FILE_BIN as u32, target]));
            }
        }
        ptr.extend([0; 12]);
        chunk(&mut v, b"PtrN", &ptr);
        chunk(&mut v, b"EndC", &[0; 8]);
        let mut b = Vec::new();
        chunk(&mut b, b"StrE", &self.strings);
        chunk(&mut b, b"EndC", &[0; 8]);
        b.extend_from_slice(&self.bin_data);
        (v, b)
    }
}

fn be(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

#[test]
fn container_and_pointer_fixups() {
    let mut bld = Builder::new(&["", "hello"]);
    let hello = bld.string_at("hello");
    let obj = bld.export(0x1234, 0x5678, &[0; 8]);
    bld.ptr(FILE_VLT, obj + 4, hello);
    let (vlt, bin) = bld.build();
    let v = Vault::parse(&vlt, &bin).unwrap();
    assert_eq!(v.deps[1].name, "t.bin");
    assert_eq!(v.exports, vec![Export { key: 0x1234, type_hash: 0x5678, size: 8, offset: obj }]);
    assert_eq!(v.bin_strings.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(), ["", "hello"]);
    let t = v.pointer(FILE_VLT, obj + 4).unwrap();
    assert_eq!(t, PtrTarget { file: FILE_BIN, offset: hello });
    assert_eq!(v.cstr(t).unwrap(), "hello");
    assert_eq!(v.pointer(FILE_VLT, obj), None);

    let mut bad = vlt.clone();
    bad[obj as usize + 7] = 1;
    assert!(Vault::parse(&bad, &bin).is_err());
}

const CLASS: u32 = fnv1(b"c");
const TYPES: [(&str, u32); 5] = [
    ("EA::Reflection::Float", 4),
    ("EA::Reflection::Text", 4),
    ("Attrib::Types::Vector3", 12),
    ("Attrib::RefSpec", 12),
    ("EA::Reflection::Bool", 1),
];

fn th(i: usize) -> u32 {
    fnv1(TYPES[i].0.as_bytes())
}

fn schema_pair() -> (Vec<u8>, Vec<u8>) {
    let mut b = Builder::new(&[]);
    let names: Vec<u8> = TYPES.iter().flat_map(|t| t.0.bytes().chain([0])).collect();
    let names_at = b.bin(&names);
    let mut db = be(&[207, 216, TYPES.len() as u32, 0]);
    db.extend(be(&TYPES.map(|t| t.1)));
    let db_at = b.export(EXPORT_DATABASE ^ 1, EXPORT_DATABASE, &db);
    b.ptr(FILE_VLT, db_at + 12, names_at);
    let def = |key: &[u8], ty: usize, off: u32, size: u32, max: u32, flags: u32, align: u32| {
        be(&[fnv1(key), th(ty), (off << 16) | size, (max << 16) | (flags << 8) | align])
    };
    let defs = [
        def(b"name", 1, 0, 4, 1, 0x06, 2),
        def(b"link", 3, 4, 12, 1, 0x26, 2),
        def(b"speed", 0, 0, 4, 1, 0x04, 2),
        def(b"dir", 2, 0, 12, 1, 0x04, 2),
        def(b"refs", 3, 0, 12, 0xffff, 0x25, 2),
        def(b"on", 4, 0, 1, 1, 0x04, 0),
    ]
    .concat();
    let defs_at = b.bin(&defs);
    let class_at = b.export(CLASS, EXPORT_CLASS, &be(&[CLASS, 2, 6, 0, 0, 0, 16, 2]));
    b.ptr(FILE_VLT, class_at + 12, defs_at);
    b.build()
}

#[test]
fn schema_collection_decode_and_inheritance() {
    let (svlt, sbin) = schema_pair();
    let sv = Vault::parse(&svlt, &sbin).unwrap();
    let schema = Schema::from_vault(&sv).unwrap();
    assert_eq!(schema.types.len(), 5);
    assert_eq!(schema.class(CLASS).unwrap().layout_size, 16);

    let (x, child) = (fnv1(b"x"), fnv1(b"child"));
    let mut b = Builder::new(&["xname"]);
    let name_at = b.string_at("xname");
    let layout = b.bin(&be(&[0, CLASS, child, 0]));
    b.ptr(FILE_BIN, layout, name_at);
    let child_layout = b.bin(&be(&[0, CLASS, 0, 0]));
    let dir = b.bin(&be(&[0, 1.0f32.to_bits(), 0]));
    let refs = b.bin(&[be(&[0x0001_0001, 0x000c_0000]), be(&[CLASS, x, 0])].concat());
    let mut col = be(&[x, CLASS, 0, 4, 0, 4, (4 << 16) | 4, 0, th(0), th(2), th(3), th(4)]);
    col.extend(be(&[fnv1(b"speed"), 1.5f32.to_bits(), (ENTRY_INLINE as u32) << 8]));
    col.extend(be(&[fnv1(b"dir"), 0, 1 << 16]));
    col.extend(be(&[fnv1(b"refs"), 0, (2 << 16) | ((ENTRY_ARRAY | ENTRY_REFERENCE) as u32) << 8]));
    col.extend(be(&[fnv1(b"on"), 0x0100_0000, (3 << 16) | (ENTRY_INLINE as u32) << 8]));
    let at = b.export(fnv1(b"c/x"), EXPORT_COLLECTION, &col);
    b.ptr(FILE_VLT, at + 0x1c, layout);
    b.ptr(FILE_VLT, at + 0x20 + 16 + 12 + 4, dir);
    b.ptr(FILE_VLT, at + 0x20 + 16 + 24 + 4, refs);
    let mut ccol = be(&[child, CLASS, x, 1, 0, 1, (1 << 16) | 2, 0, th(0), 0]);
    ccol.extend(be(&[fnv1(b"speed"), 2.0f32.to_bits(), (ENTRY_INLINE as u32) << 8]));
    let cat = b.export(fnv1(b"c/child"), EXPORT_COLLECTION, &ccol);
    b.ptr(FILE_VLT, cat + 0x1c, child_layout);
    let (vlt, bin) = b.build();

    let v = Vault::parse(&vlt, &bin).unwrap();
    let cols = collection::collections(&v).unwrap();
    let dec: Vec<_> = cols.iter().map(|c| c.decode(&v, &schema).unwrap()).collect();
    let get = |d: &DecodedCollection, k: &[u8]| d.field(fnv1(k)).unwrap().data.clone();
    assert_eq!(get(&dec[0], b"name"), FieldData::Single(Value::Text(Some("xname".into()))));
    assert_eq!(get(&dec[0], b"link"), FieldData::Single(Value::Ref(RefSpec { class: CLASS, collection: child })));
    assert_eq!(get(&dec[0], b"speed"), FieldData::Single(Value::Float(1.5)));
    assert_eq!(get(&dec[0], b"dir"), FieldData::Single(Value::Vector3([0.0, 1.0, 0.0])));
    assert_eq!(get(&dec[0], b"refs"), FieldData::Array(vec![Value::Ref(RefSpec { class: CLASS, collection: x })]));
    assert_eq!(get(&dec[0], b"on"), FieldData::Single(Value::Bool(true)));
    assert_eq!(get(&dec[1], b"name"), FieldData::Single(Value::Text(None)));
    assert_eq!(dec.iter().map(|d| d.pointer_slots.len()).sum::<usize>(), v.fixup_count());

    let index = collection::index(&cols);
    let all = collection::with_inherited(&cols, &dec, &index, 1).unwrap();
    let speed = all.iter().find(|(_, f)| f.key == fnv1(b"speed")).unwrap();
    assert_eq!((speed.0, &speed.1.data), (child, &FieldData::Single(Value::Float(2.0))));
    assert!(all.iter().any(|(from, f)| *from == x && f.key == fnv1(b"dir")));
    assert!(!all.iter().any(|(from, f)| *from == x && f.key == fnv1(b"name")), "layout is not inherited");

    let mut bad = vlt.clone();
    let flags = (at + 0x20 + 16 + 10) as usize;
    bad[flags] = 0;
    let v = Vault::parse(&bad, &bin).unwrap();
    assert!(collection::collections(&v).unwrap()[0].decode(&v, &schema).is_err());
}
