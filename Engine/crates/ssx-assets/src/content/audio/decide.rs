use std::collections::BTreeMap;
use std::sync::Arc;

use ssx_content::audio::music::{Interactive, MusicDef, Song, TrackAudio};
use ssx_content::audio::sounds::{SoundClass, SoundDef};
use ssx_content::audio::switches;
use ssx_content::audio::{AudioInfo, Limits, Pcm, decode_file};
use ssx_content::registry::MountInfo;
use ssx_content::{ContentId, Kind, Layer, Registry, Source};

use super::disc::{DiscMusic, DiscSounds, DiscTrack};
use crate::sfx_override::{OverrideSound, Samples48, SoundOverrides};

const MUSIC_RATE: u32 = crate::pathfinder::SAMPLE_RATE;

pub type ReadFile<'a> = dyn Fn(&ContentId, &str, Layer, &str) -> Result<Arc<[u8]>, String> + Sync + 'a;
pub type OpenFile = Arc<dyn Fn() -> Result<Arc<[u8]>, String> + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct AudioBudget {
    pub sound_bytes: u64,
    pub sound_files: usize,
    pub music_bytes: u64,
    pub music_files: usize,
}

pub const BUDGET: AudioBudget =
    AudioBudget { sound_bytes: 512 << 20, sound_files: 4096, music_bytes: 1 << 30, music_files: 2048 };

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AudioScope {
    Level { drop: Option<String> },
    FrontEnd,
    MusicMenu,
}

#[derive(Clone)]
pub struct MusicFile {
    pub rel: String,
    pub info: AudioInfo,
    pub(super) read: OpenFile,
}

impl MusicFile {
    pub fn read(&self) -> Result<Arc<[u8]>, String> {
        (self.read)()
    }
}

impl std::fmt::Debug for MusicFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MusicFile({}, {:.1} s)", self.rel, self.info.seconds())
    }
}

#[derive(Clone, Debug)]
pub struct Taken {
    pub id: ContentId,
    pub package: String,
}

#[derive(Clone, Debug)]
pub struct TakenSong {
    pub from: Taken,
    pub song: Song,
    pub file: MusicFile,
    pub key: Option<ContentId>,
}

#[derive(Clone, Debug)]
pub struct TakenTrack {
    pub from: Taken,
    pub audio: TrackAudio,
    pub gain: f32,
    pub file: MusicFile,
}

#[derive(Clone, Debug)]
pub struct TakenScore {
    pub from: Taken,
    pub score: Interactive,
    pub files: BTreeMap<String, MusicFile>,
}

#[derive(Clone, Debug, Default)]
pub struct MusicPlan {
    pub added: Vec<TakenSong>,
    pub replace_disc: bool,
    pub licensed: BTreeMap<u32, TakenSong>,
    pub tracks: BTreeMap<(String, usize), TakenTrack>,
    pub scores: BTreeMap<String, TakenScore>,
}

impl MusicPlan {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && !self.replace_disc
            && self.licensed.is_empty()
            && self.tracks.is_empty()
            && self.scores.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct SoundDecision {
    pub sounds: SoundOverrides,
    pub notices: Vec<String>,
    pub bytes: u64,
    pub added: usize,
}

impl SoundDecision {
    pub fn summary(&self, lang: Option<&str>) -> String {
        format!(
            "content: sounds (speech {}): {} effects and {} speech events replaced, {} bytes of samples; {} added sounds kept for scripts; {} not taken",
            lang.unwrap_or("none"),
            self.sounds.effects.len(),
            self.sounds.speech.len(),
            self.bytes,
            self.added,
            self.notices.len()
        )
    }
}

#[derive(Debug, Default)]
pub struct MusicDecision {
    pub music: MusicPlan,
    pub notices: Vec<String>,
    pub bytes: u64,
}

impl MusicDecision {
    pub fn summary(&self, scope: &AudioScope) -> String {
        let m = &self.music;
        format!(
            "content: music for {scope:?}: {} songs added{}, {} licensed songs and {} Pathfinder tracks replaced, {} scores; {} bytes held; {} not taken",
            m.added.len(),
            if m.replace_disc { " (the disc's songs replaced)" } else { "" },
            m.licensed.len(),
            m.tracks.len(),
            m.scores.len(),
            self.bytes,
            self.notices.len()
        )
    }
}

fn order(m: &MountInfo<'_>) -> (std::cmp::Reverse<Layer>, std::cmp::Reverse<i32>) {
    (std::cmp::Reverse(m.layer), std::cmp::Reverse(m.priority))
}

fn packages<'r>(registry: &'r Registry, id: &ContentId) -> Vec<(MountInfo<'r>, &'r dyn Source)> {
    registry.providers(id).filter(|(m, _)| m.layer > Layer::Base).collect()
}

