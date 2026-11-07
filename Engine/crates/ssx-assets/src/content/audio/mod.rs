pub mod apply;
pub mod convert;
pub mod decide;
pub mod disc;
pub mod stream;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use ssx_content::{ContentId, Kind, Layer, Registry};

pub use decide::{
    AudioBudget, AudioScope, BUDGET, MusicDecision, MusicFile, MusicPlan, SoundDecision, TakenScore, TakenSong,
    TakenTrack,
};
pub use disc::{DiscMusic, DiscSounds, DiscTrack};

use super::GameContent;

type SoundKey = (Option<String>, Option<ssx_content::ContentId>, u64);

#[derive(Default)]
pub struct AudioCache {
    sound_decisions: Mutex<BTreeMap<SoundKey, Arc<SoundDecision>>>,
    music_decisions: Mutex<BTreeMap<(AudioScope, u64), Arc<MusicDecision>>>,
    sounds: OnceLock<Option<Arc<DiscSounds>>>,
    music: OnceLock<Option<Arc<DiscMusic>>>,
    mounted: Mutex<(bool, bool)>,
}

pub(super) fn provides_audio(registry: &Registry) -> (bool, bool) {
    (super::store::provides_kind(registry, Kind::Sound), super::store::provides_kind(registry, Kind::Music))
}

impl GameContent {
    pub fn disc_sounds(&self) -> Option<Arc<DiscSounds>> {
        self.audio
            .sounds
            .get_or_init(|| match DiscSounds::index(self.big_root()) {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    self.notice(format!("content: the disc's sounds: {e:#}"));
                    None
                }
            })
            .clone()
    }

    pub fn disc_music(&self) -> Option<Arc<DiscMusic>> {
        self.audio
            .music
            .get_or_init(|| match DiscMusic::index(self.big_root()) {
                Ok(s) => Some(Arc::new(s)),
                Err(e) => {
                    self.notice(format!("content: the disc's music: {e:#}"));
                    None
                }
            })
            .clone()
    }

    pub(super) fn mount_audio(&self, registry: &mut Registry) -> bool {
        let (sound, music) = provides_audio(registry);
        let mut mounted = self.audio.mounted.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut any = false;
        if sound
            && !mounted.0
            && let Some(s) = self.disc_sounds()
        {
            registry.mount(Layer::Base, 0, Box::new(super::store::Shared(s)));
            mounted.0 = true;
            any = true;
        }
        if music
            && !mounted.1
            && let Some(m) = self.disc_music()
        {
            registry.mount(Layer::Base, 0, Box::new(super::store::Shared(m)));
            mounted.1 = true;
            any = true;
        }
        any
    }

    pub fn audio_overlay(&self) -> Vec<String> {
        let generation = self.generation();
        let mut out = Vec::new();
        let sounds = self.audio.sound_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for ((lang, rider, _), d) in sounds.iter().filter(|((_, _, g), _)| *g == generation) {
            if d.sounds.is_empty() && d.notices.is_empty() {
                continue;
            }
            let lang = match rider {
                Some(r) => format!("{}, {r}'s lines", lang.as_deref().unwrap_or("front end")),
                None => lang.as_deref().unwrap_or("front end").to_string(),
            };
            out.push(format!(
                "sounds ({lang}): {} effects, {} speech events replaced; {} on the disc's (see the log)",
                d.sounds.effects.len(),
                d.sounds.speech.len(),
                d.notices.len()
            ));
        }
        drop(sounds);
        let music = self.audio.music_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for ((scope, _), d) in music.iter().filter(|((_, g), _)| *g == generation) {
            if d.music.is_empty() && d.notices.is_empty() {
                continue;
            }
            let m = &d.music;
            out.push(format!(
                "music ({scope:?}): {} songs added, {} licensed, {} tracks, {} scores; {} on the disc's (see the log)",
                m.added.len(),
                m.licensed.len(),
                m.tracks.len(),
                m.scores.len(),
                d.notices.len()
            ));
        }
        out
    }

    pub fn read_audio_from(&self, id: &ContentId, package: &str, layer: Layer, rel: &str) -> Result<Arc<[u8]>, String> {
        let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (_, src) = registry
            .providers(id)
            .find(|(m, _)| m.name == package && m.layer == layer)
            .ok_or_else(|| format!("{id}: {package} is no longer mounted"))?;
        match src.audio_file(id, rel) {
            Some(r) => r.map_err(|e| e.to_string()),
            None => Err(format!("{id}: {package} holds no audio files")),
        }
    }

    pub fn sound_overrides(&self, lang: Option<&str>) -> Arc<SoundDecision> {
        self.sound_overrides_for(lang, None)
    }

    pub fn sound_overrides_for(&self, lang: Option<&str>, rider: Option<&str>) -> Arc<SoundDecision> {
        let generation = self.generation();
        let (voice_owner, voice_lines) = match (lang, rider) {
            (Some(_), Some(r)) => self.voice_lines(r),
            _ => (None, Vec::new()),
        };
        let key = (lang.map(str::to_string), voice_owner, generation);
        if let Some(hit) =
            self.audio.sound_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key)
        {
            return hit.clone();
        }
        let decision = {
            let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !provides_audio(&registry).0 {
                Arc::new(SoundDecision::default())
            } else {
                let sounds = self.disc_sounds();
                let reader = |id: &ContentId, package: &str, layer: Layer, rel: &str| {
                    let (_, src) = registry
                        .providers(id)
                        .find(|(m, _)| m.name == package && m.layer == layer)
                        .ok_or_else(|| format!("{id}: {package} is not mounted"))?;
                    src.audio_file(id, rel)
                        .unwrap_or_else(|| Err(ssx_content::LoadError::Missing(id.clone())))
                        .map_err(|e| e.to_string())
                };
                Arc::new(decide::decide_sounds(&registry, lang, sounds.as_deref(), &voice_lines, &reader))
            }
        };
        if !decision.sounds.is_empty() || !decision.notices.is_empty() || decision.added > 0 {
            self.info(decision.summary(lang));
            for n in &decision.notices {
                self.notice(n.clone());
            }
        }
        let mut cache = self.audio.sound_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|(_, _, g), _| *g == generation);
        cache.insert(key, decision.clone());
        decision
    }

    pub fn music_plan(self: &Arc<Self>, scope: &AudioScope) -> Arc<MusicDecision> {
        let generation = self.generation();
        let key = (scope.clone(), generation);
        if let Some(hit) =
            self.audio.music_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&key)
        {
            return hit.clone();
        }
        let decision = {
            let registry = self.registry.read().unwrap_or_else(std::sync::PoisonError::into_inner);
            if !provides_audio(&registry).1 {
                Arc::new(MusicDecision::default())
            } else {
                let music = self.disc_music();
                let me = self.clone();
                let music_reader = |id: &ContentId, package: &str, layer: Layer, rel: &str| {
                    let (me, id, package, rel) = (me.clone(), id.clone(), package.to_string(), rel.to_string());
                    let read: decide::OpenFile = Arc::new(move || me.read_audio_from(&id, &package, layer, &rel));
                    read
                };
                Arc::new(decide::decide_music(&registry, scope, music.as_deref(), &music_reader))
            }
        };
        if !decision.music.is_empty() || !decision.notices.is_empty() {
            self.info(decision.summary(scope));
            for n in &decision.notices {
                self.notice(n.clone());
            }
        }
        let mut cache = self.audio.music_decisions.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.retain(|(_, g), _| *g == generation);
        cache.insert(key, decision.clone());
        decision
    }
}
