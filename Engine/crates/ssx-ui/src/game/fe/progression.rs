use ssx_assets::career::DropType;
use ssx_assets::career::globe::focus::{self, Plan, Step};
use ssx_assets::career::profile::DropState;
use ssx_formats::attrib::fnv1;

use super::{FeEnv, FrontEnd, MovieRequest};

pub const ANIM_COMPLETE: u16 = 0xBF;
pub const ANIM_KEY: u16 = 0xC0;
pub const ANIM_TYPE: u16 = 0xC1;
pub const PENDING_NAVIGATION: u16 = 0x1B5;
pub const NO_SCREEN: u32 = 0x57;
pub const PREVIOUS: u32 = 0x58;
pub const QUEUE_SECONDS: f32 = 30.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Progression {
    pub steps: Vec<Step>,
    pub index: i32,
    pub clock: f32,
    pub held: u32,
}

impl Default for Progression {
    fn default() -> Self {
        Progression { steps: Vec::new(), index: -1, clock: 0.0, held: 0 }
    }
}

impl Progression {
    pub fn start(&mut self, plan: Plan) {
        *self = Progression { steps: plan.steps, index: 0, clock: 0.0, held: plan.held };
    }

    pub fn running(&self) -> bool {
        self.index >= 0
    }
}

pub fn take_calls(fe: &mut FrontEnd) {
    for call in std::mem::take(&mut fe.profile.globe_progressions) {
        let node = |d: u32| fe.career.event(d).map_or((0, 0), |e| fe.career.peak_and_range(e));
        let dd = fe.career.event(call.from).is_some_and(|e| e.deadly_descent);
        if let Some(plan) = focus::plan(call.from, &call.to, node, dd) {
            fe.note(format!("progression from {:#010x} to {:x?}: {} steps", call.from, call.to, plan.steps.len()));
            fe.progression.start(plan);
        }
    }
}

pub fn globe_dispatched(fe: &mut FrontEnd) {
    let p = &mut fe.progression;
    if p.index >= 0 && p.steps.get(p.index as usize).is_some_and(|s| s.kind == 9) {
        p.index += 1;
    }
}

pub fn go_to_node(fe: &mut FrontEnd, node: u32, level: u32) {
    fe.note(format!("FEGlobeGoToNode {{{node:#010x}, {level}}}"));
    match level {
        0 => fe.globe.range = node,
        1 => {
            fe.globe.peak = node;
            if let Some(p) = fe.career.peak(node) {
                fe.globe.range = p.range;
            }
        }
        _ => {
            let (peak, range) = fe.career.event(node).map_or((0, 0), |e| fe.career.peak_and_range(e));
            fe.globe.drop = node;
            fe.globe.peak = peak;
            fe.globe.range = range;
        }
    }
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv, dt: f32) {
    take_calls(fe);
    let movies = fe.movie.is_some() || !fe.movie_queue.is_empty();
    if fe.progression.index < 0 || movies || fe.in_drop {
        return;
    }
    let profile_movies = !fe.profile.movie_queue.is_empty();
    if !profile_movies {
        let i = fe.progression.index as usize;
        fe.progression.clock += dt;
        if let Some(step) = fe.progression.steps.get(i).copied() {
            play(fe, env, step, i, dt);
        }
    }
    let p = &mut fe.progression;
    if p.index >= 0 && (p.steps.len() as i32 <= p.index || p.clock >= QUEUE_SECONDS) {
        p.clock = 0.0;
        p.index = -1;
        env.values.set_int(ANIM_KEY, 0);
        env.values.set_int(ANIM_TYPE, 0);
        env.values.set_bool(ANIM_COMPLETE, false);
    }
}

fn play(fe: &mut FrontEnd, env: &mut FeEnv, step: Step, i: usize, dt: f32) {
    let next = |fe: &mut FrontEnd| fe.progression.index += 1;
    match step.kind {
        0 => {
            next(fe);
            env.values.set_int(ANIM_KEY, step.node as i32);
            env.values.set_int(ANIM_TYPE, step.anim);
            env.values.set_bool(ANIM_COMPLETE, false);
            if step.anim == 2 && fe.progression.held != 0 {
                fe.progression.held = 0;
            }
            fe.publish.push("dp_globe_data".into());
        }
        1 => {
            let s = &mut fe.progression.steps[i];
            s.seconds -= dt;
            if env.values.bool(ANIM_COMPLETE) || s.seconds <= 0.0 {
                next(fe);
                env.values.set_bool(ANIM_COMPLETE, false);
            }
        }
        2 => {
            next(fe);
            fe.note(format!("progression: message 0x14649424 {{{:#010x}}}", step.node));
            go_to_node(fe, step.node, 2);
            if let Some(s) = fe.globe_scene.as_mut() {
                s.select_drop(step.node);
            }
        }
        3 | 4 | 6 | 7 => {
            next(fe);
            let (level, screen) = match step.kind {
                4 => (0, 0x42),
                7 => (2, 0x44),
                _ => (1, 0x43),
            };
            go_to_node(fe, step.node, level);
            fe.note(format!("FEGenericNavMsg {{{screen:#x}}}"));
            fe.go_to_screen(env, screen);
        }
        5 => {
            next(fe);
            go_to_node(fe, step.node, 0);
            if let Some(s) = fe.globe_scene.as_mut() {
                s.set_focus(step.node, false);
            }
        }
        8 => {
            let s = &mut fe.progression.steps[i];
            s.seconds -= dt;
            if s.seconds <= 0.0 {
                next(fe);
            }
        }
        10 => {
            let gate = fnv1(b"chapter_10_drop_dd");
            if fe.profile.drop_state(gate) == DropState::Completed
                && fe.last_result_drop == gate
                && fe.last_success == Some(true)
            {
                final_drop_start(fe, env);
            }
            next(fe);
        }
        _ => {}
    }
}

