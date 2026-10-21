use ssx_formats::attrib::fnv1;

use super::profile::{DropState, Profile, Rules};
use super::world_tour;

pub const NEW_GAME_DONE: u32 = 22;

pub const NEW_GAME_HINT: u32 = 0x23E9_4763;

pub const NEW_GAME_SCREEN: u32 = 0x44;

pub const MAIN_INTRO_MOVIE: &str = "mainintro";
pub const ROCKIES_MOVIE: &str = "rockies";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrontEnd {
    pub hint_done: bool,
    pub front_end_idle: bool,
    pub movie_playing: bool,
    pub in_drop: bool,
    pub screen: u32,
    pub screen_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlowAction {
    ShowHint(u32),
    GlobeFocus { drop: u32, kind: u32 },
    PlayMovie(&'static str),
    GlobeProgression { from: u32, to: Vec<u32>, flag: bool },
    LaunchDrop(u32),
    GotoScreen(u32),
    LockInput(bool),
}

impl Profile {
    pub fn new_game_active(&self) -> bool {
        self.new_game_state != NEW_GAME_DONE
    }

    pub fn start_new_game(&mut self) -> bool {
        if self.new_game_active() {
            self.new_game_state = 1;
            true
        } else {
            false
        }
    }

    pub fn skip_new_game(&mut self) {
        self.new_game_state = NEW_GAME_DONE;
    }

    pub fn load_new_game_state(&mut self, saved: u32) {
        self.new_game_state = if saved >= 12 { NEW_GAME_DONE } else { 0 };
    }

    pub fn new_game_update(&mut self, rules: &Rules, fe: &FrontEnd) -> Vec<FlowAction> {
        let first = fnv1(world_tour::FIRST_DROP.as_bytes());
        let second = fnv1(world_tour::SECOND_DROP.as_bytes());
        let prologue = fnv1(world_tour::PROLOGUE_DROP.as_bytes());
        let mut out = Vec::new();
        match self.new_game_state {
            1 => {
                self.new_game_state = 2;
                out.push(FlowAction::ShowHint(NEW_GAME_HINT));
            }
            2 if fe.hint_done => {
                out.push(FlowAction::GlobeFocus { drop: first, kind: 2 });
                self.new_game_state = 7;
            }
            7 if fe.front_end_idle => {
                out.push(FlowAction::PlayMovie(MAIN_INTRO_MOVIE));
                self.new_game_state = 9;
                self.game_mode = 1;
                self.unlock_drop(rules, first);
                out.push(self.globe_progression(prologue, vec![first], false));
            }
            9 if fe.movie_playing => self.new_game_state = 10,
            10 if !fe.movie_playing => self.new_game_state = 0xB,
            0xB => {
                out.push(FlowAction::LaunchDrop(first));
                self.new_game_state = 0xC;
            }
            0xC if fe.in_drop => self.new_game_state = 0xD,
            0xD if !fe.in_drop => self.new_game_state = 0xE,
            0xE if fe.front_end_idle => {
                self.new_game_state = 0xF;
                self.complete_first_drop(rules, first, second);
                out.push(self.globe_progression(first, vec![second], false));
                out.push(FlowAction::GotoScreen(NEW_GAME_SCREEN));
            }
            0xF if fe.screen == NEW_GAME_SCREEN => {
                out.push(FlowAction::LockInput(true));
                self.new_game_state = 0x10;
            }
            0x10 if !fe.screen_busy => {
                out.push(FlowAction::LockInput(false));
                out.push(FlowAction::PlayMovie(ROCKIES_MOVIE));
                self.new_game_state = 0x11;
            }
            0x11 if fe.movie_playing => self.new_game_state = 0x12,
            0x12 if !fe.movie_playing => self.new_game_state = 0x13,
            0x13 => {
                self.complete_first_drop(rules, first, second);
                out.push(self.globe_progression(first, vec![second], true));
                self.new_game_state = NEW_GAME_DONE;
            }
            _ => {}
        }
        out
    }

    fn complete_first_drop(&mut self, rules: &Rules, first: u32, second: u32) {
        self.drop_states.insert(first, DropState::Completed);
        self.unlock_drop(rules, second);
    }

    fn globe_progression(&mut self, from: u32, to: Vec<u32>, flag: bool) -> FlowAction {
        self.globe_last_drop = super::globe::focus::return_node(from);
        FlowAction::GlobeProgression { from, to, flag }
    }

    pub fn is_tutorial_drop(&self, drop: u32) -> bool {
        let second = fnv1(world_tour::SECOND_DROP.as_bytes());
        let listed = [world_tour::FIRST_DROP, world_tour::SECOND_DROP, world_tour::PROLOGUE_DROP]
            .iter()
            .any(|d| fnv1(d.as_bytes()) == drop);
        listed && self.drop_state(second) != DropState::Completed
    }

    pub fn visit_range_by(&mut self, range: u32, rider: &str) -> u32 {
        let name = range_visit_stat(range, rider);
        self.add_persistent_stat(&name, 1);
        self.persistent_stat(&name)
    }

    pub fn range_visits_by(&self, range: u32, rider: &str) -> u32 {
        self.persistent_stat(&range_visit_stat(range, rider))
    }

    pub fn visit_range(&mut self, range: u32) -> u32 {
        self.visit_range_by(range, "")
    }

    pub fn persistent_stat(&self, name: &str) -> u32 {
        self.stats.records.get(&fnv1(name.as_bytes())).map_or(0, |r| r.value)
    }
}

pub fn range_visit_stat(range: u32, rider: &str) -> String {
    format!("NumTimesVisitedRange_{range}{rider}")
}

pub fn run_new_game(
    profile: &mut Profile,
    rules: &Rules,
    mut play: impl FnMut(&mut Profile, u32),
) -> Vec<(u32, FlowAction)> {
    let mut log = Vec::new();
    let mut fe = FrontEnd { front_end_idle: true, ..Default::default() };
    if !profile.start_new_game() {
        return log;
    }
    for _ in 0..64 {
        let state = profile.new_game_state;
        if state == NEW_GAME_DONE {
            break;
        }
        match state {
            2 => fe.hint_done = true,
            9 | 0x11 => fe.movie_playing = true,
            10 | 0x12 => fe.movie_playing = false,
            0xC => fe.in_drop = true,
            0xD => fe.in_drop = false,
            _ => {}
        }
        for a in profile.new_game_update(rules, &fe) {
            if let FlowAction::LaunchDrop(d) = a {
                play(profile, d);
            }
            if let FlowAction::GotoScreen(s) = a {
                fe.screen = s;
            }
            log.push((state, a));
        }
    }
    log
}
