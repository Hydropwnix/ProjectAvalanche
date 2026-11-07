use std::collections::BTreeSet;
use std::path::Path;

use ssx_content::check::{Checked, RiderEntryFile, Severity};
use ssx_content::manifest::{MANIFEST_FILE, Manifest};
use ssx_content::material::Family;
use ssx_content::rider::{OutfitSlot, gear_type};
use ssx_content::{BASE_NAMESPACE, ContentId, Kind, Layer, PackageSource};

use super::check::push;
use crate::content::GameContent;
use crate::content::characters::look::{self, LookKind, Skeletons};
use crate::content::characters::{LookSource, RIDER_KINDS};
use crate::content::resolve::MaterialUse;

struct At<'a> {
    c: &'a mut Checked,
    at: (usize, usize),
    id: &'a ContentId,
}

impl At<'_> {
    fn error(&mut self, file: &str, message: String) {
        let at = if file == MANIFEST_FILE { self.at } else { (0, 0) };
        push(self.c, Severity::Error, file, at, format!("{}: {message}", self.id));
    }

    fn note(&mut self, file: &str, message: String) {
        let at = if file == MANIFEST_FILE { self.at } else { (0, 0) };
        push(self.c, Severity::Note, file, at, format!("{}: {message}", self.id));
    }
}

pub(super) fn check(c: &mut Checked, m: &Manifest, dir: &Path, big: &Path, app_dir: &Path) {
    let patched: Vec<ContentId> = m.patches.keys().filter(|id| RIDER_KINDS.contains(&id.kind())).cloned().collect();
    if c.riders.entries.is_empty() && patched.is_empty() {
        return;
    }
    if c.errors() > 0 {
        let message =
            "the characters, gear and boards are checked on the riders' skeleton once the package has no errors";
        return push(c, Severity::Note, MANIFEST_FILE, (0, 0), message.to_string());
    }
    let source = match PackageSource::open(dir) {
        Ok(s) => s,
        Err(e) => return push(c, Severity::Error, MANIFEST_FILE, (0, 0), e.to_string()),
    };
    let content = GameContent::of(big);
    content.mount_client_mods_except(app_dir, Some(&m.resource.name));
    let all: BTreeSet<ContentId> = m.add.keys().chain(m.overrides.keys()).cloned().collect();
    content.swap_dev_package(None, source, all);
    let Some(disc) = content.disc_riders() else {
        let message = "the disc's riders and items cannot be indexed: characters, gear and boards are not checked";
        return push(c, Severity::Error, MANIFEST_FILE, (0, 0), message.to_string());
    };
    let mut db = match crate::career::CareerDb::load_disc(big) {
        Ok(db) => db,
        Err(e) => {
            let message = format!("the disc's career records cannot be read ({e:#}): characters are not checked");
            return push(c, Severity::Error, MANIFEST_FILE, (0, 0), message);
        }
    };
    let refused = crate::content::characters::career::apply(big, &mut db);
    let disc_rider = |id: &ContentId| id.kind() == Kind::Character && disc.record(id).is_some();
    let entries = c.riders.entries.clone();
    for e in &entries {
        let position = m.positions.entries.get(&e.id).copied().unwrap_or((0, 0));
        let mut at = At { c, at: position, id: &e.id };
        for (_, why) in refused.iter().filter(|(id, _)| *id == e.id) {
            let what = if e.added { "is not added" } else { "its record is not used (the disc's is)" };
            at.error(MANIFEST_FILE, format!("{what}: {why}"));
        }
        match e.id.kind() {
            Kind::Character => character(&mut at, &content, m, e, &disc_rider),
            _ => item(&mut at, &content, m, e, &db, &disc_rider),
        }
    }
    for id in &patched {
        let position = m.positions.patches.get(id).copied().unwrap_or((0, 0));
        let mut at = At { c, at: position, id };
        let resolved = match id.kind() {
            Kind::Character => content.character_def(id).map(|_| ()),
            _ => content.item_def(id).map(|_| ()),
        };
        if let Err(e) = resolved {
            at.error(MANIFEST_FILE, format!("the record with this patch merged: {e}"));
        }
        for (_, why) in refused.iter().filter(|(r, _)| r == id) {
            at.error(MANIFEST_FILE, format!("with this patch: {why}"));
        }
    }
    budget(c, &content, m, &entries);
}

fn own(m: &Manifest) -> LookSource {
    LookSource { source: m.resource.name.clone(), layer: Layer::ClientMods }
}

