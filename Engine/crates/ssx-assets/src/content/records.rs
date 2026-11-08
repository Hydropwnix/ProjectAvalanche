use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use ssx_content::Kind;
use ssx_formats::attrib::{Collection, Value, fnv1};

use super::Catalogue;
use super::vaults::{VaultIndex, archives, files_with_ext, rel, text};
use crate::career::CareerDb;

const CLASS_TEXTURE: u32 = fnv1(b"ge_texture");
const CLASS_GEOMETRY: u32 = fnv1(b"ge_geometry");
const CLASS_GAMEOBJECT: u32 = fnv1(b"ge_scene_gameobject");
const CLASS_MATERIAL: u32 = fnv1(b"ge_material");
const CLASS_CHARACTER: u32 = fnv1(b"character");
const CLASS_LIGHTING: u32 = fnv1(b"ge_globallightingparams");
pub(crate) const CLASS_POSTFX: u32 = fnv1(b"postfx");
const CLASS_TRACK: u32 = fnv1(b"ge_track");
const CLASS_DROP: u32 = fnv1(b"ge_drop");
const CLASS_UI_DROP: u32 = fnv1(b"ui_drop");
const CLASS_EQUIPMENT: u32 = fnv1(b"equipment");
const FIELD_DROP_LIGHTING: u32 = 0xC0EC_ED82;
const FIELD_DROP_POSTFX: u32 = 0x5595_A25C;
const FIELD_TRACK_NAME: u32 = 0x1A72_5E76;
const NAME_WINDOW: usize = 8;
const NAME_ROUNDS: usize = 8;
const GEAR_SLOTS: [&str; 16] = [
    "board",
    "wingsuit",
    "iceaxe",
    "armour",
    "thermal",
    "oxygen",
    "headlamp",
    "pulsegoggles",
    "boost1",
    "boost2",
    "boost3",
    "boost4",
    "jacket",
    "pants",
    "smokecanister",
    "none",
];
const KIND_BOARD: i64 = 1;
const KIND_GEOTAG: i64 = 4;

fn game_explorer_files(big: &Path) -> Result<BTreeMap<String, Vec<(String, String)>>> {
    let mut out: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for a in archives(big)? {
        let root = a.join("data/xenon/gameexplorer");
        for ext in ["xpr", "geom"] {
            for f in files_with_ext(&root, ext)? {
                let below = rel(&root, &f);
                out.entry(below.to_ascii_lowercase()).or_default().push((below, rel(big, &f)));
            }
        }
    }
    Ok(out)
}

fn record_source(idx: &VaultIndex, file: &str, c: &Collection, name: &str) -> String {
    let class = idx.class_name(c.class).unwrap_or("?");
    format!("{file}#{class}/{name}")
}

#[derive(Clone, Copy)]
enum Naming {
    Named,
    Keyed,
    Unspellable,
}

#[derive(Default)]
struct NamingCounts([usize; 3]);

impl NamingCounts {
    fn count(&mut self, n: Naming) {
        self.0[n as usize] += 1;
    }

    fn note(&self, what: &str) -> String {
        let [named, keyed, unspellable] = self.0;
        let mut s = format!("{named} {what} by their confirmed name, {keyed} by their key (no name confirmed)");
        if unspellable > 0 {
            s += &format!(", {unspellable} by their key (the confirmed name is outside the id alphabet)");
        }
        s
    }
}

fn record_name(idx: &VaultIndex, c: &Collection) -> (String, Naming) {
    let key = || format!("0x{:08x}", c.key);
    match idx.collection_name(c) {
        Some(n) => super::segment(n).map_or_else(|| (key(), Naming::Unspellable), |s| (s, Naming::Named)),
        None => (key(), Naming::Keyed),
    }
}

fn named_records(c: &mut Catalogue, idx: &VaultIndex, class: u32, kind: Kind) {
    let mut counts = NamingCounts::default();
    for (v, i) in idx.of_class(class) {
        let col = &v.cols[i];
        let (name, naming) = record_name(idx, col);
        counts.count(naming);
        c.add(kind, &name, record_source(idx, &v.file, col, &name));
    }
    c.note(kind, counts.note("records"));
}

