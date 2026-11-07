use std::collections::{BTreeMap, BTreeSet};

use ssx_content::ContentId;
use ssx_content::audio::music::{LoopRegion, Song as SongDef};

use super::{MusicBox, RaceEvent, Song, select_levelled};
use crate::content::audio::{MusicFile, MusicPlan};

pub const FIRST_PACKAGE_SONG: u32 = 0xF000_0000;

#[derive(Clone, Debug)]
pub struct SongAudio {
    pub file: MusicFile,
    pub gain: f32,
    pub looping: Option<LoopRegion>,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct PackageSongs {
    disc_info: BTreeMap<u32, Vec<(String, String)>>,
    pub audio: BTreeMap<u32, SongAudio>,
    pub added: Vec<u32>,
    pub keys: BTreeMap<u32, ContentId>,
    pub menu: BTreeSet<u32>,
    pub replace_disc: bool,
}

impl PackageSongs {
    pub fn is_empty(&self) -> bool {
        self.audio.is_empty() && !self.replace_disc
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SongSwitches {
    pub disc: Vec<(u32, bool)>,
    pub packages: Vec<(ContentId, bool)>,
}

impl SongSwitches {
    pub fn off(&self) -> usize {
        self.disc.iter().filter(|(_, on)| !on).count() + self.packages.iter().filter(|(_, on)| !on).count()
    }
}

fn flag(v: bool) -> String {
    (if v { "True" } else { "False" }).to_string()
}

fn info(def: &SongDef, index: Option<usize>, base: &[(String, String)]) -> Vec<(String, String)> {
    let mut info: Vec<(String, String)> = base.to_vec();
    let mut set = |k: &str, v: Option<String>| {
        if let Some(v) = v {
            match info.iter_mut().find(|(key, _)| key == k) {
                Some(e) => e.1 = v,
                None => info.push((k.to_string(), v)),
            }
        }
    };
    let added = index.is_some();
    set("Index", index.map(|i| i.to_string()));
    set("Artist", def.artist.clone());
    let stem = || def.file.rsplit('/').next().map(|f| f.rsplit_once('.').map_or(f, |(stem, _)| stem).to_string());
    set("Title", def.title.clone().or_else(|| added.then(stem).flatten()));
    set("RaceIt", def.race_it.or(added.then_some(true)).map(flag));
    set("TrickIt", def.trick_it.or(added.then_some(true)).map(flag));
    set("SurviveIt", def.survive_it.or(added.then_some(true)).map(flag));
    info
}

impl MusicBox {
    pub fn add_packages(&mut self, plan: &MusicPlan) {
        let mut p = PackageSongs { replace_disc: plan.replace_disc, ..Default::default() };
        let first_index =
            self.songs.iter().filter_map(|s| s.get("Index")?.parse::<usize>().ok()).max().map_or(0, |m| m + 1);
        for (k, t) in plan.added.iter().enumerate() {
            let id = FIRST_PACKAGE_SONG + k as u32;
            let record = Song { id, unk: [1, 1, 1, 100, 100, 4], info: info(&t.song, Some(first_index + k), &[]) };
            self.songs.push(record);
            p.added.push(id);
            if let Some(key) = &t.key {
                p.keys.insert(id, key.clone());
            }
            if t.song.menu {
                p.menu.insert(id);
            }
            let label = format!("{} ({})", t.from.id, t.from.package);
            p.audio.insert(id, SongAudio { file: t.file.clone(), gain: t.song.gain, looping: t.song.looping, label });
        }
        for (&sample, t) in &plan.licensed {
            if let Some(s) = self.songs.iter_mut().find(|s| s.id == sample) {
                p.disc_info.insert(sample, s.info.clone());
                s.info = info(&t.song, None, &s.info);
            }
            let label = format!("{} ({})", t.from.id, t.from.package);
            p.audio
                .insert(sample, SongAudio { file: t.file.clone(), gain: t.song.gain, looping: t.song.looping, label });
        }
        self.package = p;
        self.apply_package_switches();
    }

    pub fn apply_switches(&mut self, s: &SongSwitches) {
        self.apply_song_flags(&s.disc);
        self.package_switches = s.packages.iter().cloned().collect();
        self.apply_package_switches();
    }

    fn apply_package_switches(&mut self) {
        for (&id, key) in &self.package.keys {
            let on = self.package_switches.get(key).copied().unwrap_or(true);
            if let Some(s) = self.songs.iter_mut().find(|s| s.id == id) {
                s.unk[2] = u32::from(on);
            }
        }
    }

    pub fn menu_list(&self) -> Vec<(u32, Option<ContentId>)> {
        let disc = crate::sfx::store::in_game_music(&self.songs).into_iter().map(|(id, _)| (id, None));
        disc.chain(self.package.added.iter().map(|id| (*id, self.package.keys.get(id).cloned()))).collect()
    }

    pub fn clear_packages(&mut self) {
        let p = std::mem::take(&mut self.package);
        self.songs.retain(|s| s.id < FIRST_PACKAGE_SONG);
        for (id, info) in p.disc_info {
            if let Some(s) = self.songs.iter_mut().find(|s| s.id == id) {
                s.info = info;
            }
        }
    }

    pub fn package_audio(&self, id: u32) -> Option<&SongAudio> {
        self.package.audio.get(&id)
    }

    pub(super) fn with_packages(&self, ev: &RaceEvent, disc: Vec<u32>) -> Vec<u32> {
        let p = &self.package;
        let mut ids: Vec<u32> = if p.replace_disc { Vec::new() } else { disc };
        let menu = *ev == RaceEvent::new(super::RaceType::Menu);
        ids.extend(p.added.iter().filter(|id| !menu || p.menu.contains(id)));
        select_levelled(&self.songs, &ids)
    }
}
