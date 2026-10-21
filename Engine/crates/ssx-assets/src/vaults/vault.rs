use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value as Json, json};
use ssx_formats::attrib::collection::{self, Collection, DecodedCollection, Field, FieldData, FieldSource};
use ssx_formats::attrib::container::PtrTarget;
use ssx_formats::attrib::{NameDict, Schema, Value, Vault, fnv1};

pub struct Options {
    pub vlt: PathBuf,
    pub schema: Option<PathBuf>,
    pub with: Vec<PathBuf>,
    pub strings: Vec<PathBuf>,
    pub collections: Vec<String>,
    pub class: Option<String>,
    pub depth: usize,
    pub json: Option<PathBuf>,
}

struct Files {
    path: PathBuf,
    vlt: Vec<u8>,
    bin: Vec<u8>,
}

impl Files {
    fn read(path: &Path) -> Result<Self> {
        let vlt = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let bin_path = path.with_extension("bin");
        let bin = std::fs::read(&bin_path).with_context(|| format!("reading {}", bin_path.display()))?;
        Ok(Files { path: path.to_path_buf(), vlt, bin })
    }
}

struct Loaded<'a> {
    name: String,
    vault: Vault<'a>,
    cols: Vec<Collection>,
    decoded: Vec<DecodedCollection>,
    local: HashMap<(u32, u32), usize>,
}

struct Db<'a> {
    schema: Schema,
    dict: NameDict,
    vaults: Vec<Loaded<'a>>,
    index: HashMap<(u32, u32), (usize, usize)>,
}

fn find_schema(vlt: &Path) -> Option<PathBuf> {
    let name = "attribdb_schema.vlt";
    let dir = vlt.parent()?;
    std::iter::once(dir.join(name))
        .chain(dir.ancestors().map(|a| a.join("bootcache/data/xenon/attribdb").join(name)))
        .find(|p| p.is_file())
}

fn parse_key(s: &str) -> u32 {
    let hex = s.strip_prefix("0x").unwrap_or(s);
    match u32::from_str_radix(hex, 16) {
        Ok(k) if hex.len() == 8 => k,
        _ => fnv1(s.as_bytes()),
    }
}

fn short_type(name: &str) -> &str {
    name.strip_prefix("Attrib::Gen::").unwrap_or_else(|| name.rsplit("::").next().unwrap_or(name))
}