struct SoundTake {
    id: ContentId,
    package: String,
    layer: Layer,
    def: SoundDef,
    files: Vec<(String, AudioInfo)>,
    cost: u64,
    target: SoundTarget,
}

enum SoundTarget {
    Effect(String, String),
    Speech(String, String),
}

pub(super) fn decide_sounds(
    registry: &Registry,
    lang: Option<&str>,
    sounds: Option<&DiscSounds>,
    voice_lines: &[(ContentId, ContentId)],
    reader: &ReadFile<'_>,
) -> SoundDecision {
    let mut d = SoundDecision::default();
    let (mut taken, mut files_taken) = (Vec::<SoundTake>::new(), 0usize);
    let mut ids: Vec<(MountInfo<'_>, ContentId)> = Vec::new();
    for id in registry.list_above(Layer::Base, Kind::Sound) {
        if let Some((m, _)) = packages(registry, &id).into_iter().next() {
            ids.push((m, id));
        }
    }
    ids.sort_by(|a, b| order(&a.0).cmp(&order(&b.0)).then_with(|| a.1.cmp(&b.1)));
    for (_, id) in &ids {
        let class = SoundClass::of(id);
        let target = match class {
            SoundClass::Added => {
                d.added += 1;
                continue;
            }
            SoundClass::Trigger => {
                d.notices.push(format!(
                    "content: {id} is not used: a trigger of the speech's context module says what its contexts \
                     choose; a package replaces the speech events they fire"
                ));
                continue;
            }
            SoundClass::MusicEvent => {
                d.notices.push(format!("content: {id} is not used: the licensed music's selection is music.toml's"));
                continue;
            }
            SoundClass::Speech => {
                let Some(lang) = lang else { continue };
                if !id.path().starts_with(&format!("speech/{lang}/")) || voice_lines.iter().any(|(e, _)| e == id) {
                    continue;
                }
                match sounds.and_then(|s| s.get(id)) {
                    Some(s) => SoundTarget::Speech(lang.to_string(), s.name.clone()),
                    None => {
                        d.notices.push(format!("content: {id} is not used: no speech event of the disc's"));
                        continue;
                    }
                }
            }
            SoundClass::Effect => match sounds.and_then(|s| s.get(id)) {
                Some(s) => SoundTarget::Effect(s.file.clone(), s.name.clone()),
                None => {
                    d.notices.push(format!("content: {id} is not used: no sound of the disc's has this id"));
                    continue;
                }
            },
        };
        taken.extend(take(&mut d, registry, id, target, &mut files_taken));
    }
    for (event, sound) in voice_lines {
        let Some(lang) = lang else { break };
        if !event.path().starts_with(&format!("speech/{lang}/")) {
            continue;
        }
        let Some(e) = sounds.and_then(|s| s.get(event)) else {
            d.notices.push(format!(
                "content: the voice line {sound} for {event} is not used: no speech event of the disc's"
            ));
            continue;
        };
        let target = SoundTarget::Speech(lang.to_string(), e.name.clone());
        taken.extend(take(&mut d, registry, sound, target, &mut files_taken));
    }
    decode_sounds(&mut d, taken, reader);
    d
}

fn take(
    d: &mut SoundDecision,
    registry: &Registry,
    id: &ContentId,
    target: SoundTarget,
    files_taken: &mut usize,
) -> Option<SoundTake> {
    let mut chosen = None;
    for (m, src) in packages(registry, id) {
        let def = match src.load(id).map(|a| a.into_value::<SoundDef>()) {
            Ok(Some(def)) => def,
            Ok(None) => continue,
            Err(e) => {
                d.notices.push(format!("content: {id} from {} ({}) not used: {e}", m.name, m.layer));
                continue;
            }
        };
        let probe = match src.audio_probe(id) {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                d.notices.push(format!("content: {id} from {} ({}) not used: {e}", m.name, m.layer));
                continue;
            }
            None => continue,
        };
        let unique: BTreeMap<&str, &AudioInfo> = probe.iter().map(|(r, i)| (r.as_str(), i)).collect();
        let cost: u64 = unique.values().map(|i| i.decoded_bytes(crate::sfx_aems::MIXER_RATE)).sum();
        let count = unique.len();
        let over_bytes = d.bytes + cost > BUDGET.sound_bytes;
        let over_files = *files_taken + count > BUDGET.sound_files;
        if over_bytes || over_files {
            let why = if over_bytes {
                format!(
                    "the scope's package sounds would hold {} bytes of samples; at most {}",
                    d.bytes + cost,
                    BUDGET.sound_bytes
                )
            } else {
                format!(
                    "the scope's package sounds would be {} files; at most {}",
                    *files_taken + count,
                    BUDGET.sound_files
                )
            };
            d.notices.push(format!("content: {id} from {} not used (the disc's is): {why}", m.name));
            break;
        }
        d.bytes += cost;
        *files_taken += count;
        let files = probe;
        chosen = Some(SoundTake {
            id: id.clone(),
            package: m.name.to_string(),
            layer: m.layer,
            def: (*def).clone(),
            files,
            cost,
            target,
        });
        break;
    }
    chosen
}

