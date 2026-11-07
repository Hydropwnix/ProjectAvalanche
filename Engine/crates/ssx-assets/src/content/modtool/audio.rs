use std::collections::BTreeSet;
use std::path::Path;

use ssx_content::audio::music::{MusicDef, TrackAudio};
use ssx_content::audio::sounds::{Route, SoundClass};
use ssx_content::check::{Checked, Severity, locate_key};
use ssx_content::manifest::{MANIFEST_FILE, Manifest};
use ssx_content::{BASE_NAMESPACE, ContentId, Kind};

use super::check::push;
use crate::content::audio::{BUDGET, DiscSounds};

pub(super) fn named_ids(c: &Checked) -> Vec<ContentId> {
    let mut out = Vec::new();
    for (_, _, def) in &c.audio.music {
        let songs: Vec<&ssx_content::audio::music::Song> = match &**def {
            MusicDef::Playlist { songs, .. } => songs.iter().collect(),
            MusicDef::Song(s) => vec![s],
            _ => Vec::new(),
        };
        out.extend(songs.into_iter().flat_map(|s| s.drops.iter().flatten().cloned()));
    }
    out
}

fn locate_quoted(text: &str, value: &str, key: &str) -> (usize, usize) {
    let quoted = format!("{value:?}");
    text.find(&quoted).map_or_else(|| locate_key(text, key), |at| ssx_content::manifest::line_col(text, at))
}

fn locate_value(text: &str, key: &str, value: &str) -> (usize, usize) {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let v = t.strip_prefix(key).and_then(|r| r.trim_start().strip_prefix('='));
        if v.is_some_and(|v| v.split('#').next().unwrap_or("").trim() == value) {
            return ssx_content::manifest::line_col(text, at + line.len() - t.len());
        }
        at += line.len();
    }
    locate_key(text, key)
}

