use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use glam::{Quat, Vec3};

use crate::event::{EventResult, EventType};
use crate::nis::{IntroAssets, IntroChoice, IntroVariant, SceneOrigin};

pub const OUTRO_LOCATOR: &str = "nis_outro_locator";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutroScript {
    Standard,
    HeliPickup,
    HeliJump,
}

pub fn outro_of_scripts(scripts: &[String]) -> Option<OutroScript> {
    scripts.iter().find_map(|s| match Path::new(s).file_name()?.to_str()? {
        "outro_standard_celebration.lua" => Some(OutroScript::Standard),
        "outro_heli_pickup.lua" => Some(OutroScript::HeliPickup),
        "outro_heli_jump.lua" => Some(OutroScript::HeliJump),
        _ => None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    WorldTour,
    Explore,
}

pub fn game_mode(event: &str) -> GameMode {
    if event.starts_with("chapter_") { GameMode::WorldTour } else { GameMode::Explore }
}

pub fn beats(ascending: bool, a: u32, b: u32) -> bool {
    match (a, b) {
        (u32::MAX, _) => false,
        (_, u32::MAX) => true,
        _ if ascending => a < b,
        _ => a > b,
    }
}

pub fn track_end_reaction(mode: GameMode, result: &EventResult) -> Option<i64> {
    use ssx_assets::nis::{OUTRO_REACTION_LOSE, OUTRO_REACTION_NEUT, OUTRO_REACTION_WIN};
    if result.event_type == Some(EventType::SurviveToDeath) {
        return None;
    }
    let asc = result.event_type.is_some_and(EventType::ranks_ascending);
    let own = result.value;
    let (y, x, flag) = match mode {
        GameMode::WorldTour => match (result.rivals.first(), result.rivals.last()) {
            (Some(&first), Some(&last)) => (first, last, true),
            _ => (own, own, true),
        },
        // TODO(guess): with fewer than three ranked rivals `FUN_82481540` leaves stat 0x87 as it was; taken as no value (−1: the player wins); settle: the stat's default in a fresh profile (`FUN_825A8B70` table at +8, entry 0x87).
        GameMode::Explore => {
            if result.rivals.len() < 3 {
                crate::guess_hit!();
            }
            (if result.rivals.len() >= 3 { result.rivals[0] } else { u32::MAX }, u32::MAX, false)
        }
    };
    Some(if beats(asc, own, y) {
        OUTRO_REACTION_WIN
    } else if flag && beats(asc, x, own) {
        OUTRO_REACTION_LOSE
    } else {
        OUTRO_REACTION_NEUT
    })
}

pub fn outro_locator(world: &crate::rider::World) -> Option<(Vec3, Quat)> {
    locator(&world.triggers, OUTRO_LOCATOR)
}

pub fn locator(triggers: &crate::triggers::TriggerSet, name: &str) -> Option<(Vec3, Quat)> {
    let v = triggers.volumes.iter().find(|v| v.name == name && v.type_text == "locator")?;
    Some((v.centre, v.rotation))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VistaCamera {
    pub position: Vec3,
    pub rotation: Quat,
    pub fov_deg: f32,
    pub object: u32,
    pub scope: crate::triggers::TriggerScope,
}

const CLASS_GAMEOBJECT: u32 = ssx_assets::vault_db::fnv1("ge_scene_gameobject");
const FIELD_CONTROLLER: u32 = ssx_assets::vault_db::fnv1("Controller");
pub const GAMEEXPLORER_DIR: &str = "memresident/data/xenon/gameexplorer";

pub fn camera_end_fov(name: &str, script: &str) -> Option<f32> {
    crate::controllers::script_sets(name, script).kinds.iter().find_map(|k| match k {
        ssx_lua::game::controllers::Track::NisDataToGameState { fov, .. } => Some(*fov),
        _ => None,
    })
}

pub fn session_vista(session: &crate::session::Session, triggers: &crate::triggers::TriggerSet) -> Option<VistaCamera> {
    use ssx_lua::game::objectives::{GsValue, Name, game_state_name};
    let o = session.objectives.as_ref()?;
    static IDS: std::sync::OnceLock<(u32, [Option<u16>; 3])> = std::sync::OnceLock::new();
    let (cam, ids) = *IDS.get_or_init(|| {
        let id = |n: &str| (1..300u16).find(|&i| game_state_name(i) == n);
        let ids = [id("VEC_CameraVistaPosition"), id("QUAT_CameraVistaOrientation"), id("FLT_CameraVistaFOV")];
        (Name::new("TheCamera").hash, ids)
    });
    let get = |k: usize| ids[k].and_then(|i| o.overlay.get(&(cam, i)));
    let (Some(GsValue::Vector(p)), Some(GsValue::Quat(q)), Some(GsValue::Float(fov_deg))) = (get(0), get(1), get(2))
    else {
        return None;
    };
    let position = Vec3::from_array(*p);
    let e = triggers.entities.iter().rev().find(|e| e.position == position);
    Some(VistaCamera {
        position,
        rotation: Quat::from_array(*q),
        fov_deg: *fov_deg,
        object: e.map_or(0, |e| e.object),
        scope: e.map_or(crate::triggers::TriggerScope::Track, |e| e.scope),
    })
}

pub fn vista_cameras(big_root: &Path, track: &str, triggers: &crate::triggers::TriggerSet) -> Result<Vec<VistaCamera>> {
    use ssx_assets::vault_db::{refs, with_track_db};
    let mut objects: Vec<u32> =
        triggers.volumes.iter().filter(|v| v.type_text == "locator").map(|v| v.object).collect();
    objects.sort_unstable();
    objects.dedup();
    let scripts: Vec<(u32, String)> = with_track_db(big_root, track, |db| {
        let mut out = Vec::new();
        for &o in &objects {
            let Some(f) = db.field(CLASS_GAMEOBJECT, o, FIELD_CONTROLLER)? else { continue };
            let Some(r) = refs(&f.data).first().copied().filter(|r| r.collection != 0) else { continue };
            if let Some(t) = db.text_of(r.class, r.collection)? {
                out.push((o, t));
            }
        }
        Ok(out)
    })?;
    let mut fovs = std::collections::HashMap::new();
    for (o, path) in scripts.iter().filter(|(_, p)| p.contains("nis_cameraend_")) {
        let file = big_root.join(GAMEEXPLORER_DIR).join(path.replace('\\', "/"));
        let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
        fovs.insert(*o, camera_end_fov(path, &text).with_context(|| format!("no NISDataToGameStateTrack in {path}"))?);
    }
    Ok(triggers
        .volumes
        .iter()
        .filter(|v| v.type_text == "locator")
        .filter_map(|v| {
            let fov_deg = *fovs.get(&v.object)?;
            Some(VistaCamera { position: v.centre, rotation: v.rotation, fov_deg, object: v.object, scope: v.scope })
        })
        .collect())
}

pub fn prepare(big_root: &Path, event: &str, character: &str, reaction: i64) -> Result<Arc<IntroAssets>> {
    let nis = ssx_assets::nis::load_outro(big_root, character, reaction)?;
    anyhow::ensure!(nis.branch_out.is_some(), "the outro's scene 1 has no NIS_Out window");
    let choice = IntroChoice {
        script: "outro_standard_celebration.lua".into(),
        binding: nis.binding.clone(),
        origin: SceneOrigin::LevelEnd,
        locator_alternate: None,
        variant: IntroVariant::Outro,
    };
    Ok(Arc::new(IntroAssets {
        choice,
        nis,
        jump_clip: usize::MAX,
        jump_clips: Vec::new(),
        event: event.to_string(),
        heli: Err("the standard outro binds no helicopter".into()),
    }))
}

pub fn start(world: &crate::rider::World, assets: Arc<IntroAssets>) -> Result<crate::rider::RiderState> {
    let (p, q) = outro_locator(world).context("the drop has no nis_outro_locator")?;
    crate::nis::level_start(world, p, q, assets).context("no ground below the outro locator")
}

pub fn prepare_heli_pickup(big_root: &Path, event: &str, character: &str) -> Result<Arc<IntroAssets>> {
    let heli = ssx_assets::helicopter::drop_helicopter(big_root, event).map_err(|e| format!("{e:#}"));
    prepare_heli_pickup_with(big_root, heli, event, character)
}

pub fn prepare_heli_pickup_with(
    big_root: &Path,
    heli: Result<ssx_assets::helicopter::HeliChoice, String>,
    event: &str,
    character: &str,
) -> Result<Arc<IntroAssets>> {
    let nis = ssx_assets::nis::load_heli_pickup_outro(big_root, character)?;
    let choice = IntroChoice {
        script: "outro_heli_pickup.lua".into(),
        binding: nis.binding.clone(),
        origin: SceneOrigin::LevelEnd,
        locator_alternate: None,
        variant: IntroVariant::Outro,
    };
    Ok(Arc::new(IntroAssets {
        choice,
        nis,
        jump_clip: usize::MAX,
        jump_clips: Vec::new(),
        event: event.to_string(),
        heli,
    }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenFade {
    pub level: f32,
    delay: f32,
    duration: f32,
    remaining: f32,
    direction: u32,
}

impl Default for ScreenFade {
    fn default() -> Self {
        Self { level: 0.0, delay: 0.0, duration: 0.0, remaining: 0.0, direction: 2 }
    }
}

impl ScreenFade {
    pub fn start(&mut self, out: bool, seconds: f32, delay: f32) {
        let direction = u32::from(out);
        self.remaining = if self.remaining > 0.0 {
            let left = self.remaining / self.duration;
            if direction == self.direction { left * seconds } else { (1.0 - left) * seconds }
        } else {
            seconds
        };
        (self.duration, self.delay, self.direction) = (seconds, delay, direction);
        self.step(0.0);
    }

    pub fn step(&mut self, dt_s: f32) {
        if self.delay > 0.0 {
            self.delay -= dt_s;
        } else if self.remaining > 0.0 {
            self.remaining = (self.remaining - dt_s).max(0.0);
            let done = (self.duration - self.remaining) / self.duration;
            match self.direction {
                0 => self.level = 1.0 - done,
                1 => self.level = done,
                _ => {}
            }
        } else if self.direction == 1 {
            self.level = 1.0;
        } else {
            (self.level, self.direction) = (0.0, 2);
        }
    }

    pub fn update(&mut self, effects: &[(String, crate::objectives::Effect)], dt_s: f32) {
        self.step(dt_s);
        for (_, e) in effects {
            if let crate::objectives::Effect::Fade { fade_out, seconds, delay } = e {
                self.start(*fade_out, *seconds, *delay);
            }
        }
    }
}

pub fn prepare_heli_grab(
    big_root: &Path,
    event: &str,
    character: &str,
    reaction: Option<i64>,
) -> Result<Arc<IntroAssets>> {
    let nis = ssx_assets::nis::load_heli_grab_outro(big_root, character, reaction.unwrap_or(-1))?;
    let choice = IntroChoice {
        script: "outro_heli_jump.lua".into(),
        binding: nis.binding.clone(),
        origin: SceneOrigin::Locator,
        locator_alternate: None,
        variant: IntroVariant::Outro,
    };
    let heli = ssx_assets::helicopter::drop_helicopter(big_root, event).map_err(|e| format!("{e:#}"));
    Ok(Arc::new(IntroAssets {
        choice,
        nis,
        jump_clip: usize::MAX,
        jump_clips: Vec::new(),
        event: event.to_string(),
        heli,
    }))
}

pub fn prepare_sequence(
    big_root: &Path,
    event: &str,
    character: &str,
    script: &str,
    binding: &str,
) -> Result<Arc<IntroAssets>> {
    let nis = ssx_assets::nis::load_sequence_nis(big_root, binding, character)?;
    let choice = IntroChoice {
        script: script.into(),
        binding: nis.binding.clone(),
        origin: SceneOrigin::LevelStart,
        locator_alternate: None,
        variant: IntroVariant::Outro,
    };
    Ok(Arc::new(IntroAssets {
        choice,
        nis,
        jump_clip: usize::MAX,
        jump_clips: Vec::new(),
        event: event.to_string(),
        heli: Err(format!("{binding} binds no helicopter")),
    }))
}

pub fn start_sequence(
    world: &crate::rider::World,
    s: &crate::rider::RiderState,
    assets: Arc<IntroAssets>,
    at: Vec3,
    q: Quat,
    ground: Vec3,
) -> Result<crate::rider::RiderState> {
    let mut n = crate::nis::level_start_on(world, ground, at, q, assets)
        .with_context(|| format!("no ground state at {ground} for the NIS at {at}"))?;
    carry_bound_rider(&mut n, s);
    Ok(n)
}

pub fn carry_bound_rider(n: &mut crate::rider::RiderState, s: &crate::rider::RiderState) {
    n.carry_game_state(s);
    (n.score, n.goofy) = (s.score, s.goofy);
    (n.track_started, n.trigger_go, n.force_race_end_timer) = (s.track_started, s.trigger_go, s.force_race_end_timer);
    (n.allow_race_end, n.level_start) = (s.allow_race_end, s.level_start);
    n.wingsuit = s.wingsuit.clone();
}

pub fn start_at(
    world: &crate::rider::World,
    assets: Arc<IntroAssets>,
    name: &str,
    ground: Vec3,
) -> Result<crate::rider::RiderState> {
    let (p, q) = locator(&world.triggers, name).with_context(|| format!("the drop has no {name}"))?;
    crate::nis::level_start(world, p, q, assets.clone())
        .or_else(|| crate::nis::level_start_on(world, ground, p, q, assets))
        .with_context(|| format!("no ground below {name} nor at {ground}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beats_follows_the_event_order_and_the_missing_value() {
        assert!(beats(true, 10, 20) && !beats(true, 20, 10) && !beats(true, 10, 10));
        assert!(beats(false, 20, 10) && !beats(false, 10, 20));
        assert!(!beats(true, u32::MAX, 5) && beats(true, 5, u32::MAX));
    }

    #[test]
    fn camera_end_scripts_name_their_fov() {
        let s = "local s = CreateStateSet(\"EndNISCamera\") local st = s:AddState(\"A\") \
                 currentTrack = NISDataToGameStateTrack(\"TheCamera\", VEC_CameraVistaPosition, \
                 QUAT_CameraVistaOrientation, FLT_CameraVistaFOV, 75.0)";
        assert_eq!(camera_end_fov("t.lua", s), Some(75.0));
        assert_eq!(camera_end_fov("t.lua", "local x = 1"), None);
    }

    #[test]
    fn the_fader_follows_the_game() {
        use crate::objectives::Effect;
        let fade =
            |out: bool, seconds: f32, delay: f32| vec![(String::new(), Effect::Fade { fade_out: out, seconds, delay })];
        let mut f = ScreenFade::default();
        f.update(&fade(true, 0.2, 0.0), 1.0 / 30.0);
        assert_eq!(f.level, 0.0);
        f.update(&[], 0.1);
        assert!((f.level - 0.5).abs() < 1e-6);
        f.update(&fade(false, 1.0, 0.0), 0.0);
        assert!((f.level - 0.5).abs() < 1e-6);
        f.update(&[], 0.25);
        assert!((f.level - 0.25).abs() < 1e-6);
        f.update(&[], 1.0);
        assert_eq!(f.level, 0.0);
        f.update(&fade(false, 1.0, 0.5), 0.1);
        f.update(&[], 0.5);
        assert_eq!(f.level, 0.0);
        f.update(&[], 0.25);
        assert!((f.level - 0.75).abs() < 1e-6);
        f.start(true, 0.0, 0.0);
        assert_eq!(f.level, 1.0);
    }
}
