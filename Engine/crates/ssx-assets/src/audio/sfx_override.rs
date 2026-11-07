use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub use ssx_content::audio::sounds::{Looping, Others, PlayerRef, Route, SoundDef, Variation};

#[derive(Debug)]
pub struct Samples48 {
    pub file: String,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Samples48 {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

#[derive(Debug)]
struct Pick {
    next: usize,
    rng: u64,
}

#[derive(Debug)]
pub struct OverrideSound {
    pub id: String,
    pub package: String,
    pub set: Vec<Arc<Samples48>>,
    pub variation: Variation,
    pub volume: f32,
    pub pitch: (f32, f32),
    pub looping: Looping,
    pub route: Route,
    pub players: Option<Vec<PlayerRef>>,
    pub voices: Option<Vec<String>>,
    pub others: Others,
    pick: Mutex<Pick>,
}

#[derive(Clone, Debug)]
pub enum Choice {
    Play { sound: Arc<OverrideSound>, draw: Draw },
    Mute,
    Route(String),
}

#[derive(Clone, Debug)]
pub struct Draw {
    pub sample: Arc<Samples48>,
    pub pitch: f32,
}

impl OverrideSound {
    pub fn new(id: &str, package: &str, set: Vec<Arc<Samples48>>, def: &SoundDef) -> Self {
        let h = ssx_content::ContentHash::of(id.as_bytes()).0;
        let seed = u64::from_le_bytes([h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]]) | 1;
        Self {
            id: id.to_string(),
            package: package.to_string(),
            set,
            variation: def.variation,
            volume: def.volume,
            pitch: def.pitch,
            looping: def.looping,
            route: def.route.clone(),
            players: def.players.clone(),
            voices: def.voices.clone(),
            others: def.others,
            pick: Mutex::new(Pick { next: 0, rng: seed }),
        }
    }

    pub fn draw(&self) -> Option<Draw> {
        let mut p = self.pick.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let n = self.set.len();
        if n == 0 {
            return None;
        }
        let k = match self.variation {
            Variation::Random => (xorshift(&mut p.rng) % n as u64) as usize,
            Variation::Sequential => {
                let k = p.next % n;
                p.next = (k + 1) % n;
                k
            }
        };
        let (lo, hi) = self.pitch;
        let pitch =
            if hi > lo { lo + (hi - lo) * ((xorshift(&mut p.rng) >> 40) as f32 / (1u64 << 24) as f32) } else { lo };
        Some(Draw { sample: self.set[k].clone(), pitch })
    }

    pub fn takes(&self, bank: &str, module: usize, player: usize, voice: Option<(&str, &str)>) -> bool {
        let by_player = self.players.as_ref().is_none_or(|ps| {
            ps.iter().any(|p| {
                p.bank.eq_ignore_ascii_case(bank)
                    && p.module.is_none_or(|m| m as usize == module)
                    && p.player as usize == player
            })
        });
        let by_voice = self.voices.as_ref().is_none_or(|vs| {
            voice
                .is_some_and(|(patch, v)| vs.iter().any(|n| n.eq_ignore_ascii_case(v) || n.eq_ignore_ascii_case(patch)))
        });
        by_player && by_voice
    }
}

fn xorshift(s: &mut u64) -> u64 {
    *s ^= *s >> 12;
    *s ^= *s << 25;
    *s ^= *s >> 27;
    s.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

pub const EMITTER_BUSES: [(&str, &str); 12] = [
    ("boarder_board_center", "Board_Center_BUS"),
    ("rider_voice", "Rider_Speech_BUS"),
    ("avalanche_center", "Avalanche_Center_BUS"),
    ("avalanche_front_left", "Avalanche_Front_Left_BUS"),
    ("avalanche_front_right", "Avalanche_Front_Right_BUS"),
    ("avalanche_rear_left", "Avalanche_Rear_Left_BUS"),
    ("avalanche_rear_right", "Avalanche_Rear_Right_BUS"),
    ("helicopter_center", "Helicopter_Center_BUS"),
    ("helicopter_left", "Helicopter_Left_BUS"),
    ("helicopter_head", "Helicopter_Head_BUS"),
    ("helicopter_tail", "Helicopter_Tail_BUS"),
    ("helicopter_right", "Helicopter_Right_BUS"),
];

pub fn emitter_bus(name: &str) -> Option<&'static str> {
    EMITTER_BUSES.iter().find(|(e, _)| e.eq_ignore_ascii_case(name)).map(|(_, b)| *b)
}

#[derive(Clone, Debug, Default)]
pub struct SoundOverrides {
    pub effects: HashMap<(String, String), Arc<OverrideSound>>,
    pub speech: HashMap<(String, String), Arc<OverrideSound>>,
}

impl SoundOverrides {
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.speech.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_content::audio::sounds::{SoundClass, parse_toml};