pub const PRE_LAUNCH_MOVIES: [(&str, &[(&str, u32)]); 10] = [
    ("rockies", &[("rockies_stinger", 0)]),
    ("patagonia", &[("patagonia_stinger", 0)]),
    ("africa", &[("kilimanjaro_stinger", 0)]),
    ("siberia", &[("siberia_stinger", 0)]),
    ("alaska", &[("alaska_stinger", 0)]),
    ("himalayas", &[("himalayas_stinger", 0)]),
    ("alps", &[("alps_stinger", 0)]),
    ("antarctica", &[("antarctica_stinger", 0)]),
    ("new_zealand", &[("newzealand_stinger", 0)]),
    ("chapter_11_drop_h2h", &[("comic_griff", 1), ("himalayas_lhotse_stinger", 0)]),
];

pub const FINAL_TRACK_MOVIE: &str = "himalayas_lhotse_stinger";

pub fn pre_launch_movies(fe: &mut FrontEnd, drop: u32) -> bool {
    let Some(e) = fe.career.event(drop).filter(|e| e.deadly_descent) else { return false };
    let range = fe.career.peak_and_range(e).1;
    let entry = PRE_LAUNCH_MOVIES
        .iter()
        .find(|(k, _)| fnv1(k.as_bytes()) == drop)
        .or_else(|| PRE_LAUNCH_MOVIES.iter().find(|(k, _)| fnv1(k.as_bytes()) == range));
    let Some((_, movies)) = entry else { return false };
    let unlocked = fe.profile.drop_state(drop) == DropState::Unlocked;
    for &(name, kind) in movies.iter() {
        if kind == 0 || unlocked {
            fe.note(format!("FLOW_PlayMovie {name} (pre-launch)"));
            fe.queue_movie(MovieRequest::new(name, "prelaunch"));
        }
    }
    true
}

pub fn final_drop_start(fe: &mut FrontEnd, _env: &mut FeEnv) {
    let drop = focus::FINAL_DROP;
    fe.current_drop = drop;
    fe.messages.push(super::FeMessage::LoadoutRefresh);
    fe.note("final drop: 0x17D251BE, its items equipped, the pre-launch movies (0x8257D1A8)");
    for &(key, slot) in &FINAL_DROP_ITEMS {
        super::equipment::flow_equip(fe, key, i64::from(slot));
    }
    pre_launch_movies(fe, drop);
}

pub const FINAL_DROP_ITEMS: [(u32, u32); 6] =
    [(0x405D_C1D5, 0), (0x55BB_536F, 0), (0x2002_1A25, 0), (0x2039_B98F, 0), (0x2636_71F3, 1), (0xEED8_4B39, 2)];

// TODO(guess): a replay of the completed final drop goes through the equipment, which plays the stinger too and launches itself (0x825FB4A0); the port lets the equipment's launch stand; settle: where `FMMovie`'s update (0x823FB648, vtable 0x82016070) runs in the FE frame 0x823D4C80.
pub fn load_to_final_track(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.note("LoadToFinalTrackMessage (0xD0061E41)");
    if fe.screen == super::screens::id::EQUIPMENT && fe.equipment.state == super::equipment::state::MOVIES {
        return;
    }
    super::flow::launch(fe, env, focus::FINAL_DROP, true);
}

pub fn quit_return(fe: &mut FrontEnd, env: &mut FeEnv) {
    if fe.ava.maps.riding.take().is_some() {
        fe.change_gear = false;
        fe.ava.maps.return_due = true;
        env.values.set_int(PENDING_NAVIGATION, NO_SCREEN as i32);
        fe.note("the quit's return: a map's ride → the Maps page");
        return;
    }
    let drop = fe.current_drop;
    fe.profile.globe_last_drop = drop;
    if std::mem::take(&mut fe.change_gear) {
        fe.note("the quit's return: FE manager +0x2E (change gear) → +0x7C(0), screen 0x30");
        env.values.set_int(PENDING_NAVIGATION, super::screens::id::EQUIPMENT as i32);
        return;
    }
    let wt = fe.career.event(drop).is_some_and(|e| e.drop_type == DropType::Campaign);
    let screen = if wt {
        let node = fe.profile.globe_last_drop;
        let (level, screen) = focus::node_screen(
            fe.career.event(node).is_some(),
            fe.career.peak(node).is_some(),
            fe.career.range(node).is_some(),
        );
        if !fe.profile.new_game_active() && level >= 0 {
            go_to_node(fe, node, level as u32);
        }
        screen
    } else {
        PREVIOUS
    };
    env.values.set_int(PENDING_NAVIGATION, screen as i32);
}

pub fn pending_navigation(fe: &mut FrontEnd, env: &mut FeEnv) {
    if std::mem::take(&mut fe.ava.maps.return_due) {
        fe.go_to_screen_by(env, super::screens::id::MAIN_MENU, super::ava::maps::OPEN);
        return;
    }
    let s = env.values.int(PENDING_NAVIGATION);
    if s != 0 && s as u32 != NO_SCREEN {
        env.values.set_int(PENDING_NAVIGATION, NO_SCREEN as i32);
        fe.note(format!("NAVIGATE_PendingNavigation → FEGenericNavMsg {{{s:#x}}}"));
        if s as u32 == PREVIOUS {
            fe.note("FEGenericNavMsg {0x58} → 0x8259FC30: the nav event previous");
            env.nav_events.push("previous".into());
        } else {
            fe.go_to_screen(env, s as u32);
        }
    }
}