fn mounted(content: &GameContent, id: &ContentId) -> bool {
    content.has(id)
}

fn materials(
    at: &mut At<'_>,
    content: &GameContent,
    file: &str,
    bound: &std::collections::BTreeMap<String, ContentId>,
) {
    for (name, id) in bound {
        match content.material(id, MaterialUse::Riders) {
            Ok(f) if f.value.family == Some(Family::Character) => {}
            Ok(_) => at.error(file, format!("[materials] {name:?} = {id}: not of the character family")),
            Err(e) => at.error(file, format!("[materials] {name:?} = {id}: {e}")),
        }
    }
}

fn bind(at: &mut At<'_>, content: &GameContent, m: &Manifest, e: &RiderEntryFile, kind: LookKind, face: Option<&str>) {
    let Some((file, model)) = &e.model else { return };
    let materials = match (&e.character, &e.item) {
        (Some((_, d)), _) => d.materials.clone(),
        (_, Some((_, d))) => d.materials.clone(),
        _ => match e.id.kind() {
            Kind::Character => content.character_def(&e.id).map(|d| d.value.materials).unwrap_or_default(),
            _ => content.item_def(&e.id).map(|d| d.value.materials).unwrap_or_default(),
        },
    };
    let big = content.big_root().to_path_buf();
    let plain = match content.riders.skeletons.get(&big, None) {
        Ok(s) => s,
        Err(e) => return at.error(file, format!("the riders' skeleton: {e:#}")),
    };
    let face_sk = match face.map(|f| content.riders.skeletons.get(&big, Some(f))) {
        Some(Ok(s)) => Some(s),
        Some(Err(e)) => return at.error(file, format!("the face set {}: {e:#}", face.unwrap_or(""))),
        None => None,
    };
    let skeletons = Skeletons { face: face_sk.as_ref().map(|s| &s.skeleton), plain: &plain.skeleton };
    match look::build(content, &e.id, &own(m), model.clone(), kind, &materials, &skeletons) {
        Ok(l) => {
            let palette = l.parts.iter().map(|p| p.palette.len()).max().unwrap_or(0);
            let on_face = l.parts.iter().filter(|p| p.face_offsets).count();
            let face = face.map_or(String::new(), |f| format!(", {on_face} on {f}'s face offsets"));
            let message = format!(
                "binds to the riders' skeleton: {} parts in {} LODs, at most {palette} joints a part{face}",
                l.parts.len(),
                l.lods()
            );
            at.note(file, message);
            for n in l.notes {
                at.note(file, n);
            }
        }
        Err(problems) => {
            for p in problems {
                at.error(file, p);
            }
        }
    }
}

fn character(
    at: &mut At<'_>,
    content: &GameContent,
    m: &Manifest,
    e: &RiderEntryFile,
    disc_rider: &dyn Fn(&ContentId) -> bool,
) {
    let file = e.character.as_ref().map_or(MANIFEST_FILE, |(f, _)| f.as_str()).to_string();
    let def = match content.character_def(&e.id) {
        Ok(d) => d.value,
        Err(err) => return at.error(MANIFEST_FILE, format!("its record: {err}")),
    };
    let override_of_disc = e.id.namespace() == BASE_NAMESPACE;
    if override_of_disc && def.stands_in_for.as_ref().is_some_and(|s| *s != e.id) {
        at.error(&file, "stands_in_for: an override of a disc rider takes its own place (leave it out)".into());
    }
    for (key, v) in [("stands_in_for", &def.stands_in_for), ("face", &def.face), ("voice", &def.voice)] {
        if let Some(v) = v
            && !disc_rider(v)
        {
            at.error(&file, format!("{key} = {v}: not one of the disc's riders"));
        }
    }
    let speech = if def.voice_lines.is_empty() { None } else { content.disc_sounds() };
    for (event, sound) in &def.voice_lines {
        if speech.as_ref().is_none_or(|s| s.get(event).is_none()) {
            at.error(&file, format!("[voice_lines] {event}: not a speech event of the base game"));
        }
        if !mounted(content, sound) {
            at.error(&file, format!("[voice_lines] {event} = {sound}: no such sound is mounted"));
        }
    }
    if let Some(b) = &def.default_board
        && let Err(err) = content.item_def(b)
    {
        at.error(&file, format!("default_board = {b}: {err}"));
    }
    for (slot, o) in &def.outfit {
        let OutfitSlot::Gear(g) = o else { continue };
        match content.item_def(g) {
            Ok(d) => {
                let fills = gear_type(d.value.slot).map(|(_, _, s)| s).unwrap_or(&[]);
                if !fills.contains(&slot.as_str()) {
                    at.error(&file, format!("[outfit] {slot} = {g}: it fills {}", fills.join(", ")));
                }
                if content.look_sources(g).is_empty() {
                    at.error(&file, format!("[outfit] {slot} = {g}: it has no model"));
                }
            }
            Err(err) => at.error(&file, format!("[outfit] {slot} = {g}: {err}")),
        }
    }
    if let Some(c) = &def.career {
        for (key, t) in [("portrait", &c.portrait), ("round_icon", &c.round_icon), ("icon", &c.icon)] {
            if let Some(t) = t
                && !mounted(content, t)
            {
                at.error(&file, format!("[career] {key} = {t}: no such texture is mounted"));
            }
        }
        if e.added && c.portrait.is_none() {
            at.note(&file, "[career] has no portrait: the rider select shows the stand-in's place without one".into());
        }
    }
    materials(at, content, &file, &def.materials);
    let face = look::face_set(&e.id, &def);
    bind(at, content, m, e, LookKind::Character, face.as_deref());
}