    fn sound(text: &str, files: usize) -> OverrideSound {
        let def = parse_toml(text.as_bytes(), SoundClass::Effect).unwrap();
        let set = (0..files)
            .map(|k| Arc::new(Samples48 { file: format!("f{k}.wav"), channels: 1, samples: vec![0.0; 480] }))
            .collect();
        OverrideSound::new("ssx:sound/sfx/boardsfx/sfx_board", "p", set, &def)
    }

    #[test]
    fn the_variation_draws_in_order_or_at_random_within_the_pitch_range() {
        let s = sound("files = [\"a.wav\", \"b.wav\", \"c.wav\"]\nvariation = \"sequential\"\n", 3);
        let order: Vec<String> = (0..5).map(|_| s.draw().unwrap().sample.file.clone()).collect();
        assert_eq!(order, ["f0.wav", "f1.wav", "f2.wav", "f0.wav", "f1.wav"]);
        assert_eq!(s.draw().unwrap().pitch, 1.0, "no range: the game's pitch");
        let text = "files = [\"a.wav\", \"b.wav\", \"c.wav\"]\nvariation = \"random\"\npitch = [0.9, 1.1]\n";
        let (r1, r2) = (sound(text, 3), sound(text, 3));
        let draws: Vec<(String, f32)> =
            (0..200).map(|_| r1.draw().map(|d| (d.sample.file.clone(), d.pitch)).unwrap()).collect();
        let again: Vec<(String, f32)> =
            (0..200).map(|_| r2.draw().map(|d| (d.sample.file.clone(), d.pitch)).unwrap()).collect();
        assert_eq!(draws, again, "the same draws on every run (seeded by the id)");
        assert!(draws.iter().all(|(_, p)| (0.9..1.1).contains(p)));
        for f in ["f0.wav", "f1.wav", "f2.wav"] {
            assert!(draws.iter().filter(|(x, _)| x == f).count() > 30, "{f} drawn");
        }
        assert!(sound("files = [\"a.wav\"]\n", 0).draw().is_none(), "an empty set plays nothing");
    }

    #[test]
    fn a_set_takes_the_voices_it_names() {
        let all = sound("files = [\"a.wav\"]\n", 1);
        assert!(all.takes("board_packed", 3, 5, None), "no selector: every voice of the id");
        let by_player = sound(
            "files = [\"a.wav\"]\nplayers = [{ bank = \"board_packed\", module = 3, player = 5 }, { bank = \"board_ice\", player = 1 }]\n",
            1,
        );
        assert!(by_player.takes("BOARD_PACKED", 3, 5, None));
        assert!(!by_player.takes("board_packed", 2, 5, None), "another module");
        assert!(by_player.takes("board_ice", 7, 1, None), "any module of the bank");
        assert!(!by_player.takes("board_ice", 7, 2, None));
        let by_voice = sound("files = [\"a.wav\"]\nvoices = [\"Board_Packed_Glide_Carve_Voice\"]\n", 1);
        assert!(by_voice.takes("board_packed", 3, 5, Some(("Board_Packed_Patch", "board_packed_glide_carve_voice"))));
        assert!(!by_voice.takes("board_packed", 3, 4, Some(("Board_Packed_Patch", "Board_Packed_Debris_Voice"))));
        assert!(!by_voice.takes("board_packed", 3, 5, None), "a voice without a mixer voice is not named");
        assert_eq!(emitter_bus("Boarder_Board_Center"), Some("Board_Center_BUS"));
        assert_eq!(emitter_bus("nowhere"), None);
    }
}