fn text(c: &Checked, f: &str) -> String {
    c.tree
        .as_ref()
        .and_then(|t| t.read(f, ssx_content::limits::MAX_RECORD_BYTES).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default()
}

pub(super) fn check(c: &mut Checked, m: &Manifest, big: &Path, base: Option<&BTreeSet<ContentId>>) {
    if c.audio.sounds.is_empty() && c.audio.music.is_empty() {
        return;
    }
    check_sounds(c, m, big);
    check_music(c, big, base);
    totals(c);
}

fn check_sounds(c: &mut Checked, m: &Manifest, big: &Path) {
    if c.audio.sounds.is_empty() {
        return;
    }
    let voices = crate::sfx_mixer::voice_patch_table(big);
    let buses = crate::sfx_bus::PatchSet::load(big).map(|s| s.bus_names());
    let disc = DiscSounds::index(big);
    for (id, file, def) in c.audio.sounds.clone() {
        let at_key = |c: &Checked, key: &str| locate_key(&text(c, &file), key);
        if SoundClass::of(&id) == SoundClass::Effect && id.namespace() == BASE_NAMESPACE {
            let entry = disc.as_ref().ok().and_then(|d| d.get(&id).cloned());
            if let Some(names) = &def.voices {
                match &voices {
                    Ok(t) => {
                        for n in names {
                            let known = t.values().any(|(p, v)| p.eq_ignore_ascii_case(n) || v.eq_ignore_ascii_case(n));
                            if !known {
                                let at = locate_quoted(&text(c, &file), n, "voices");
                                push(
                                    c,
                                    Severity::Error,
                                    &file,
                                    at,
                                    format!("voices: {n:?} is no mixer voice of the game's (`audio_mixer_voices`)"),
                                );
                            }
                        }
                    }
                    Err(e) => push(
                        c,
                        Severity::Error,
                        &file,
                        (0, 0),
                        format!("the game's mixer voices cannot be read ({e:#})"),
                    ),
                }
            }
            if let (Some(players), Some(e)) = (&def.players, &entry) {
                for p in players {
                    if let Err(why) = player_exists(big, &p.bank, p.module, p.player, &e.name) {
                        let at = locate_quoted(&text(c, &file), &p.bank, "players");
                        push(c, Severity::Error, &file, at, format!("players: {}: {why}", p.bank));
                    }
                }
            }
            match (&def.route, &buses) {
                (Route::Bus(b), Ok(names)) if !names.iter().any(|n| n.eq_ignore_ascii_case(b)) => {
                    let at = at_key(c, "bus");
                    push(
                        c,
                        Severity::Error,
                        &file,
                        at,
                        format!("bus {b:?} is no bus of the game's mixer ({} buses)", names.len()),
                    );
                }
                (Route::Emitter(e), _) if crate::sfx_override::emitter_bus(e).is_none() => {
                    let list: Vec<&str> = crate::sfx_override::EMITTER_BUSES.iter().map(|(n, _)| *n).collect();
                    let at = at_key(c, "attenuation");
                    push(
                        c,
                        Severity::Error,
                        &file,
                        at,
                        format!("attenuation {e:?}: one of the game's emitters {list:?}"),
                    );
                }
                (Route::Bus(_), Err(e)) => {
                    push(c, Severity::Error, &file, (0, 0), format!("the game's buses cannot be read ({e:#})"))
                }
                _ => {}
            }
        }
        if SoundClass::of(&id) == SoundClass::Speech {
            let at = m.positions.entries.get(&id).copied().unwrap_or((0, 0));
            push(
                c,
                Severity::Note,
                MANIFEST_FILE,
                at,
                format!("{id}: said where the game says this speech event, on its event's voice"),
            );
        }
    }
}

fn player_exists(big: &Path, bank: &str, module: Option<u32>, player: u32, name: &str) -> Result<(), String> {
    let rel = format!("data/xenon/audio/sfx/{}.abk", bank.to_ascii_lowercase());
    let path = crate::music::find_file(big, &rel).ok_or_else(|| format!("no bank {rel} on the disc"))?;
    let data = std::fs::read(&path).map_err(|e| e.to_string())?;
    let abk = ssx_formats::abk::AbkFile::parse(&data).map_err(|e| e.to_string())?;
    let bound = |mi: usize| {
        let m = &abk.modules[mi];
        abk.names.iter().any(|r| {
            r.name == name
                && (r.target == m.offset + 4
                    || (r.target >= m.data_offset + ssx_formats::aems::INSTANCE_HEADER
                        && r.target < m.data_offset + m.data_size))
        })
    };
    let players = |mi: usize| abk.modules[mi].program.iter().filter(|n| n.op == ssx_formats::aems::Op::Player).count();
    let candidates: Vec<usize> = match module {
        Some(mi) if (mi as usize) < abk.modules.len() => vec![mi as usize],
        Some(mi) => return Err(format!("module {mi}: the bank has {}", abk.modules.len())),
        None => (0..abk.modules.len()).collect(),
    };
    let bound: Vec<usize> = candidates.into_iter().filter(|&mi| bound(mi)).collect();
    if bound.is_empty() {
        return Err(format!("no module of the bank plays {name}"));
    }
    if !bound.iter().any(|&mi| (player as usize) < players(mi)) {
        return Err(format!(
            "player {player}: the modules playing {name} have {:?} players",
            bound.iter().map(|&mi| players(mi)).collect::<Vec<_>>()
        ));
    }
    Ok(())
}

fn check_switch_ids(c: &mut Checked) {
    use ssx_content::audio::switches;
    let manifest = text(c, MANIFEST_FILE);
    let mut entries: Vec<(ContentId, Option<usize>)> = c
        .audio
        .music
        .iter()
        .filter(|(id, _, _)| switches::added(id))
        .filter_map(|(id, _, d)| match &**d {
            MusicDef::Playlist { songs, .. } if !songs.is_empty() => Some((id.clone(), Some(songs.len()))),
            MusicDef::Song(_) => Some((id.clone(), None)),
            _ => None,
        })
        .collect();
    entries.sort();
    let mut playlists = std::collections::BTreeMap::new();
    for (id, songs) in &entries {
        let at = locate_key(&manifest, &format!("{:?}", format!("music/{}", id.path())));
        match songs {
            Some(songs) => match switches::song_key(id, *songs) {
                Ok(_) => {
                    playlists.insert(id.clone(), *songs);
                }
                Err(e) => push(c, Severity::Error, MANIFEST_FILE, at, format!("{e}; the game does not use it")),
            },
            None => {
                if let Some((owner, n)) = switches::playlist_song_of(id, &playlists) {
                    let message = format!(
                        "{id}: its id is the music menu's switch id of {owner}'s song {n} (a playlist's songs are \
                         <playlist id>/1 …); the game does not use {id}: give it another path"
                    );
                    push(c, Severity::Error, MANIFEST_FILE, at, message);
                }
            }
        }
    }
}

fn check_music(c: &mut Checked, big: &Path, base: Option<&BTreeSet<ContentId>>) {
    check_switch_ids(c);
    let disc = crate::content::audio::DiscMusic::index(big);
    for (id, file, def) in c.audio.music.clone() {
        let songs: Vec<&ssx_content::audio::music::Song> = match &*def {
            MusicDef::Playlist { songs, .. } => songs.iter().collect(),
            MusicDef::Song(s) => vec![s],
            _ => Vec::new(),
        };
        for s in songs {
            for d in s.drops.iter().flatten() {
                if d.namespace() == BASE_NAMESPACE && base.is_some_and(|b| !b.contains(d)) {
                    let at = locate_quoted(&text(c, &file), &d.to_string(), "drops");
                    push(c, Severity::Error, &file, at, format!("drops: {d}: the base game has no such drop"));
                }
            }
        }
        let track = disc.as_ref().ok().and_then(|d| d.get(&id).cloned());
        let Some(crate::content::audio::DiscTrack::Pathfinder { project, track, mpf }) = track else { continue };
        let Ok(mpf) = std::fs::read(&mpf)
            .map_err(|e| e.to_string())
            .and_then(|b| ssx_formats::mpf::MpfFile::parse(&b).map_err(|e| e.to_string()))
        else {
            push(c, Severity::Error, &file, (0, 0), format!("{id}: the project's .mpf cannot be read"));
            continue;
        };
        match &*def {
            MusicDef::Track { audio: TrackAudio::Sectioned { sections, .. }, .. } => {
                let starts: BTreeSet<u32> = mpf
                    .nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, n)| n.sample == ssx_formats::mpf::SECTION_START && n.track as usize == track)
                    .map(|(i, _)| i as u32)
                    .collect();
                for (node, _) in sections {
                    if !starts.contains(node) {
                        let at = locate_value(&text(c, &file), "node", &node.to_string());
                        push(
                            c,
                            Severity::Error,
                            &file,
                            at,
                            format!(
                                "node {node}: not a section start of {project} track {track} (`content export {id}` lists them)"
                            ),
                        );
                    }
                }
                let missing = starts.iter().filter(|n| !sections.iter().any(|(s, _)| s == *n)).count();
                if missing > 0 {
                    push(
                        c,
                        Severity::Warning,
                        &file,
                        (0, 0),
                        format!("{missing} of the track's {} sections have no region: they are silent", starts.len()),
                    );
                }
            }
            MusicDef::Interactive(score) => {
                if track != 0 {
                    push(
                        c,
                        Severity::Error,
                        &file,
                        (0, 0),
                        format!("{id}: an interactive score replaces a project's whole score: it overrides track 0"),
                    );
                }
                let names: Vec<&str> = if project == "ssxdd" {
                    crate::music::SSXDD_EVENTS.iter().map(|(n, _)| *n).collect()
                } else {
                    Vec::new()
                };
                for key in score.events.keys() {
                    let ok = match key.strip_prefix("0x").and_then(|h| u32::from_str_radix(h, 16).ok()) {
                        Some(v) => mpf.event_by_id(v & 0xFF_FFFF).is_some(),
                        None => names.contains(&key.as_str()),
                    };
                    if !ok {
                        let at = locate_key(&text(c, &file), key);
                        let known = if names.is_empty() {
                            "this project's events have no names: give their values (0x…)".to_string()
                        } else {
                            format!("the project's events are {}", names.join(", "))
                        };
                        push(c, Severity::Error, &file, at, format!("event {key:?} is no event of {project}; {known}"));
                    }
                }
            }
            _ => {}
        }
    }
}