fn path_records(
    c: &mut Catalogue,
    idx: &VaultIndex,
    files: &BTreeMap<String, Vec<(String, String)>>,
    class: u32,
    kind: Kind,
) -> Result<()> {
    let ext = if kind == Kind::Texture { "xpr" } else { "geom" };
    let (mut missing, mut bad, mut no_path) = (Vec::new(), 0usize, 0usize);
    let mut several = std::collections::BTreeSet::new();
    let mut ambiguous = std::collections::BTreeSet::new();
    for (v, i) in idx.of_class(class) {
        let Some(stored) = idx.fields(v, i)?.iter().find_map(|f| text(&f.data).map(str::to_string)) else {
            no_path += 1;
            continue;
        };
        let p = stored.replace('\\', "/").to_ascii_lowercase();
        let p = p.strip_suffix(&format!(".{ext}")).unwrap_or(&p).to_string();
        let Some(holders) = files.get(&format!("{p}.{ext}")) else {
            missing.push(stored);
            continue;
        };
        let spellings: std::collections::BTreeSet<&str> = holders.iter().map(|(b, _)| b.as_str()).collect();
        if spellings.len() > 1 {
            ambiguous.insert(p);
            continue;
        }
        if holders.len() > 1 {
            several.insert(p.clone());
        }
        match super::path_segments(&p) {
            Some(id) => c.add(kind, &id, holders[0].1.clone()),
            None => bad += 1,
        }
    }
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        let dlc = missing.iter().filter(|m| m.rsplit('\\').next().is_some_and(|n| n.starts_with("dlc"))).count();
        let shown = missing.iter().filter(|m| !m.contains("\\dlc")).take(6).cloned().collect::<Vec<_>>().join(", ");
        c.note(
            kind,
            format!(
                "{} stored paths name no file on the disc (left out): {dlc} of the `dlc*` riders, {} others, e.g. {shown}",
                missing.len(),
                missing.len() - dlc
            ),
        );
    }
    if !several.is_empty() {
        let n = several.len();
        c.note(kind, format!("{n} files are in several archives (the first archive in name order is listed)"));
    }
    if !ambiguous.is_empty() {
        let shown = ambiguous.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
        c.note(kind, format!("{} paths match files that differ only in case (left out): {shown}", ambiguous.len()));
    }
    if bad > 0 {
        c.note(kind, format!("{bad} paths have characters outside the id alphabet (left out)"));
    }
    if no_path > 0 {
        c.note(kind, format!("{no_path} records hold no path (left out)"));
    }
    Ok(())
}

fn unreferenced_models(c: &mut Catalogue, files: &BTreeMap<String, Vec<(String, String)>>) {
    let before = c.entries.keys().filter(|id| id.kind() == Kind::Model).count();
    let mut added = 0;
    for (lower, holders) in files.iter().filter(|(p, _)| p.ends_with(".geom")) {
        let stem = &lower[..lower.len() - ".geom".len()];
        let Some(path) = super::path_segments(stem) else { continue };
        let Ok(id) = ssx_content::ContentId::new(ssx_content::BASE_NAMESPACE, Kind::Model, &path) else { continue };
        let spellings: std::collections::BTreeSet<&str> = holders.iter().map(|(b, _)| b.as_str()).collect();
        if spellings.len() == 1 && !c.entries.contains_key(&id) {
            c.add(Kind::Model, &path, holders[0].1.clone());
            added += 1;
        }
    }
    c.note(Kind::Model, format!("{before} named by ge_geometry records; {added} more .geom files no record names"));
}