fn item(
    at: &mut At<'_>,
    content: &GameContent,
    m: &Manifest,
    e: &RiderEntryFile,
    db: &crate::career::CareerDb,
    disc_rider: &dyn Fn(&ContentId) -> bool,
) {
    let file = e.item.as_ref().map_or(MANIFEST_FILE, |(f, _)| f.as_str()).to_string();
    let def = match content.item_def(&e.id) {
        Ok(d) => d.value,
        Err(err) => return at.error(MANIFEST_FILE, format!("its record: {err}")),
    };
    let wearer = def.rider.as_ref().filter(|r| disc_rider(r)).map(|r| r.path().to_string());
    if let Some(r) = &def.rider {
        let key = if r.namespace() == BASE_NAMESPACE {
            ssx_formats::attrib::fnv1(r.path().as_bytes())
        } else {
            crate::content::characters::career::content_key(r)
        };
        if !db.riders.iter().any(|x| x.key == key) {
            at.error(&file, format!("rider = {r}: not a rider (the disc's or one added)"));
        }
    }
    if let Some(t) = &def.icon
        && !mounted(content, t)
    {
        at.error(&file, format!("icon = {t}: no such texture is mounted"));
    }
    materials(at, content, &file, &def.materials);
    let kind = match gear_type(def.slot) {
        Some(("board", _, _)) => LookKind::Board,
        Some((_, _, slots)) => LookKind::Gear(slots),
        None => return at.error(&file, format!("slot {:?}: not a GearItemType", def.slot)),
    };
    if wearer.is_none() && kind != LookKind::Board {
        at.note(
            &file,
            "checked on the plain rig (no disc rider named): a rider's face offsets are checked when worn".into(),
        );
    }
    bind(at, content, m, e, kind, wearer.as_deref());
}

fn budget(c: &mut Checked, content: &GameContent, m: &Manifest, entries: &[RiderEntryFile]) {
    let with_model = |k: Kind| -> Vec<ContentId> {
        entries.iter().filter(|e| e.id.kind() == k && e.model.is_some()).map(|e| e.id.clone()).collect()
    };
    let characters = with_model(Kind::Character);
    let items: Vec<ContentId> = with_model(Kind::Gear).into_iter().chain(with_model(Kind::Board)).collect();
    if characters.is_empty() && items.is_empty() {
        return;
    }
    let d = content.rider_looks(None, &characters, &items);
    for o in d.outcomes.iter().filter(|o| o.source == m.resource.name) {
        let at = m.positions.entries.get(&o.id).copied().unwrap_or((0, 0));
        let u = &o.cost;
        match &o.refused {
            Some(why) => {
                let message = format!("{}: over a level's budget on its own, the disc's look is drawn: {why}", o.id);
                push(c, Severity::Warning, MANIFEST_FILE, at, message);
            }
            None => {
                let message = format!(
                    "{}: takes {} vertices, {} triangles, {} joints, {} bytes held, {} bytes of images of a level's budget",
                    o.id, u.vertices, u.triangles, u.joints, u.held_bytes, u.image_bytes
                );
                push(c, Severity::Note, MANIFEST_FILE, at, message);
            }
        }
    }
}
