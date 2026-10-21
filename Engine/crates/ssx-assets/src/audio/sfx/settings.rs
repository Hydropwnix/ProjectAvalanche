use std::path::Path;

use anyhow::{Context, Result};

pub const SPEAKER_TYPE: usize = 0;
pub const SFX_VOLUME: usize = 1;
pub const RIDER_SPEECH_VOLUME: usize = 2;
pub const PILOT_SPEECH_VOLUME: usize = 3;
pub const MENU_SFX_VOLUME: usize = 4;
pub const MUSIC_VOLUME: usize = 5;
pub const PERSONAL_MUSIC_VOLUME: usize = 6;
pub const FE_MUSIC_PICKS: usize = 7;
pub const TRICKY_STATE: usize = 9;
pub const MUSIC_REMIX: usize = 10;
pub const ENTRIES: usize = 11;

const FIELDS: [(usize, u32, &str); 9] = [
    (SPEAKER_TYPE, 0x49E9_989D, "SpeakerType"),
    (SFX_VOLUME, 0x3886_2C5A, "InGameSfxVolume"),
    (RIDER_SPEECH_VOLUME, 0xB739_B720, "CharacterSpeechVolume"),
    (PILOT_SPEECH_VOLUME, 0x01D3_52E4, "ChopperSpeechVolume"),
    (MENU_SFX_VOLUME, 0x6531_9EB9, "MenuSfxVolume"),
    (MUSIC_VOLUME, 0x6E53_5F70, "GameMusicVolume"),
    (PERSONAL_MUSIC_VOLUME, 0x1D8E_2EF2, "PersonalMusicVolume"),
    (MUSIC_REMIX, 0x975D_22B9, "MusicRemixState"),
    (TRICKY_STATE, 0xE251_5A16, "TrickyState"),
];

pub const UI_VALUES: [(u32, usize); 9] = [
    (0x21, SFX_VOLUME),
    (0x2A, MENU_SFX_VOLUME),
    (0x23, PILOT_SPEECH_VOLUME),
    (0x26, RIDER_SPEECH_VOLUME),
    (0x27, MUSIC_VOLUME),
    (0x22, PERSONAL_MUSIC_VOLUME),
    (0x28, SPEAKER_TYPE),
    (0x24, MUSIC_REMIX),
    (0x29, TRICKY_STATE),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Range {
    pub min: i32,
    pub max: i32,
    pub default: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeRanges(pub [Range; ENTRIES]);

impl FeRanges {
    pub fn load(big_root: &Path) -> Result<Self> {
        use crate::vault_db::single;
        use ssx_formats::attrib::{Schema, Value, Vault, collection};
        let dir = big_root.join("bootcache/data/xenon/attribdb");
        let read = |stem: &str| -> Result<(Vec<u8>, Vec<u8>)> {
            let p = |ext: &str| dir.join(format!("{stem}.{ext}"));
            let v = std::fs::read(p("vlt")).with_context(|| format!("reading {}", p("vlt").display()))?;
            Ok((v, std::fs::read(p("bin"))?))
        };
        let (sv, sb) = read("attribdb_schema")?;
        let schema = Schema::from_vault(&Vault::parse(&sv, &sb)?).context("reading the schema")?;
        let (gv, gb) = read("ge_attribdb")?;
        let vault = Vault::parse(&gv, &gb)?;
        let cols = collection::collections(&vault)?;
        let c = cols
            .iter()
            .find(|c| c.class == 0x1DEE_75C5 && c.key == 0x9388_A9A8)
            .context("audio_fe_settings/0x9388A9A8 missing")?;
        let d = c.decode(&vault, &schema)?;
        let mut out = [Range::default(); ENTRIES];
        for (entry, key, name) in FIELDS {
            let f = d.fields.iter().find(|f| f.key == key).with_context(|| format!("no field {name}"))?;
            let Some(Value::Raw { bytes, .. }) = single(&f.data) else { anyhow::bail!("{name} is not a raw struct") };
            let w = |o: usize| bytes.get(o..o + 4).map(|b| i32::from_be_bytes(b.try_into().unwrap()));
            let (Some(min), Some(max), Some(default)) = (w(4), w(8), w(12)) else { anyhow::bail!("{name}: short") };
            out[entry] = Range { min, max, default };
        }
        Ok(Self(out))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioSettings(pub [i32; ENTRIES]);

impl AudioSettings {
    pub fn defaults(r: &FeRanges) -> Self {
        Self(std::array::from_fn(|i| r.0[i].default))
    }

    pub fn get(&self, entry: usize) -> i32 {
        self.0.get(entry).copied().unwrap_or(0)
    }

    pub fn set_ui(&mut self, ui: u32, value: i32) -> bool {
        match UI_VALUES.iter().find(|(id, _)| *id == ui) {
            Some(&(_, e)) => {
                self.0[e] = value;
                true
            }
            None => false,
        }
    }

    pub fn ui_values(&self) -> [(u32, i32); 9] {
        UI_VALUES.map(|(id, e)| (id, self.0[e]))
    }
}

#[derive(Clone, Debug)]
pub struct FeFaders {
    last: [i32; 5],
}

impl Default for FeFaders {
    fn default() -> Self {
        Self { last: [10; 5] }
    }
}

pub const FADERS: [(&str, usize); 5] = [
    ("FE_Setting_SFX", SFX_VOLUME),
    ("FE_Setting_MenuHUD", MENU_SFX_VOLUME),
    ("FE_Setting_Speech", PILOT_SPEECH_VOLUME),
    ("FE_Setting_RiderSpeech", RIDER_SPEECH_VOLUME),
    ("FE_Setting_Music", MUSIC_VOLUME),
];

pub const FADE_SECONDS: f32 = 0.1;

pub fn fader_gain(value: i32, max: i32) -> f32 {
    let m = (max as f64).clamp(1.0, 100.0);
    ((value as f64 / m) as f32).clamp(0.0, 1.0)
}

impl FeFaders {
    pub fn update(&mut self, s: &AudioSettings, r: &FeRanges) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        for (k, &(bus, entry)) in FADERS.iter().enumerate() {
            let v = s.get(entry);
            if v != self.last[k] {
                out.push((bus, fader_gain(v, r.0[entry].max)));
            }
            self.last[k] = v;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges() -> FeRanges {
        let mut r = [Range { min: 0, max: 10, default: 10 }; ENTRIES];
        r[MUSIC_VOLUME].max = 11;
        FeRanges(r)
    }

    #[test]
    fn faders_move_only_on_a_change() {
        let r = ranges();
        let mut s = AudioSettings::defaults(&r);
        let mut f = FeFaders::default();
        assert!(f.update(&s, &r).is_empty());
        assert!(s.set_ui(0x27, 5));
        assert_eq!(f.update(&s, &r), vec![("FE_Setting_Music", 5.0 / 11.0)]);
        assert!(f.update(&s, &r).is_empty());
        s.set_ui(0x27, 10);
        assert_eq!(f.update(&s, &r), vec![("FE_Setting_Music", 10.0 / 11.0)]);
        s.set_ui(0x2A, 0);
        assert_eq!(f.update(&s, &r), vec![("FE_Setting_MenuHUD", 0.0)]);
        assert!(!s.set_ui(0x30, 1));
    }

    #[test]
    fn gains_clamp() {
        assert_eq!(fader_gain(11, 11), 1.0);
        assert_eq!(fader_gain(12, 11), 1.0);
        assert_eq!(fader_gain(-1, 10), 0.0);
        assert_eq!(fader_gain(3, 0), 1.0);
    }
}