fn add_lighting_names(idx: &mut VaultIndex) -> Result<()> {
    let mut found = Vec::new();
    for (v, i) in idx.of_class(CLASS_DROP) {
        let fields = idx.fields(v, i)?;
        let lighting = fields.iter().find(|f| f.key == FIELD_DROP_LIGHTING).and_then(|f| match &f.data {
            ssx_formats::attrib::FieldData::Single(Value::Ref(r)) if r.class == CLASS_LIGHTING => Some(r.collection),
            _ => None,
        });
        let postfx = fields.iter().find(|f| f.key == FIELD_DROP_POSTFX).and_then(|f| text(&f.data));
        if let (Some(key), Some(base)) = (lighting, postfx.and_then(|p| p.strip_suffix("_postfx"))) {
            for cand in [format!("{base}_lighting"), format!("{base}_pre_lighting")] {
                if fnv1(cand.as_bytes()) == key {
                    found.push(cand);
                }
            }
        }
    }
    for n in found {
        idx.names.insert(&n);
    }
    Ok(())
}

pub fn add(c: &mut Catalogue, big: &Path, want: &dyn Fn(Kind) -> bool) -> Result<()> {
    let mut idx = VaultIndex::load(big)?;
    add_lighting_names(&mut idx)?;
    let rounds = idx.recover_names(NAME_WINDOW, NAME_ROUNDS);
    let total: usize = idx.vaults.iter().map(|v| v.cols.len()).sum();
    c.info(format!(
        "vault collection names confirmed by their export keys, of {total}: {rounds:?} (literal strings, + token \
         windows, + suffix rounds)"
    ));
    if want(Kind::Texture) || want(Kind::Model) {
        let files = game_explorer_files(big)?;
        if want(Kind::Texture) {
            path_records(c, &idx, &files, CLASS_TEXTURE, Kind::Texture)?;
        }
        if want(Kind::Model) {
            path_records(c, &idx, &files, CLASS_GEOMETRY, Kind::Model)?;
            unreferenced_models(c, &files);
        }
    }
    for (class, kind) in [
        (CLASS_GAMEOBJECT, Kind::Object),
        (CLASS_MATERIAL, Kind::Material),
        (CLASS_CHARACTER, Kind::Character),
        (CLASS_LIGHTING, Kind::Lighting),
        (CLASS_POSTFX, Kind::Postfx),
    ] {
        if want(kind) {
            named_records(c, &idx, class, kind);
        }
    }
    if want(Kind::Track) {
        tracks(c, &idx, big)?;
    }
    if want(Kind::Drop) || want(Kind::Event) {
        let mut counts = NamingCounts::default();
        for (v, i) in idx.of_class(CLASS_UI_DROP) {
            let col = &v.cols[i];
            let (name, naming) = record_name(&idx, col);
            counts.count(naming);
            for kind in [Kind::Drop, Kind::Event].into_iter().filter(|&k| want(k)) {
                c.add(kind, &name, record_source(&idx, &v.file, col, &name));
            }
        }
        for kind in [Kind::Drop, Kind::Event].into_iter().filter(|&k| want(k)) {
            c.note(kind, format!("one per ui_drop: {}", counts.note("ui_drops")));
        }
    }
    if want(Kind::Gear) || want(Kind::Board) {
        gear(c, &idx, big, want)?;
    }
    Ok(())
}

fn tracks(c: &mut Catalogue, idx: &VaultIndex, big: &Path) -> Result<()> {
    let mut no_folder = Vec::new();
    for (v, i) in idx.of_class(CLASS_TRACK) {
        let col = &v.cols[i];
        let fields = idx.fields(v, i)?;
        let stored = fields.iter().find(|f| f.key == FIELD_TRACK_NAME).and_then(|f| text(&f.data).map(str::to_string));
        let Some(name) = stored.as_deref().and_then(|s| s.strip_suffix("_track")) else {
            c.note(Kind::Track, format!("ge_track {:08x} has no `<name>_track` text (left out)", col.key));
            continue;
        };
        let mut holders = Vec::new();
        for a in archives(big)? {
            let folder = a.join("data/xenon/gameexplorer").join(name);
            if folder.is_dir() {
                holders.push(rel(big, &folder));
            }
        }
        match (holders.first(), super::segment(name)) {
            (Some(folder), Some(id)) => c.add(Kind::Track, &id, folder.clone()),
            (None, _) => no_folder.push(name.to_string()),
            (_, None) => c.note(Kind::Track, format!("{name:?} is not a valid id segment (left out)")),
        }
    }
    if !no_folder.is_empty() {
        c.note(
            Kind::Track,
            format!("{} ge_track records have no track folder (left out): {}", no_folder.len(), no_folder.join(", ")),
        );
    }
    Ok(())
}