impl Db<'_> {
    fn class_name(&self, key: u32) -> String {
        self.dict.show(key)
    }

    fn collection_name(&self, c: &Collection) -> String {
        match (self.dict.get(c.class), self.dict.get(c.key)) {
            (Some(cn), Some(nn)) if fnv1(format!("{cn}/{nn}").as_bytes()) == c.export_key => nn.to_owned(),
            _ => format!("{:08x}", c.key),
        }
    }

    fn lookup(&self, class: u32, key: u32) -> Option<(&Loaded<'_>, &Collection, &DecodedCollection)> {
        let &(v, i) = self.index.get(&(class, key))?;
        let l = &self.vaults[v];
        Some((l, &l.cols[i], &l.decoded[i]))
    }

    fn type_name(&self, hash: u32) -> String {
        self.schema.type_info(hash).map_or_else(|| format!("{hash:08x}"), |t| t.name.clone())
    }

    fn target_text(&self, vault: &Vault, t: PtrTarget) -> String {
        match vault.cstr(t) {
            Ok(s) if s.len() < 200 && s.bytes().all(|b| (0x20..0x7f).contains(&b)) => format!("{s:?}"),
            _ => format!("{}+{:#x}", if t.file == 0 { "vlt" } else { "bin" }, t.offset),
        }
    }

    fn show_value(&self, vault: &Vault, v: &Value) -> String {
        match v {
            Value::Bool(b) => b.to_string(),
            Value::Int8(x) => x.to_string(),
            Value::UInt8(x) => x.to_string(),
            Value::Int16(x) => x.to_string(),
            Value::UInt16(x) => x.to_string(),
            Value::Int32(x) => x.to_string(),
            Value::UInt32(x) => x.to_string(),
            Value::Int64(x) => x.to_string(),
            Value::UInt64(x) => x.to_string(),
            Value::Float(x) => x.to_string(),
            Value::Vector2(a) => format!("{a:?}"),
            Value::Vector3(a) => format!("{a:?}"),
            Value::Vector4(a) => format!("{a:?}"),
            Value::Text(Some(s)) => format!("{s:?}"),
            Value::Text(None) => "null".into(),
            Value::Key(k) => self.dict.show(*k),
            Value::Ref(r) if r.collection == 0 => format!("{}/<none>", self.class_name(r.class)),
            Value::Ref(r) => match self.lookup(r.class, r.collection) {
                Some((l, c, _)) => format!("{}/{}  [{}]", self.class_name(r.class), self.collection_name(c), l.name),
                None => format!("{}/{}  [not loaded]", self.class_name(r.class), self.dict.show(r.collection)),
            },
            Value::Raw { bytes, pointers } => {
                let words: Vec<String> =
                    bytes.chunks(4).map(|c| c.iter().map(|b| format!("{b:02x}")).collect()).collect();
                let mut s = format!("raw {}", words.join(" "));
                for p in pointers {
                    s += &format!("; +{} -> {}", p.offset, self.target_text(vault, p.target));
                }
                s
            }
        }
    }

    fn value_json(&self, vault: &Vault, v: &Value) -> Json {
        match v {
            Value::Bool(b) => json!(b),
            Value::Int8(x) => json!(x),
            Value::UInt8(x) => json!(x),
            Value::Int16(x) => json!(x),
            Value::UInt16(x) => json!(x),
            Value::Int32(x) => json!(x),
            Value::UInt32(x) => json!(x),
            Value::Int64(x) => json!(x),
            Value::UInt64(x) => json!(x),
            Value::Float(x) => json!(x),
            Value::Vector2(a) => json!(a),
            Value::Vector3(a) => json!(a),
            Value::Vector4(a) => json!(a),
            Value::Text(s) => json!(s),
            Value::Key(k) => json!({ "key": format!("{k:08x}"), "name": self.dict.get(*k) }),
            Value::Ref(r) => {
                let found = self.lookup(r.class, r.collection);
                json!({
                    "class": format!("{:08x}", r.class),
                    "class_name": self.dict.get(r.class),
                    "collection": format!("{:08x}", r.collection),
                    "collection_name": found.map(|(_, c, _)| self.collection_name(c)),
                    "vault": found.map(|(l, _, _)| l.name.clone()),
                })
            }
            Value::Raw { bytes, pointers } => json!({
                "raw": bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                "pointers": pointers.iter().map(|p| json!({
                    "offset": p.offset,
                    "target": self.target_text(vault, p.target),
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

fn source_tag(s: FieldSource) -> &'static str {
    match s {
        FieldSource::Layout => "L",
        FieldSource::Inline => "I",
        FieldSource::OutOfLine => "P",
    }
}

fn values(f: &Field) -> Vec<&Value> {
    match &f.data {
        FieldData::Single(v) => vec![v],
        FieldData::Array(a) => a.iter().collect(),
    }
}

impl Db<'_> {
    fn print_class(&self, key: u32) {
        let Some(c) = self.schema.class(key) else {
            println!("class {} [{key:08x}]: not in the schema", self.class_name(key));
            return;
        };
        println!(
            "class {} [{key:08x}]: {} definitions, layout {} bytes, static {} bytes, unk_collection_capacity {}, unk_layout {}/{}",
            self.class_name(key),
            c.definitions.len(),
            c.layout_size,
            c.static_size,
            c.unk_collection_capacity,
            c.unk_layout_a,
            c.unk_layout_b
        );
        let mut defs: Vec<_> = c.definitions.iter().collect();
        defs.sort_by_key(|d| (!d.in_layout(), d.is_static(), d.offset));
        for d in defs {
            let kind = if d.in_layout() {
                "L"
            } else if d.is_static() {
                "S"
            } else {
                "T"
            };
            println!(
                "  {kind} +{:<4} {:<34} {:<28} size {:>3} max {:>5} flags {:#04x} align 2^{}",
                d.offset,
                self.dict.show(d.key),
                short_type(&self.type_name(d.type_hash)),
                d.size,
                d.max_count,
                d.flags,
                d.alignment_log2
            );
        }
        for (k, v) in &c.static_values {
            println!("  static {} = {v:?}", self.dict.show(*k));
        }
    }

    fn print(&self, v: usize, i: usize, indent: usize, depth: usize, seen: &mut HashSet<(u32, u32)>) -> Result<()> {
        let l = &self.vaults[v];
        let c = &l.cols[i];
        let pad = " ".repeat(indent);
        let title =
            format!("{} [{:08x}] class {} [{:08x}]", self.collection_name(c), c.key, self.class_name(c.class), c.class);
        if !seen.insert((c.class, c.key)) {
            println!("{pad}collection {title} (see above)");
            return Ok(());
        }
        let parent = if c.parent == 0 { "-".to_owned() } else { self.dict.show(c.parent) };
        println!("{pad}collection {title} parent {parent}  ({})", l.name);
        let mut refs = Vec::new();
        for (holder, f) in collection::with_inherited(&l.cols, &l.decoded, &l.local, i)? {
            let from = if holder == c.key { String::new() } else { format!("  (from {})", self.dict.show(holder)) };
            let head = format!(
                "{pad}  {} {:<34} {:<24}",
                source_tag(f.source),
                self.dict.show(f.key),
                short_type(&self.type_name(f.type_hash))
            );
            let shown: Vec<String> = values(f).iter().map(|x| self.show_value(&l.vault, x)).collect();
            match &f.data {
                FieldData::Single(_) => println!("{head} = {}{from}", shown[0]),
                FieldData::Array(_) if shown.join(", ").len() <= 100 => {
                    println!("{head} = [{}]{from}", shown.join(", "))
                }
                FieldData::Array(_) => {
                    println!("{head} = [{} elements]{from}", shown.len());
                    for (k, s) in shown.iter().enumerate() {
                        println!("{pad}      [{k}] {s}");
                    }
                }
            }
            for x in values(f) {
                if let Value::Ref(r) = x {
                    refs.push((r.class, r.collection));
                }
            }
        }
        if depth > 0 {
            for key in refs {
                if let Some(&(rv, ri)) = self.index.get(&key) {
                    self.print(rv, ri, indent + 4, depth - 1, seen)?;
                }
            }
        }
        Ok(())
    }

    fn collection_json(&self, v: usize, i: usize) -> Result<Json> {
        let l = &self.vaults[v];
        let c = &l.cols[i];
        let mut fields = Vec::new();
        for (holder, f) in collection::with_inherited(&l.cols, &l.decoded, &l.local, i)? {
            let vals: Vec<Json> = values(f).iter().map(|x| self.value_json(&l.vault, x)).collect();
            fields.push(json!({
                "key": format!("{:08x}", f.key),
                "name": self.dict.get(f.key),
                "type": self.type_name(f.type_hash),
                "storage": source_tag(f.source),
                "inherited_from": (holder != c.key).then(|| format!("{holder:08x}")),
                "value": match f.data { FieldData::Single(_) => vals[0].clone(), FieldData::Array(_) => Json::Array(vals) },
            }));
        }
        Ok(json!({
            "key": format!("{:08x}", c.key),
            "name": self.collection_name(c),
            "class": format!("{:08x}", c.class),
            "class_name": self.dict.get(c.class),
            "parent": (c.parent != 0).then(|| format!("{:08x}", c.parent)),
            "vault": l.name,
            "fields": fields,
        }))
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

pub fn run(o: &Options) -> Result<()> {
    let schema_path = match &o.schema {
        Some(p) => p.clone(),
        None => find_schema(&o.vlt).context("attribdb_schema.vlt not found next to the vault; pass --schema")?,
    };
    let mut paths = vec![o.vlt.clone()];
    paths.extend(o.with.iter().cloned());
    let schema_index = match paths.iter().position(|p| same_file(p, &schema_path)) {
        Some(i) => i,
        None => {
            paths.push(schema_path);
            paths.len() - 1
        }
    };
    let files: Vec<Files> = paths.iter().map(|p| Files::read(p)).collect::<Result<_>>()?;
    let parsed: Vec<Vault> = files
        .iter()
        .map(|f| Vault::parse(&f.vlt, &f.bin).with_context(|| format!("parsing {}", f.path.display())))
        .collect::<Result<_>>()?;
    let schema = Schema::from_vault(&parsed[schema_index]).context("reading the schema")?;
    let mut dict = NameDict::new();
    for f in &files {
        dict.add_strings_from(&f.vlt);
        dict.add_strings_from(&f.bin);
    }
    for s in &o.strings {
        dict.add_strings_from(&std::fs::read(s).with_context(|| format!("reading {}", s.display()))?);
    }
    for t in &schema.types {
        dict.insert(&t.name);
    }
    let mut vaults = Vec::new();
    for (f, vault) in files.iter().zip(parsed) {
        let cols = collection::collections(&vault)?;
        let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        let local = collection::index(&cols);
        let name = f.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        vaults.push(Loaded { name, vault, cols, decoded, local });
    }
    let mut index = HashMap::new();
    for (vi, l) in vaults.iter().enumerate() {
        for (ci, c) in l.cols.iter().enumerate() {
            index.entry((c.class, c.key)).or_insert((vi, ci));
        }
    }
    let db = Db { schema, dict, vaults, index };
    let p = &db.vaults[0];
    println!(
        "{}: {} exports, {} collections, {} pointer fixups, {} StrE strings; schema: {} classes, {} types; {} names in the dictionary",
        p.name,
        p.vault.exports.len(),
        p.cols.len(),
        p.vault.fixup_count(),
        p.vault.bin_strings.len(),
        db.schema.classes.len(),
        db.schema.types.len(),
        db.dict.len()
    );
    let class = o.class.as_deref().map(parse_key);
    if let Some(k) = class {
        db.print_class(k);
    } else if p.cols.is_empty() {
        for c in &db.schema.classes {
            db.print_class(c.key);
        }
    }
    let wanted: Vec<u32> = o.collections.iter().map(|s| parse_key(s)).collect();
    let selected: Vec<usize> = (0..p.cols.len())
        .filter(|&i| class.is_none_or(|k| p.cols[i].class == k))
        .filter(|&i| wanted.is_empty() || wanted.contains(&p.cols[i].key))
        .collect();
    if selected.is_empty() && !wanted.is_empty() {
        bail!("no matching collection in {}", p.name);
    }
    let mut seen = HashSet::new();
    for &i in &selected {
        db.print(0, i, 0, o.depth, &mut seen)?;
    }
    if let Some(out) = &o.json {
        let all: Vec<Json> = selected.iter().map(|&i| db.collection_json(0, i)).collect::<Result<_>>()?;
        let file = std::fs::File::create(out).with_context(|| format!("creating {}", out.display()))?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(file), &all)?;
        println!("wrote {} collections to {}", all.len(), out.display());
    }
    Ok(())
}
