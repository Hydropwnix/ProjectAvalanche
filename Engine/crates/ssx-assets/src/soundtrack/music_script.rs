use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use ssx_lua::game::objectives::binder::load_batch;
use ssx_lua::game::objectives::{
    Do, Effect, GsType, GsValue, Name, NoWorld, Node, Runner, SetId, World as ObjectiveWorld, game_state_name,
    game_state_type,
};

use crate::music::ssxdd_event;
use crate::pathfinder::Pathfinder;

pub mod cmd {
    pub const PLAY_EVENT: i32 = 1;
    pub const CONTROL_LEVEL: i32 = 2;
    pub const VOLUME_LEVEL: i32 = 3;
    pub const SET_ALLOW_REPEAT_EVENTS: i32 = 4;
    pub const SET_STOP_FRAME_COUNT: i32 = 5;
    pub const SET_START_EVENT: i32 = 6;
    pub const SET_STOP_EVENT: i32 = 7;
    pub const SET_SUCCESS_EVENT: i32 = 8;
    pub const SET_FAIL_EVENT: i32 = 9;

    pub fn by_name(name: &str) -> Option<i32> {
        Some(match name {
            "None" => 0,
            "PlayEvent" => PLAY_EVENT,
            "ControlLevel" => CONTROL_LEVEL,
            "VolumeLevel" => VOLUME_LEVEL,
            "SetAllowRepeatEvents" => SET_ALLOW_REPEAT_EVENTS,
            "SetStopFrameCount" => SET_STOP_FRAME_COUNT,
            "SetStartEvent" => SET_START_EVENT,
            "SetStopEvent" => SET_STOP_EVENT,
            "SetSuccessEvent" => SET_SUCCESS_EVENT,
            "SetFailEvent" => SET_FAIL_EVENT,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct GameMusicState {
    pub nis_active: bool,
    pub descent: f32,
    pub speed: f32,
    pub surface_rock: f32,
    pub awesome: bool,
    pub super_awesome: bool,
    pub rewind_backwards: bool,
    pub rewind_count: i32,
    pub wingsuit: bool,
    pub in_air: bool,
    pub grinding: bool,
    pub ticks_to_landing: Option<f32>,
    pub track_finished: Option<bool>,
    pub armour: Option<f32>,
    pub oxygen_tank: Option<f32>,
    pub vignette_oxygen: Option<f32>,
    pub temperature: Option<f32>,
    pub temperature_max: Option<f32>,
    pub race_started: bool,
    pub result: u8,
}

impl GameMusicState {
    pub fn value(&self, name: &str) -> Option<f64> {
        let b = |x: bool| Some(if x { 1.0 } else { 0.0 });
        match name {
            "BLN_NisActive" => b(self.nis_active),
            "FLT_Descent" => Some(f64::from(self.descent)),
            "FLT_Speed" => Some(f64::from(self.speed)),
            "FLT_SurfaceWeightRock" => Some(f64::from(self.surface_rock)),
            "BLN_IsAwesome" => b(self.awesome),
            "BLN_IsSuperAwesome" => b(self.super_awesome),
            "BLN_BackwardsPortionOfRewind" => b(self.rewind_backwards),
            "INT_RewindCountCurrent" => Some(f64::from(self.rewind_count)),
            "BLN_InAir" => b(self.in_air),
            "BLN_State_Wingsuit" => b(self.wingsuit),
            "BLN_IsGrinding" => b(self.grinding),
            "FLT_NumTicksToLanding" => self.ticks_to_landing.map(f64::from),
            "BLN_IsTrackFinished" => self.track_finished.and_then(b),
            "FLT_Armour" => self.armour.map(f64::from),
            "FLT_OxygenTankLevel" => self.oxygen_tank.map(f64::from),
            "FLT_VignetteOxygen" => self.vignette_oxygen.map(f64::from),
            "FLT_InternalTemperature" => self.temperature.map(f64::from),
            "FLT_InternalTemperatureMax" => self.temperature_max.map(f64::from),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ObjectiveScript {
    pub name: String,
    pub source: Arc<[u8]>,
    pub objectives: Vec<String>,
    pub path_events: Vec<(i32, i32)>,
}

fn path_events(n: &Node, out: &mut Vec<(i32, i32)>) {
    match n {
        Node::Do(Do::Effect(Effect::AudioPathEvent { command, value })) => out.push((*command, *value)),
        Node::And { a, b, .. } | Node::Or { a, b, .. } => {
            path_events(a, out);
            b.iter().for_each(|b| path_events(b, out));
        }
        Node::Not(Some(c)) => path_events(c, out),
        _ => {}
    }
}

impl ObjectiveScript {
    pub fn parse(name: &str, src: &str) -> Self {
        let source: Arc<[u8]> = Arc::from(src.as_bytes());
        let (runner, _, _) =
            load_batch(Runner::default(), SetId::Level, &[(name.to_string(), source.to_vec())], NoWorld, None);
        let objectives = runner.sets[1].iter().map(|o| o.name.text.to_string()).collect();
        let mut events = Vec::new();
        for o in &runner.sets[1] {
            o.nodes.iter().for_each(|n| path_events(n, &mut events));
        }
        Self { name: name.to_string(), source, objectives, path_events: events }
    }

    pub fn load(big_root: &Path, name: &str) -> Result<Self> {
        let rel = format!("data/common/scripts/{name}.lua");
        let path = crate::music::find_file(big_root, &rel).with_context(|| format!("{rel} not found"))?;
        let src = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Self::parse(name, &src))
    }

    pub fn song(&self) -> Option<u8> {
        self.path_events.iter().find_map(|&(c, v)| {
            (c == cmd::PLAY_EVENT)
                .then(|| (1..=10u8).find(|k| ssxdd_event(&format!("DD_ID_{k:02}")) == Some(v as u32)))
                .flatten()
        })
    }
}

#[derive(Clone, Debug)]
pub struct MusicModule {
    pub allow_repeat: bool,
    pub stop_frames: i32,
    pub start_event: u32,
    pub stop_event: u32,
    pub success_event: u32,
    pub fail_event: u32,
    last_event: u32,
    countdown: i32,
    prev_started: bool,
    prev_result: u8,
}

impl Default for MusicModule {
    fn default() -> Self {
        Self {
            allow_repeat: true,
            stop_frames: 0,
            start_event: 0,
            stop_event: 0,
            success_event: 0,
            fail_event: 0,
            last_event: 0,
            countdown: 0,
            prev_started: false,
            prev_result: 0,
        }
    }
}

impl MusicModule {
    pub fn command(&mut self, rt: &mut Pathfinder, c: i32, v: i32) {
        match c {
            cmd::PLAY_EVENT => self.play(rt, v as u32),
            cmd::CONTROL_LEVEL => rt.set_control_level(v as u32),
            cmd::VOLUME_LEVEL => rt.set_master_volume(v),
            cmd::SET_ALLOW_REPEAT_EVENTS => self.allow_repeat = v != 0,
            cmd::SET_STOP_FRAME_COUNT => self.stop_frames = v,
            cmd::SET_START_EVENT => self.start_event = v as u32,
            cmd::SET_STOP_EVENT => self.stop_event = v as u32,
            cmd::SET_SUCCESS_EVENT => self.success_event = v as u32,
            cmd::SET_FAIL_EVENT => self.fail_event = v as u32,
            _ => {}
        }
    }

    pub fn play(&mut self, rt: &mut Pathfinder, event: u32) {
        if event != self.last_event || self.allow_repeat {
            rt.post(event);
        } else {
            let name = rt.event_name(event);
            rt.logf(format!("repeat of {name} dropped"));
        }
        self.last_event = event;
    }

    pub fn game_state(&mut self, rt: &mut Pathfinder, gs: &GameMusicState) {
        if self.prev_result == 0 {
            if gs.result == 1 && self.success_event != 0 {
                self.play(rt, self.success_event);
            }
            if matches!(gs.result, 2 | 3) && self.fail_event != 0 {
                self.play(rt, self.fail_event);
            }
        }
        if !self.prev_started && gs.race_started && self.start_event != 0 {
            self.play(rt, self.start_event);
        }
        self.prev_started = gs.race_started;
        self.prev_result = gs.result;
    }

    pub fn start(&mut self, rt: &mut Pathfinder) {
        if self.start_event != 0 {
            self.play(rt, self.start_event);
        }
        self.countdown = 0;
    }

    pub fn stop(&mut self, rt: &mut Pathfinder) {
        if self.stop_event != 0 {
            self.play(rt, self.stop_event);
        }
        if self.stop_frames > 0 {
            self.countdown = self.stop_frames;
        }
    }

    pub fn frame(&mut self, rt: &mut Pathfinder) {
        if self.countdown > 0 {
            self.countdown -= 1;
            if self.countdown == 0 {
                rt.stop_all();
                self.countdown = -1;
            }
        }
    }
}

struct MusicWorld<'a> {
    gs: &'a GameMusicState,
    commands: &'a mut dyn FnMut(i32, i32),
}

impl ObjectiveWorld for MusicWorld<'_> {
    fn game_state(&mut self, entity: &Name, id: u16) -> Option<GsValue> {
        if !entity.is("Player1") {
            return None;
        }
        let v = self.gs.value(game_state_name(id))?;
        Some(match game_state_type(i32::from(id))? {
            GsType::Bool => GsValue::Bool(v != 0.0),
            GsType::Float => GsValue::Float(v as f32),
            GsType::Int => GsValue::Int(v as i32),
            GsType::Vector | GsType::Quat => return None,
        })
    }
    fn effect(&mut self, _objective: &Name, e: &Effect) {
        if let Effect::AudioPathEvent { command, value } = e {
            (self.commands)(*command, *value);
        }
    }
}

// TODO(guess): the music runs its Deadly Descent script in a runner of its own over [`GameMusicState`] — the game runs it once in the level's batch (`ssx_physics::objectives`), and the script reads only its own objectives and flags, so the commands are the same; settle: hand the session's `DO_AudioPathEvent` effects to the module instead.
#[derive(Clone, Debug)]
pub struct ScriptRunner {
    pub script: ObjectiveScript,
    pub objectives: Runner,
    pub missing: Vec<String>,
}

impl ScriptRunner {
    pub fn new(script: ObjectiveScript) -> Self {
        let runner = Runner { level_active: true, ..Runner::default() };
        let batch = [(format!("{}.lua", script.name), script.source.to_vec())];
        let (objectives, _, _) = load_batch(runner, SetId::Level, &batch, NoWorld, None);
        Self { script, objectives, missing: Vec::new() }
    }

    pub fn update_with(&mut self, dt: f32, gs: &GameMusicState, commands: &mut dyn FnMut(i32, i32)) {
        self.objectives.update(dt, &mut MusicWorld { gs, commands });
        self.missing = self.objectives.unprovided.iter().cloned().collect();
    }

    pub fn update(&mut self, dt: f32, gs: &GameMusicState, module: &mut MusicModule, rt: &mut Pathfinder) {
        self.update_with(dt, gs, &mut |c, v| {
            rt.logf(format!("script: DO_AudioPathEvent({c}, {v:#x})"));
            module.command(rt, c, v);
        });
    }
}

pub const MODULE_FRAMES_PER_STEP: usize = 2;

pub struct DdMusic {
    pub music: crate::music::PathfinderMusic,
    pub runner: ScriptRunner,
    pub module: MusicModule,
}

impl DdMusic {
    pub fn load(big_root: &Path, script: ObjectiveScript, chapter: Option<u32>) -> Result<Self> {
        let bank = match chapter {
            Some(c) => crate::music::bank_for_chapter(c),
            None => {
                let probe = crate::music::PathfinderMusic::load(big_root, "ssxdd", &[])?;
                script.song().and_then(|s| crate::music::PathfinderMusic::bank_for_part(&probe.rt.mpf, s))
            }
        };
        let music = crate::music::PathfinderMusic::load(big_root, "ssxdd", bank.as_slice())?;
        Ok(Self { music, runner: ScriptRunner::new(script), module: MusicModule::default() })
    }

    pub fn update(&mut self, dt: f32, gs: &GameMusicState) {
        let rt = &mut self.music.rt;
        self.runner.update(dt, gs, &mut self.module, rt);
        self.module.game_state(rt, gs);
        for _ in 0..MODULE_FRAMES_PER_STEP {
            self.module.frame(rt);
        }
    }
}

pub fn track_drop_scripts(big_root: &Path, track: &str) -> Result<Vec<(u32, String, Vec<String>)>> {
    use crate::vault_db::{fnv1, single};
    use ssx_formats::attrib::{Schema, Value, Vault, collection};
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let read = |stem: &str| -> Result<(Vec<u8>, Vec<u8>)> {
        let p = |ext: &str| dir.join(format!("{stem}.{ext}"));
        Ok((
            std::fs::read(p("vlt")).with_context(|| format!("reading {}", p("vlt").display()))?,
            std::fs::read(p("bin"))?,
        ))
    };
    let (sv, sb) = read("attribdb_schema")?;
    let schema = Schema::from_vault(&Vault::parse(&sv, &sb)?).context("reading the schema")?;
    let (gv, gb) = read("ge_attribdb")?;
    let gvault = Vault::parse(&gv, &gb)?;
    let gcols = collection::collections(&gvault)?;
    let want = fnv1(&format!("{track}_track"));
    let mut drops = Vec::new();
    for c in gcols.iter().filter(|c| c.class == fnv1("ge_drop")) {
        let d = c.decode(&gvault, &schema)?;
        let on_track = d
            .fields
            .iter()
            .any(|f| f.key == fnv1("Track") && matches!(single(&f.data), Some(Value::Ref(r)) if r.collection == want));
        if on_track {
            drops.push(c.key);
        }
    }
    let (v, b) = read("attribdb")?;
    let mut names = ssx_formats::attrib::NameDict::new();
    names.add_strings_from(&v);
    names.add_strings_from(&b);
    let vault = Vault::parse(&v, &b)?;
    let cols = collection::collections(&vault)?;
    let mut decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
    crate::content::data::events::route_ui_drops(big_root, &cols, &mut decoded);
    let index = collection::index(&cols);
    let mut out = Vec::new();
    for (i, c) in cols.iter().enumerate().filter(|(_, c)| c.class == fnv1("ui_drop")) {
        let fields = collection::with_inherited(&cols, &decoded, &index, i)?;
        let drop = fields.iter().find_map(|(_, f)| match single(&f.data) {
            Some(Value::Ref(r)) if f.key == 0xB9C2_CFC5 => Some(r.collection),
            _ => None,
        });
        let Some(drop) = drop.filter(|d| drops.contains(d)) else { continue };
        let scripts: Vec<String> = fields
            .iter()
            .filter(|(_, f)| f.key == 0x0E24_143E)
            .flat_map(|(_, f)| match &f.data {
                collection::FieldData::Array(vs) => vs.clone(),
                collection::FieldData::Single(v) => vec![v.clone()],
            })
            .filter_map(
                |v| if let Value::Text(Some(t)) = v { Some(t.trim_end_matches(".lua").to_string()) } else { None },
            )
            .collect();
        let name = names.get(c.key).map_or_else(|| format!("{:#010x}", c.key), str::to_string);
        out.push((drop, name, scripts));
    }
    Ok(out)
}

pub fn dd_script_name(drops: &[(u32, String, Vec<String>)], ge_drop: Option<u32>) -> Option<(String, Option<u32>)> {
    let dd = |d: &(u32, String, Vec<String>)| {
        let script = d.2.iter().find(|s| s.starts_with("eventtype_dd_"))?.clone();
        let chapter = d.1.strip_prefix("chapter_").and_then(|r| r.split('_').next()).and_then(|n| n.parse().ok());
        Some((script, chapter))
    };
    match ge_drop {
        Some(k) => drops.iter().filter(|d| d.0 == k).find_map(dd),
        None => drops.iter().find_map(dd),
    }
}

pub fn demo_timeline(t: f32) -> GameMusicState {
    let mut gs =
        GameMusicState { nis_active: t < 5.0, descent: (t - 5.0).max(0.0) * 15.0, speed: 20.0, ..Default::default() };
    gs.surface_rock = if (12.0..17.0).contains(&t) { 0.5 } else { 0.05 };
    gs.awesome = (20.0..24.0).contains(&t) || (36.0..40.0).contains(&t);
    gs.super_awesome = (36.0..40.0).contains(&t);
    gs.rewind_backwards = (28.0..30.0).contains(&t);
    gs.rewind_count = if t < 30.0 { 2 } else { 1 };
    gs.in_air = (t % 7.0) < 2.0;
    gs.track_finished = Some(false);
    gs.armour = Some(1.0);
    gs.result = if t >= 50.0 { 2 } else { 0 };
    gs
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
BeginObjective("Interactive Music Intro")
	WAIT_GameStateValue("Player1", BLN_NisActive, EqualTo, false)
	DO_AudioPathEvent(PlayEvent, PATH_EVENT_SSXDD_GLOBAL_DD_ID_01)
EndObjective()
BeginObjective("Interactive Music Recovery Tricky")
	WAIT_ObjectiveComplete("Interactive Music Intro")
	WAIT_CheckNamedFlag("path_event_supertricky")
	NOT()
	WAIT_CheckNamedFlag("path_event_supertricky")
	AND()
	NOT()
	WAIT_CheckNamedFlag("path_event_rewind")
	WAIT_GameStateValue("Player1", BLN_IsTrackFinished, EqualTo, false)
	DO_AudioPathEvent(PlayEvent, PATH_EVENT_SSXDD_GLOBAL_Music_Event_B)
	DO_RestartObjective()
EndObjective()
BeginObjective("Temperature")
	WAIT_WorldEntityValue( FloatExpressionValue( FloatGameStateValue("Player1", FLT_InternalTemperature),
		DividedBy, FloatGameStateValue("Player1", FLT_InternalTemperatureMax) ), LessThan, 0.8 )
	DO_AudioPathEvent(ControlLevel, 50)
EndObjective()
"#;

    #[test]
    fn the_script_runs_in_the_runner() {
        let script = ObjectiveScript::parse("t", SRC);
        assert_eq!(script.objectives.len(), 3);
        assert_eq!(script.song(), Some(1));
        let mut r = ScriptRunner::new(script);
        let mut gs = GameMusicState { nis_active: true, track_finished: Some(false), ..Default::default() };
        let mut got = Vec::new();
        r.update_with(0.1, &gs, &mut |c, v| got.push((c, v)));
        assert!(got.is_empty(), "the intro waits for the NIS's end");
        gs.nis_active = false;
        r.update_with(0.1, &gs, &mut |c, v| got.push((c, v)));
        assert_eq!(got, [(cmd::PLAY_EVENT, ssxdd_event("DD_ID_01").expect("event") as i32)]);
        assert!(r.missing.iter().any(|m| m.contains("FLT_InternalTemperature")), "{:?}", r.missing);
        gs.temperature = Some(0.7);
        gs.temperature_max = Some(0.9);
        r.update_with(0.1, &gs, &mut |c, v| got.push((c, v)));
        assert_eq!(got.last(), Some(&(cmd::CONTROL_LEVEL, 50)), "0.7 / 0.9 < 0.8");
    }
}