fn gear(c: &mut Catalogue, idx: &VaultIndex, big: &Path, want: &dyn Fn(Kind) -> bool) -> Result<()> {
    let db = CareerDb::load_disc(big)?;
    let by_key: BTreeMap<u32, (&str, &Collection)> =
        idx.of_class(CLASS_EQUIPMENT).map(|(v, i)| (v.cols[i].key, (v.file.as_str(), &v.cols[i]))).collect();
    let (mut geotags, mut empty) = (0usize, Vec::new());
    let mut counts: BTreeMap<Kind, NamingCounts> = BTreeMap::new();
    for item in &db.items {
        let Some(&(file, col)) = by_key.get(&item.key) else {
            c.note(Kind::Gear, format!("item {:08x} has no equipment collection (left out)", item.key));
            continue;
        };
        if item.kind == KIND_GEOTAG {
            geotags += 1;
            continue;
        }
        let Some(slot) = usize::try_from(item.slot).ok().and_then(|s| GEAR_SLOTS.get(s)) else {
            empty.push(item.title.clone());
            continue;
        };
        let kind = if item.kind == KIND_BOARD { Kind::Board } else { Kind::Gear };
        if !want(kind) {
            continue;
        }
        let (name, naming) = record_name(idx, col);
        counts.entry(kind).or_default().count(naming);
        let path = if kind == Kind::Board { name.clone() } else { format!("{slot}/{name}") };
        c.add(kind, &path, record_source(idx, file, col, &name));
    }
    for (kind, n) in counts {
        c.note(kind, n.note("items"));
    }
    if want(Kind::Gear) {
        c.note(Kind::Gear, format!("{geotags} geotag items are not gear (left out)"));
    }
    c.note(
        if want(Kind::Gear) { Kind::Gear } else { Kind::Board },
        format!(
            "{} items without a GearItemType are the empty placeholders (left out): {}",
            empty.len(),
            empty.join(", ")
        ),
    );
    Ok(())
}

type RecordOf = (String, (u32, u32));

pub fn record_ids(big: &Path, classes: &[(u32, Kind)]) -> Result<BTreeMap<(u32, u32), ssx_content::ContentId>> {
    let mut idx = VaultIndex::load(big)?;
    add_lighting_names(&mut idx)?;
    idx.recover_names(NAME_WINDOW, NAME_ROUNDS);
    let mut by_id: BTreeMap<ssx_content::ContentId, Vec<RecordOf>> = BTreeMap::new();
    for &(class, kind) in classes {
        for (v, i) in idx.of_class(class) {
            let col = &v.cols[i];
            let (name, _) = record_name(&idx, col);
            if let Ok(id) = ssx_content::ContentId::new(ssx_content::BASE_NAMESPACE, kind, &name) {
                let record = record_source(&idx, &v.file, col, &name);
                let list = by_id.entry(id).or_default();
                if !list.iter().any(|(r, _)| *r == record) {
                    list.push((record, (class, col.key)));
                }
            }
        }
    }
    Ok(by_id.into_iter().filter(|(_, l)| l.len() == 1).map(|(id, l)| (l[0].1, id)).collect())
}

pub struct VaultNames {
    pub fields: BTreeMap<u32, String>,
    pub refs: BTreeMap<(u32, u32), String>,
}

pub type RecordIds = BTreeMap<(u32, u32), ssx_content::ContentId>;

