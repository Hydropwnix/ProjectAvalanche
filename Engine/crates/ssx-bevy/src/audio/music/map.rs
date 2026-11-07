use ssx_assets::content::audio::MusicPlan;
use ssx_assets::musicbox::FIRST_PACKAGE_SONG;
use ssx_content::{BASE_NAMESPACE, ContentId};

use super::*;

pub fn load_map_level(
    big_root: Option<&Path>,
    music: Option<&ContentId>,
    event_type: i64,
    song_flags: &SongSwitches,
) -> Option<MusicLevel> {
    let mode = std::env::var("SSX_MUSIC").unwrap_or_default().to_ascii_lowercase();
    if matches!(mode.as_str(), "off" | "0" | "false") {
        return None;
    }
    let Some(big_root) = big_root else {
        ssx_core::eprintln!("warning: music unavailable: no extracted archive root");
        return None;
    };
    let volume = std::env::var("SSX_MUSIC_VOLUME").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.0, 1.0);
    match load(big_root, music, event_type, song_flags) {
        Ok((engine, script, label, remix_type, scope, race)) => {
            let verbose = std::env::var("SSX_MUSIC_LOG").is_ok_and(|v| v == "1");
            Some(MusicLevel { engine, script, label, remix_type, volume, verbose, scope, race })
        }
        Err(e) => {
            ssx_core::eprintln!("warning: music unavailable: {e:#}");
            None
        }
    }
}

fn load(
    big_root: &Path,
    music: Option<&ContentId>,
    event_type: i64,
    song_flags: &SongSwitches,
) -> anyhow::Result<Loaded> {
    let scope = AudioScope::Level { drop: None };
    let plan = ssx_assets::content::GameContent::of(big_root).music_plan(&scope);
    let mut mb = MusicBox::load(big_root)?;
    mb.apply_switches(song_flags);
    if !plan.music.is_empty() {
        mb.add_packages(&plan.music);
    }
    let kind = u32::try_from(event_type).map_or(6, ssx_assets::sfx_game::event_kind);
    let race = RaceType::from_event(0, kind);
    let remix_type = match race {
        Some(RaceType::RaceIt) => 1,
        Some(RaceType::TrickIt) => 0,
        Some(RaceType::SurviveIt) => 2,
        Some(RaceType::Menu) | None => -1,
    };
    let named = music.map(|id| (id, songs_of(&mb, &plan.music, id)));
    let (engine, label) = match (named, race) {
        (Some((id, Ok(ids))), _) => {
            let n = ids.len();
            (LicensedMusic::new(mb, Playlist { ids, next: 0 }), format!("the map's music {id} ({n} songs, looped)"))
        }
        (named, Some(r)) => {
            if let Some((id, Err(why))) = named {
                ssx_core::eprintln!("warning: the map's music {id} does not play ({why}); the event type's music");
            }
            (LicensedMusic::game(mb, r), format!("licensed soundtrack (a map's event type {event_type} → {r:?})"))
        }
        (named, None) => {
            if let Some((id, Err(why))) = named {
                ssx_core::eprintln!("warning: the map's music {id} does not play ({why})");
            }
            let label = format!("no music (a map's event type {event_type}: kind {kind}, songs mask 8, no song)");
            (LicensedMusic::new(mb, Playlist { ids: Vec::new(), next: 0 }), label)
        }
    };
    Ok((Engine::Licensed(Box::new(engine)), None, label, remix_type, scope, race))
}

fn songs_of(mb: &MusicBox, plan: &MusicPlan, id: &ContentId) -> Result<Vec<u32>, String> {
    if id.namespace() == BASE_NAMESPACE {
        let Some(sample) = id.path().strip_prefix("licensed/") else {
            return Err("a Pathfinder track plays only in a Deadly Descent".into());
        };
        let sample: u32 = sample.parse().map_err(|_| format!("{sample:?} is not a song's MBSI id"))?;
        return match mb.song(sample) {
            Some(_) => Ok(vec![sample]),
            None => Err("no such song on the disc".into()),
        };
    }
    let ids: Vec<u32> = (plan.added.iter().enumerate())
        .filter(|(_, t)| t.from.id == *id)
        .filter_map(|(k, _)| u32::try_from(k).ok().and_then(|k| FIRST_PACKAGE_SONG.checked_add(k)))
        .collect();
    if ids.is_empty() {
        return Err("no song of it is taken for a level (its drops, or a stronger package's playlist)".into());
    }
    Ok(ids)
}
