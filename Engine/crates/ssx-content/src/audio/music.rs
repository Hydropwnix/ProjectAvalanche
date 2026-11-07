use serde::Deserialize;

use crate::id::ContentId;
use crate::manifest::line_col;
use crate::material::Problem;

pub use super::music_score::{Action, HELD, Interactive, Section, Stinger, When};

pub const MAX_SONGS: usize = 1024;
pub const MAX_DROPS: usize = 1024;
pub const MAX_TRACK_SECTIONS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopRegion {
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub file: String,
    pub artist: Option<String>,
    pub title: Option<String>,
    pub race_it: Option<bool>,
    pub trick_it: Option<bool>,
    pub survive_it: Option<bool>,
    pub menu: bool,
    pub drops: Option<Vec<ContentId>>,
    pub gain: f32,
    pub looping: Option<LoopRegion>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrackAudio {
    Looped { file: String, region: Option<LoopRegion> },
    Sectioned { file: String, sections: Vec<(u32, LoopRegion)> },
}

#[derive(Clone, Debug, PartialEq)]
pub enum MusicDef {
    Playlist { replace: bool, songs: Vec<Song> },
    Song(Song),
    Track { audio: TrackAudio, gain: f32 },
    Interactive(Interactive),
}

impl MusicDef {
    pub fn files(&self) -> Vec<&str> {
        match self {
            MusicDef::Playlist { songs, .. } => songs.iter().map(|s| s.file.as_str()).collect(),
            MusicDef::Song(s) => vec![s.file.as_str()],
            MusicDef::Track { audio: TrackAudio::Looped { file, .. } | TrackAudio::Sectioned { file, .. }, .. } => {
                vec![file.as_str()]
            }
            MusicDef::Interactive(i) => i.files(),
        }
    }

    pub fn song_file(rel: &str) -> Self {
        MusicDef::Song(Song {
            file: rel.to_string(),
            artist: None,
            title: None,
            race_it: None,
            trick_it: None,
            survive_it: None,
            menu: false,
            drops: None,
            gain: 1.0,
            looping: None,
        })
    }
}

#[derive(Deserialize)]
struct TypeOnly {
    #[serde(rename = "type")]
    kind: Option<toml::Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlaylist {
    #[serde(rename = "type")]
    _kind: String,
    replace: Option<bool>,
    song: toml::Spanned<Vec<RawSong>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawSong {
    #[serde(rename = "type")]
    _kind: Option<String>,
    file: toml::Spanned<String>,
    #[serde(rename = "Artist")]
    artist: Option<toml::Spanned<String>>,
    #[serde(rename = "Title")]
    title: Option<toml::Spanned<String>>,
    #[serde(rename = "RaceIt")]
    race_it: Option<bool>,
    #[serde(rename = "TrickIt")]
    trick_it: Option<bool>,
    #[serde(rename = "SurviveIt")]
    survive_it: Option<bool>,
    menu: Option<bool>,
    drops: Option<toml::Spanned<Vec<toml::Spanned<String>>>>,
    gain: Option<toml::Spanned<f64>>,
    #[serde(rename = "loop")]
    looping: Option<toml::Spanned<Vec<f64>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrack {
    #[serde(rename = "type")]
    _kind: String,
    file: toml::Spanned<String>,
    gain: Option<toml::Spanned<f64>>,
    #[serde(rename = "loop")]
    looping: Option<toml::Spanned<Vec<f64>>>,
    section: Option<toml::Spanned<Vec<RawTrackSection>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrackSection {
    node: toml::Spanned<i64>,
    start: f64,
    end: f64,
}

pub(super) struct At<'a> {
    pub text: &'a str,
    pub problems: Vec<Problem>,
}

impl At<'_> {
    pub fn at(&mut self, span: std::ops::Range<usize>, m: String) {
        let (line, column) = line_col(self.text, span.start);
        self.problems.push((line, column, m));
    }

    pub fn audio_file(&mut self, s: &toml::Spanned<String>) -> String {
        let p = s.get_ref();
        if let Err(e) = crate::path::check_package_path(p, false) {
            self.at(s.span(), e.to_string());
        } else if super::Format::from_name(p).is_err() {
            self.at(s.span(), format!("{}: not .ogg, .flac or .wav", ssx_core::text::quoted(p)));
        }
        p.clone()
    }

    pub fn gain(&mut self, g: &Option<toml::Spanned<f64>>) -> f32 {
        match g {
            Some(v) if v.get_ref().is_finite() && (0.0..=4.0).contains(v.get_ref()) => *v.get_ref() as f32,
            Some(v) => {
                self.at(v.span(), "gain: from 0 to 4".into());
                1.0
            }
            None => 1.0,
        }
    }

    pub fn region(&mut self, r: &toml::Spanned<Vec<f64>>) -> Option<LoopRegion> {
        match r.get_ref().as_slice() {
            [s, e] if s.is_finite() && e.is_finite() && 0.0 <= *s && s < e && *e <= 86_400.0 => {
                Some(LoopRegion { start: *s, end: *e })
            }
            _ => {
                self.at(r.span(), "loop: [start, end] in seconds, start from 0 and below end".into());
                None
            }
        }
    }
}

pub fn parse_toml(bytes: &[u8], override_of: Option<&ContentId>) -> Result<MusicDef, Vec<Problem>> {
    crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let syntax = |e: toml::de::Error| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    };
    let kind: TypeOnly = toml::from_str(text).map_err(syntax)?;
    let Some(k) = kind.kind else {
        return Err(vec![(0, 0, "type: playlist, song, track or interactive".into())]);
    };
    let licensed = override_of.is_some_and(|id| id.path().starts_with("licensed/"));
    let pathfinder = override_of.is_some_and(|id| id.path().starts_with("pathfinder/"));
    let allowed = match (override_of, k.get_ref().as_str()) {
        (None, "playlist" | "song") => true,
        (Some(_), "song") => licensed,
        (Some(_), "track" | "interactive") => pathfinder,
        _ => false,
    };
    if !allowed {
        let (line, column) = line_col(text, k.span().start);
        let what = match override_of {
            None => "an added music entry is a playlist or a song",
            Some(_) if licensed => "a licensed song's override is a song",
            Some(_) if pathfinder => "a Pathfinder track's override is a track or an interactive score",
            Some(_) => "this music id cannot be overridden",
        };
        return Err(vec![(line, column, format!("type {}: {what}", ssx_core::text::quoted(k.get_ref())))]);
    }
    let mut at = At { text, problems: Vec::new() };
    let def = match k.get_ref().as_str() {
        "playlist" => {
            let raw: RawPlaylist = toml::from_str(text).map_err(syntax)?;
            if raw.song.get_ref().is_empty() || raw.song.get_ref().len() > MAX_SONGS {
                at.at(raw.song.span(), format!("song: 1 to {MAX_SONGS} songs"));
            }
            let songs = raw.song.get_ref().iter().map(|s| song(&mut at, s)).collect();
            MusicDef::Playlist { replace: raw.replace.unwrap_or(false), songs }
        }
        "song" => {
            let raw: RawSong = toml::from_str(text).map_err(syntax)?;
            MusicDef::Song(song(&mut at, &raw))
        }
        "track" => track(&mut at, toml::from_str(text).map_err(syntax)?),
        _ => MusicDef::Interactive(super::music_score::parse(&mut at, text).map_err(syntax)?),
    };
    if at.problems.is_empty() { Ok(def) } else { Err(at.problems) }
}

pub(super) fn song(at: &mut At<'_>, s: &RawSong) -> Song {
    let file = at.audio_file(&s.file);
    let text = |v: &Option<toml::Spanned<String>>, what: &str, at: &mut At<'_>| {
        v.as_ref().and_then(|t| {
            let ok = (1..=256).contains(&t.get_ref().len()) && !t.get_ref().chars().any(char::is_control);
            if !ok {
                at.at(t.span(), format!("{what}: 1 to 256 bytes on one line"));
            }
            ok.then(|| t.get_ref().clone())
        })
    };
    let drops = s.drops.as_ref().map(|d| {
        if d.get_ref().len() > MAX_DROPS {
            at.at(d.span(), format!("drops: at most {MAX_DROPS}"));
        }
        d.get_ref()
            .iter()
            .filter_map(|x| match x.get_ref().parse::<ContentId>() {
                Ok(id) if id.kind() == crate::id::Kind::Drop => Some(id),
                _ => {
                    at.at(
                        x.span(),
                        format!("{}: a drop id (`ssx:drop/<ui_drop>`)", ssx_core::text::quoted(x.get_ref())),
                    );
                    None
                }
            })
            .collect()
    });
    Song {
        file,
        artist: text(&s.artist, "Artist", at),
        title: text(&s.title, "Title", at),
        race_it: s.race_it,
        trick_it: s.trick_it,
        survive_it: s.survive_it,
        menu: s.menu.unwrap_or(false),
        drops,
        gain: at.gain(&s.gain),
        looping: s.looping.as_ref().and_then(|r| at.region(r)),
    }
}

fn track(at: &mut At<'_>, raw: RawTrack) -> MusicDef {
    let file = at.audio_file(&raw.file);
    let gain = at.gain(&raw.gain);
    let audio = match (&raw.looping, &raw.section) {
        (Some(_), Some(s)) => {
            at.at(s.span(), "a track is looped (`loop`) or in sections (`[[section]]`), not both".into());
            TrackAudio::Looped { file, region: None }
        }
        (_, Some(s)) => {
            let list = s.get_ref();
            if list.is_empty() || list.len() > MAX_TRACK_SECTIONS {
                at.at(s.span(), format!("section: 1 to {MAX_TRACK_SECTIONS} sections"));
            }
            let mut sections: Vec<(u32, LoopRegion)> = Vec::new();
            for sec in list {
                let node = u32::try_from(*sec.node.get_ref()).ok().filter(|&n| n < 65_536);
                let Some(node) = node else {
                    at.at(sec.node.span(), "node: the section's first node, from 0 to 65535".into());
                    continue;
                };
                if sections.iter().any(|(n, _)| *n == node) {
                    at.at(sec.node.span(), format!("node {node} is given twice"));
                }
                let ok = sec.start.is_finite() && sec.end.is_finite() && 0.0 <= sec.start && sec.start < sec.end;
                if !ok {
                    at.at(sec.node.span(), format!("section of node {node}: start from 0 and below end"));
                    continue;
                }
                sections.push((node, LoopRegion { start: sec.start, end: sec.end }));
            }
            TrackAudio::Sectioned { file, sections }
        }
        (r, None) => TrackAudio::Looped { file, region: r.as_ref().and_then(|r| at.region(r)) },
    };
    MusicDef::Track { audio, gain }
}
