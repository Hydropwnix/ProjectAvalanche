pub mod asm;
pub mod write;

use std::collections::BTreeMap;

use ssx_formats::gfx::avm1::Action;
use ssx_formats::gfx::{Limits, Movie, Tag, TagRecord, tag_name};

use crate::id::{ContentId, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Fws,
    Cws,
    Gfx,
    Cfx,
}

impl Container {
    pub fn of(bytes: &[u8]) -> Result<Container, String> {
        match bytes.get(..3) {
            Some(b"FWS") => Ok(Container::Fws),
            Some(b"CWS") => Ok(Container::Cws),
            Some(b"GFX") => Ok(Container::Gfx),
            Some(b"CFX") => Ok(Container::Cfx),
            Some(b"ZWS") => {
                Err("an LZMA movie (ZWS) is not read: the player plays FWS, CWS, GFX and CFX movies of SWF \
                                 10 at most (Scaleform GFx 4.0.12); save it uncompressed or with zlib"
                    .into())
            }
            Some(m) => Err(format!("not a movie: it starts {:02x?} (FWS, CWS, GFX or CFX expected)", m)),
            None => Err("not a movie: shorter than its signature".into()),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Container::Fws => "FWS",
            Container::Cws => "CWS",
            Container::Gfx => "GFX",
            Container::Cfx => "CFX",
        }
    }
}

pub const VERSIONS: std::ops::RangeInclusive<u8> = 7..=10;

fn unplayed(code: u16) -> Option<&'static str> {
    Some(match code {
        6 | 8 | 20 | 21 | 35 | 36 | 90 => {
            "an embedded image is not drawn: name the image as a file (DefineExternalImage, a \
                                           package texture under the movie's folder)"
        }
        14 | 15 | 18 | 19 | 45 | 89 => "sounds in a movie are not played (the game's sounds are the `sound` kind)",
        7 | 17 | 23 | 34 => "button tags are not run (clips with onPress / onRelease are the movies' buttons)",
        60 | 61 => "video in a movie is not played (movies are the `movie` kind)",
        10 | 13 | 62 => "a DefineFont (1) font has no layout the player reads (DefineFont2 / DefineFont3)",
        _ => return None,
    })
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MovieInfo {
    pub container: Option<Container>,
    pub version: u8,
    pub length: u32,
    pub totals: [u64; 5],
    pub blocks: usize,
    pub actions: usize,
    pub imports: Vec<String>,
    pub exports: Vec<String>,
    pub images: Vec<String>,
    pub fonts: Vec<String>,
    pub unplayed: Vec<(&'static str, usize, &'static str)>,
}

impl MovieInfo {
    pub fn has_actionscript(&self) -> bool {
        self.actions > 0
    }
}

pub fn check_movie(bytes: &[u8], limits: &Limits) -> Result<MovieInfo, String> {
    let container = Container::of(bytes)?;
    let version = bytes.get(3).copied().unwrap_or(0);
    if !VERSIONS.contains(&version) {
        return Err(format!(
            "SWF version {version}: the player runs ActionScript 2 movies of SWF {} to {} (the disc's are 7, 8, 10)",
            VERSIONS.start(),
            VERSIONS.end()
        ));
    }
    let mut budget = ssx_formats::gfx::Budget::new(*limits);
    let movie = Movie::parse_in(bytes, &mut budget).map_err(|e| e.to_string())?;
    let mut info = MovieInfo {
        container: Some(container),
        version,
        length: movie.file_length,
        totals: budget.totals(),
        ..Default::default()
    };
    let mut unplayed_counts: BTreeMap<u16, usize> = BTreeMap::new();
    for (_, t) in movie.all_tags() {
        survey(t, &mut info, &mut unplayed_counts)?;
    }
    info.unplayed = unplayed_counts
        .into_iter()
        .filter_map(|(code, n)| unplayed(code).map(|why| (tag_name(code).unwrap_or("?"), n, why)))
        .collect();
    info.imports.sort();
    info.imports.dedup();
    info.fonts.sort();
    info.fonts.dedup();
    Ok(info)
}

fn block(info: &mut MovieInfo, actions: &[Action]) {
    info.blocks += 1;
    info.actions += actions.len();
}

fn survey(t: &TagRecord, info: &mut MovieInfo, unplayed_counts: &mut BTreeMap<u16, usize>) -> Result<(), String> {
    match &t.tag {
        Tag::DoAction(a) => block(info, a),
        Tag::DoInitAction { actions, .. } => block(info, actions),
        Tag::PlaceObject(p) => {
            for r in p.clip_actions.iter().flat_map(|c| &c.records) {
                block(info, &r.actions);
            }
        }
        Tag::FileAttributes(f) if f & 0x08 != 0 => {
            return Err("the movie is ActionScript 3 (FileAttributes): the player runs ActionScript 2".into());
        }
        Tag::ImportAssets { url, .. } => info.imports.push(url.clone()),
        Tag::ExportAssets(a) => info.exports.extend(a.iter().map(|(_, n)| n.clone())),
        Tag::ExternalImage(e) => info.images.push(e.file_name.clone()),
        Tag::Font(f) => info.fonts.push(f.name.clone()),
        Tag::Unknown(_) if matches!(t.code, 72 | 82) => {
            return Err("the movie carries ActionScript 3 (DoABC): the player runs ActionScript 2".into());
        }
        Tag::Unknown(_) => *unplayed_counts.entry(t.code).or_default() += 1,
        _ => {}
    }
    Ok(())
}

pub fn movie_path(id: &ContentId) -> Option<String> {
    if id.kind() != Kind::Ui {
        return None;
    }
    Some(match id.namespace() {
        crate::BASE_NAMESPACE | crate::PORT_NAMESPACE => format!("data/common/fe/{}.gfx", id.path()),
        ns => format!("data/common/fe/mods/{ns}/{}.gfx", id.path()),
    })
}

pub fn movie_id(path: &str, port: bool) -> Option<ContentId> {
    let rest = path.strip_prefix("data/common/fe/")?;
    let stem = rest.strip_suffix(".gfx").or_else(|| rest.strip_suffix(".swf"))?;
    match stem.strip_prefix("mods/") {
        Some(m) => {
            let (ns, p) = m.split_once('/')?;
            if ns == crate::BASE_NAMESPACE || ns == crate::PORT_NAMESPACE {
                return None;
            }
            ContentId::new(ns, Kind::Ui, p).ok()
        }
        None => ContentId::new(if port { crate::PORT_NAMESPACE } else { crate::BASE_NAMESPACE }, Kind::Ui, stem).ok(),
    }
}

#[cfg(test)]
mod tests;