fn totals(c: &mut Checked) {
    let measured: std::collections::BTreeMap<(String, Kind), ssx_content::audio::AudioInfo> =
        c.audio.files.iter().map(|(f, k, i)| ((f.clone(), *k), *i)).collect();
    let info = |f: &str, kind: Kind| measured.get(&(f.to_string(), kind)).copied();
    let (mut decoded, mut sound_files) = (0u64, 0usize);
    for (_, _, def) in &c.audio.sounds {
        let files: BTreeSet<&String> = def.files.iter().collect();
        sound_files += files.len();
        decoded += files
            .into_iter()
            .filter_map(|f| info(f, Kind::Sound))
            .map(|i| i.decoded_bytes(crate::sfx_aems::MIXER_RATE))
            .sum::<u64>();
    }
    if sound_files > BUDGET.sound_files {
        let text = format!(
            "the package's sounds name {sound_files} files; a level takes at most {} (the disc's sounds play for the \
             rest, in the stated order)",
            BUDGET.sound_files
        );
        push(c, Severity::Warning, MANIFEST_FILE, (0, 0), text);
    }
    if decoded > BUDGET.sound_bytes {
        push(
            c,
            Severity::Warning,
            MANIFEST_FILE,
            (0, 0),
            format!(
                "the package's sounds hold {decoded} bytes of samples; a level takes at most {} (the disc's sounds play for the rest, in the stated order)",
                BUDGET.sound_bytes
            ),
        );
    }
    let mut distinct: BTreeSet<(&str, bool)> = BTreeSet::new();
    for (_, _, d) in &c.audio.music {
        let decoded = matches!(&**d, MusicDef::Track { .. } | MusicDef::Interactive(_));
        for f in d.files() {
            distinct.insert((f, decoded));
        }
    }
    let held: u64 = distinct
        .into_iter()
        .filter_map(|(f, decoded)| {
            let i = info(f, Kind::Music)?;
            Some(if decoded { i.frames_at(crate::pathfinder::SAMPLE_RATE) * 8 } else { i.file_bytes })
        })
        .sum();
    let music_files =
        c.audio.music.iter().map(|(_, _, d)| d.files().into_iter().collect::<BTreeSet<_>>().len()).sum::<usize>();
    if music_files > BUDGET.music_files {
        let text = format!(
            "the package's music names {music_files} files; a level takes at most {} (the disc's music plays for the \
             rest)",
            BUDGET.music_files
        );
        push(c, Severity::Warning, MANIFEST_FILE, (0, 0), text);
    }
    if held > BUDGET.music_bytes {
        push(
            c,
            Severity::Warning,
            MANIFEST_FILE,
            (0, 0),
            format!(
                "the package's music holds {held} bytes; a level takes at most {} (the disc's music plays for the rest)",
                BUDGET.music_bytes
            ),
        );
    }
}
