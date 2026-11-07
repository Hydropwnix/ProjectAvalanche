use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::{Checked, Severity};
use crate::audio::music::MusicDef;
use crate::audio::sounds::{SoundClass, SoundDef};
use crate::audio::{AudioInfo, Decoder, Limits};
use crate::id::{ContentId, Kind};
use crate::manifest::MANIFEST_FILE;
use crate::path::PackageTree;

#[derive(Default)]
pub struct AudioFiles {
    pub sounds: Vec<(ContentId, String, Arc<SoundDef>)>,
    pub music: Vec<(ContentId, String, Arc<MusicDef>)>,
    pub files: Vec<(String, Kind, AudioInfo)>,
    decoded: BTreeSet<(String, Kind)>,
    sound_records: BTreeMap<(String, u8), Result<Arc<SoundDef>, Vec<crate::material::Problem>>>,
    music_records: BTreeMap<(String, u8), Result<Arc<MusicDef>, Vec<crate::material::Problem>>>,
}

pub(super) fn check_entry(
    c: &mut Checked,
    tree: &PackageTree,
    id: &ContentId,
    f: &str,
    at: (usize, usize),
    own: &str,
    named: &mut BTreeSet<String>,
) {
    let kind = id.kind();
    let toml = f.to_ascii_lowercase().ends_with(".toml");
    let read = || tree.read(f, crate::limits::MAX_RECORD_BYTES).map_err(|e| vec![(0, 0, e.to_string())]);
    let files: Vec<String> = if kind == Kind::Sound {
        let class = SoundClass::of(id);
        match class {
            SoundClass::Trigger => c.push(
                Severity::Error,
                MANIFEST_FILE,
                at,
                format!(
                    "{id}: a trigger of the speech's context module; what it says is its contexts' choice: replace \
                     the speech events they fire (ids under …/pilot_and_rider_eventdef_<language>/)"
                ),
            ),
            SoundClass::MusicEvent => c.push(
                Severity::Error,
                MANIFEST_FILE,
                at,
                format!("{id}: the licensed music's selection event: songs are added or replaced in a music.toml"),
            ),
            SoundClass::Added => c.push(
                Severity::Note,
                MANIFEST_FILE,
                at,
                format!(
                    "{id}: a sound of the package's own: registered and checked; a character's [voice_lines] play it \
                     (M3), scripts and markers from phase M7"
                ),
            ),
            _ => {}
        }
        let def = if toml {
            let parsed = c
                .audio
                .sound_records
                .entry((f.to_string(), class as u8))
                .or_insert_with(|| crate::audio::sounds::parse_toml(&read()?, class).map(Arc::new));
            match parsed.clone() {
                Ok(d) => d,
                Err(ps) => return problems(c, f, ps),
            }
        } else {
            Arc::new(SoundDef::file(f))
        };
        let files = def.files.clone();
        c.audio.sounds.push((id.clone(), f.to_string(), def));
        files
    } else {
        let over = (id.namespace() != own).then_some(id);
        let class: u8 = match over {
            None => 0,
            Some(o) if o.path().starts_with("licensed/") => 1,
            Some(o) if o.path().starts_with("pathfinder/") => 2,
            Some(_) => 3,
        };
        let def = if toml {
            let parsed = c
                .audio
                .music_records
                .entry((f.to_string(), class))
                .or_insert_with(|| crate::audio::music::parse_toml(&read()?, over).map(Arc::new));
            match parsed.clone() {
                Ok(d) => d,
                Err(ps) => return problems(c, f, ps),
            }
        } else if over.is_some_and(|o| o.path().starts_with("pathfinder/")) {
            let audio = crate::audio::music::TrackAudio::Looped { file: f.to_string(), region: None };
            Arc::new(MusicDef::Track { audio, gain: 1.0 })
        } else {
            Arc::new(MusicDef::song_file(f))
        };
        let files = def.files().into_iter().map(str::to_string).collect();
        c.audio.music.push((id.clone(), f.to_string(), def));
        files
    };
    let limits = if kind == Kind::Music { Limits::music() } else { Limits::sound() };
    for file in files {
        named.insert(file.clone());
        if !c.audio.decoded.insert((file.clone(), kind)) {
            continue;
        }
        decode_check(c, tree, &file, kind, &limits);
    }
}

fn problems(c: &mut Checked, f: &str, ps: Vec<crate::material::Problem>) {
    for (l, col, m) in ps {
        c.push(Severity::Error, f, (l, col), m);
    }
}

fn decode_check(c: &mut Checked, tree: &PackageTree, file: &str, kind: Kind, limits: &Limits) {
    if tree.kind(file).is_none() {
        c.push(
            Severity::Error,
            file,
            (0, 0),
            "named by its record, but the package does not have it (names are case-sensitive)",
        );
        return;
    }
    let bytes: Arc<[u8]> = match tree.read(file, limits.max_file_bytes) {
        Ok(b) => b.into(),
        Err(e) => return c.push(Severity::Error, file, (0, 0), e.to_string()),
    };
    let mut d = match Decoder::open(file, bytes, limits) {
        Ok(d) => d,
        Err(e) => return c.push(Severity::Error, file, (0, 0), e.message),
    };
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match d.next(&mut buf) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return c.push(Severity::Error, file, (0, 0), e.message),
        }
    }
    let i = *d.info();
    c.push(
        Severity::Note,
        file,
        (0, 0),
        format!(
            "{}: {} channel{} at {} Hz, {:.2} s; {} bytes decoded at {} Hz",
            i.format.name(),
            i.channels,
            if i.channels == 1 { "" } else { "s" },
            i.rate,
            i.seconds(),
            i.decoded_bytes(limits.target_rate),
            limits.target_rate
        ),
    );
    c.audio.files.push((file.to_string(), kind, i));
}