pub fn ids_and_names(big: &Path, classes: &[(u32, Kind)], field_keys: &[u32]) -> Result<(RecordIds, VaultNames)> {
    let mut idx = VaultIndex::load(big)?;
    add_lighting_names(&mut idx)?;
    idx.recover_names(NAME_WINDOW, NAME_ROUNDS);
    let mut by_id: BTreeMap<ssx_content::ContentId, Vec<RecordOf>> = BTreeMap::new();
    for &(class, kind) in classes {
        for (v, i) in idx.of_class(class) {
            let col = &v.cols[i];
            let (name, _) = record_name(&idx, col);
            if let Ok(id) = ssx_content::ContentId::new(ssx_content::BASE_NAMESPACE, kind, &name) {
                let record = record_source(&idx, &v.file, col, &name);
                let list = by_id.entry(id).or_default();
                if !list.iter().any(|(r, _)| *r == record) {
                    list.push((record, (class, col.key)));
                }
            }
        }
    }
    let ids = by_id.into_iter().filter(|(_, l)| l.len() == 1).map(|(id, l)| (l[0].1, id)).collect();
    let fields = field_keys.iter().filter_map(|&k| idx.names.unique(k).map(|n| (k, n.to_string()))).collect();
    let mut refs = BTreeMap::new();
    for v in &idx.vaults {
        for c in &v.cols {
            let class = idx.class_name(c.class).map_or_else(|| format!("0x{:08x}", c.class), str::to_string);
            let name = idx.collection_name(c).map_or_else(|| format!("0x{:08x}", c.key), str::to_string);
            refs.entry((c.class, c.key)).or_insert_with(|| format!("{class}/{name}"));
        }
    }
    Ok((ids, VaultNames { fields, refs }))
}

pub const CLASS_KEYS: [(u32, Kind); 4] = [
    (CLASS_GAMEOBJECT, Kind::Object),
    (CLASS_MATERIAL, Kind::Material),
    (CLASS_LIGHTING, Kind::Lighting),
    (CLASS_POSTFX, Kind::Postfx),
];

pub fn rider_ids(big: &Path) -> Result<BTreeMap<ssx_content::ContentId, (Kind, u32)>> {
    let mut idx = VaultIndex::load(big)?;
    add_lighting_names(&mut idx)?;
    idx.recover_names(NAME_WINDOW, NAME_ROUNDS);
    let mut found: BTreeMap<ssx_content::ContentId, Vec<(Kind, u32)>> = BTreeMap::new();
    let mut put = |kind: Kind, path: &str, key: u32| {
        if let Ok(id) = ssx_content::ContentId::new(ssx_content::BASE_NAMESPACE, kind, path) {
            let list = found.entry(id).or_default();
            if !list.contains(&(kind, key)) {
                list.push((kind, key));
            }
        }
    };
    for (v, i) in idx.of_class(CLASS_CHARACTER) {
        let (name, _) = record_name(&idx, &v.cols[i]);
        put(Kind::Character, &name, v.cols[i].key);
    }
    let db = CareerDb::load_disc(big)?;
    let by_key: BTreeMap<u32, &Collection> =
        idx.of_class(CLASS_EQUIPMENT).map(|(v, i)| (v.cols[i].key, &v.cols[i])).collect();
    for item in db.items.iter().filter(|i| i.kind != KIND_GEOTAG) {
        let (Some(col), Some(slot)) =
            (by_key.get(&item.key), usize::try_from(item.slot).ok().and_then(|s| GEAR_SLOTS.get(s)))
        else {
            continue;
        };
        let (name, _) = record_name(&idx, col);
        if item.kind == KIND_BOARD {
            put(Kind::Board, &name, item.key);
        } else {
            put(Kind::Gear, &format!("{slot}/{name}"), item.key);
        }
    }
    Ok(found.into_iter().filter(|(_, l)| l.len() == 1).map(|(id, l)| (id, l[0])).collect())
}