fn distinct(files: &[(String, AudioInfo)]) -> std::collections::BTreeSet<String> {
    files.iter().map(|(f, _)| f.clone()).collect()
}

pub(super) fn as_measured(rel: &str, measured: Option<AudioInfo>, pcm: &Pcm) -> Result<(), String> {
    let Some(m) = measured else { return Err(format!("{rel}: not measured")) };
    let frames = pcm.frames() as u64;
    if (pcm.channels, pcm.rate, frames) != (m.channels, m.rate, m.frames) {
        return Err(format!(
            "{rel}: the file changed since it was measured ({} frames at {} Hz × {} then, {frames} at {} Hz × {} now)",
            m.frames, m.rate, m.channels, pcm.rate, pcm.channels
        ));
    }
    Ok(())
}

fn decode_sounds(d: &mut SoundDecision, taken: Vec<SoundTake>, reader: &ReadFile<'_>) {
    let jobs: Vec<(usize, String)> =
        taken.iter().enumerate().flat_map(|(k, t)| distinct(&t.files).into_iter().map(move |f| (k, f))).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    type Done = Vec<(usize, String, Result<Arc<Samples48>, String>)>;
    let results: Vec<Done> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some((k, rel)) = jobs.get(i) else { return out };
                        let t = &taken[*k];
                        let measured = t.files.iter().find(|(f, _)| f == rel).map(|(_, m)| *m);
                        let got = reader(&t.id, &t.package, t.layer, rel)
                            .and_then(|bytes| decode_file(rel, bytes, &Limits::sound()).map_err(|e| e.to_string()))
                            .and_then(|pcm| as_measured(rel, measured, &pcm).map(|()| pcm))
                            .map(|pcm| super::convert::samples48(rel, pcm));
                        out.push((*k, rel.clone(), got));
                    }
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap_or_default()).collect()
    });
    let mut sets: Vec<BTreeMap<String, Arc<Samples48>>> = vec![BTreeMap::new(); taken.len()];
    let mut failed: Vec<Option<String>> = vec![None; taken.len()];
    for (k, rel, got) in results.into_iter().flatten() {
        match got {
            Ok(s) => {
                sets[k].insert(rel, s);
            }
            Err(e) => failed[k] = Some(e),
        }
    }
    for (k, t) in taken.into_iter().enumerate() {
        if sets[k].len() != distinct(&t.files).len() {
            let why = failed[k].take().unwrap_or_else(|| "a file's decoder ended without a result".into());
            d.notices.push(format!("content: {} from {} not used (the disc's is): {why}", t.id, t.package));
            d.bytes = d.bytes.saturating_sub(t.cost);
            continue;
        }
        let set: Vec<Arc<Samples48>> = t.files.iter().filter_map(|(f, _)| sets[k].get(f).cloned()).collect();
        let sound = Arc::new(OverrideSound::new(&t.id.to_string(), &t.package, set, &t.def));
        match t.target {
            SoundTarget::Effect(file, name) => d.sounds.effects.insert((file, name), sound),
            SoundTarget::Speech(lang, event) => d.sounds.speech.insert((lang, event), sound),
        };
    }
}

pub(super) fn decide_music(
    registry: &Registry,
    scope: &AudioScope,
    disc: Option<&DiscMusic>,
    music_reader: &dyn Fn(&ContentId, &str, Layer, &str) -> OpenFile,
) -> MusicDecision {
    let mut d = MusicDecision::default();
    let mut ids: Vec<(MountInfo<'_>, ContentId)> = Vec::new();
    for id in registry.list_above(Layer::Base, Kind::Music) {
        if let Some((m, _)) = packages(registry, &id).into_iter().next() {
            ids.push((m, id));
        }
    }
    ids.sort_by(|a, b| order(&a.0).cmp(&order(&b.0)).then_with(|| a.1.cmp(&b.1)));
    let mut files = 0usize;
    let drop = match scope {
        AudioScope::Level { drop } => drop.clone(),
        AudioScope::FrontEnd | AudioScope::MusicMenu => None,
    };
    let mut playlists: BTreeMap<ContentId, usize> = BTreeMap::new();
    for (_, id) in &ids {
        let disc_track = disc.and_then(|m| m.get(id)).cloned();
        if id.namespace() == ssx_content::BASE_NAMESPACE && disc_track.is_none() {
            d.notices.push(format!("content: {id} is not used: no music of the disc's has this id"));
            continue;
        }
        for (m, src) in packages(registry, id) {
            let def = match src.load(id).map(|a| a.into_value::<MusicDef>()) {
                Ok(Some(def)) => def,
                Ok(None) => continue,
                Err(e) => {
                    d.notices.push(format!("content: {id} from {} ({}) not used: {e}", m.name, m.layer));
                    continue;
                }
            };
            match &*def {
                MusicDef::Playlist { songs, .. } if switches::added(id) && !songs.is_empty() => {
                    if let Err(e) = switches::song_key(id, songs.len()) {
                        d.notices.push(format!("content: {id} from {} not used: {e}", m.name));
                        break;
                    }
                    playlists.insert(id.clone(), songs.len());
                }
                MusicDef::Song(_) if switches::added(id) => {
                    if let Some((owner, n)) = switches::playlist_song_of(id, &playlists) {
                        d.notices.push(format!(
                            "content: {id} from {} not used: its switch id is {owner}'s song {n}'s",
                            m.name
                        ));
                        break;
                    }
                }
                _ => {}
            }
            let wanted = |song: &Song| match scope {
                AudioScope::FrontEnd => song.menu,
                AudioScope::MusicMenu => true,
                AudioScope::Level { .. } => {
                    song.drops.as_ref().is_none_or(|ds| ds.iter().any(|x| Some(x.path()) == drop.as_deref()))
                }
            };
            let project_here = |p: &str| match scope {
                AudioScope::FrontEnd => p == "ssx_fe",
                AudioScope::MusicMenu => false,
                AudioScope::Level { .. } => p != "ssx_fe",
            };
            let relevant = match (&*def, &disc_track) {
                (MusicDef::Playlist { replace, songs }, _) => *replace || songs.iter().any(wanted),
                (MusicDef::Song(s), None) => wanted(s),
                (MusicDef::Song(_), Some(_)) => true,
                (MusicDef::Track { .. } | MusicDef::Interactive(_), Some(DiscTrack::Pathfinder { project, .. })) => {
                    project_here(project)
                }
                _ => false,
            };
            if !relevant {
                break;
            }
            let probe = match src.audio_probe(id) {
                Some(Ok(p)) => p,
                Some(Err(e)) => {
                    d.notices.push(format!("content: {id} from {} ({}) not used: {e}", m.name, m.layer));
                    continue;
                }
                None => continue,
            };
            let infos: BTreeMap<String, AudioInfo> = probe.into_iter().collect();
            let counted: Vec<&String> = match &*def {
                MusicDef::Playlist { songs, .. } => {
                    songs.iter().filter(|s| wanted(s)).map(|s| &s.file).filter(|f| infos.contains_key(*f)).collect()
                }
                _ => infos.keys().collect(),
            };
            let held = |i: &AudioInfo| match &*def {
                MusicDef::Track { .. } | MusicDef::Interactive(_) => i.frames_at(MUSIC_RATE).saturating_mul(8),
                _ => i.file_bytes,
            };
            let cost: u64 = counted.iter().map(|f| held(&infos[*f])).sum();
            let over_bytes = d.bytes + cost > BUDGET.music_bytes;
            if over_bytes || files + counted.len() > BUDGET.music_files {
                let why = if over_bytes {
                    format!(
                        "the scope's package music would hold {} bytes of files; at most {}",
                        d.bytes + cost,
                        BUDGET.music_bytes
                    )
                } else {
                    format!(
                        "the scope's package music would be {} files; at most {}",
                        files + counted.len(),
                        BUDGET.music_files
                    )
                };
                d.notices.push(format!("content: {id} from {} not used (the disc's is): {why}", m.name));
                break;
            }
            d.bytes += cost;
            files += counted.len();
            let from = Taken { id: id.clone(), package: m.name.to_string() };
            let file = |rel: &str| MusicFile {
                rel: rel.to_string(),
                info: infos[rel],
                read: music_reader(id, m.name, m.layer, rel),
            };
            match ((*def).clone(), &disc_track) {
                (MusicDef::Playlist { replace, songs }, _) => {
                    d.music.replace_disc |= replace;
                    let added = switches::added(id);
                    for (n, s) in songs.into_iter().enumerate().filter(|(_, s)| wanted(s)) {
                        let f = file(&s.file);
                        let key = added.then(|| switches::song_key(id, n + 1).ok()).flatten();
                        d.music.added.push(TakenSong { from: from.clone(), song: s, file: f, key });
                    }
                }
                (MusicDef::Song(s), None) => {
                    let f = file(&s.file);
                    let key = switches::added(id).then(|| id.clone());
                    d.music.added.push(TakenSong { from, song: s, file: f, key });
                }
                (MusicDef::Song(s), Some(DiscTrack::Licensed(sample))) => {
                    let f = file(&s.file);
                    d.music.licensed.insert(*sample, TakenSong { from, song: s, file: f, key: None });
                }
                (MusicDef::Track { audio, gain }, Some(DiscTrack::Pathfinder { project, track, .. })) => {
                    let rel = match &audio {
                        TrackAudio::Looped { file, .. } | TrackAudio::Sectioned { file, .. } => file.clone(),
                    };
                    let f = file(&rel);
                    d.music.tracks.insert((project.clone(), *track), TakenTrack { from, audio, gain, file: f });
                }
                (MusicDef::Interactive(score), Some(DiscTrack::Pathfinder { project, track: 0, .. })) => {
                    let fs = score.files().into_iter().map(|rel| (rel.to_string(), file(rel))).collect();
                    d.music.scores.insert(project.clone(), TakenScore { from, score, files: fs });
                }
                (MusicDef::Interactive(_), Some(DiscTrack::Pathfinder { project, .. })) => d.notices.push(format!(
                    "content: {id} from {} not used: an interactive score replaces a project's whole score, so it overrides the project's track 0 (ssx:music/pathfinder/{project}/0)",
                    m.name
                )),
                _ => d.notices.push(format!("content: {id} from {}: its type does not fit the id", m.name)),
            }
            break;
        }
    }
    d
}
