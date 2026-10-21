use ssx_physics::event::{EventState, EventType};
use ssx_physics::rider::{RiderState, World};

const USER_GAMERTAG: u16 = 0x22B;
const USER_GAMERPIC: u16 = 0x22C;
use ssx_physics::scoring::{RecordKind, TrickRecord};
use ssx_physics::session::{Execution, Session};

use super::GameUi;
use super::hud::HudMessage;
use super::hud::billboards::{Camera, Ghost, ghost_list, tombstone_list, tombstone_points, wingsuit_timer_pos};
use super::hud::trick::ComboEnded;
use super::leaderboard::{ExploreLeaderboard, RaceEntrant, TrickBot, TrickStandings, race_update};
use super::tricks::{TrickRecordView, trick_name};
use super::values::{SplitsCallout, UiValue, id};

pub const TRICKY_WARN_TICKS: f32 = 120.0;

// TODO(guess): true for every racer of the port (bots and ghosts keep their slot after their playback ended); the race leaderboard 0x824CC8B8 calls vt+0x28 of the racer record's +0x14 interface 0xD6FEE28B (at 0x824CCDD4; false → value −1, last); the player's actor component 0x82ABCFF0 registers 0xD6FEE28B with seven methods (0x82132568),
// so the racers' answer is another component's; the record's +0x10 interface 0x3D4D6AFE (vtable 0x821325CC) has a flag +0x454 (set by +0x24 0x82ABCB90, read by +0x28 0x82ABCB98, 0 from the constructor); settle: the component that answers 0xD6FEE28B for the racers and its +0x28.
pub const RACER_PRESENT: bool = true;

pub fn record_view(r: &TrickRecord) -> TrickRecordView {
    TrickRecordView {
        kind: r.kind as i32,
        trick: r.trick_id as u32,
        flips_fwd: r.flips_fwd,
        flips_bwd: r.flips_bwd,
        interaction: r.interaction,
        tier: u32::from(r.tier),
        tweaked: r.tweaked,
        awesome: r.awesome,
        super_awesome: r.super_awesome,
        nose_press: r.nose_press,
        goofy: r.goofy,
        duplicate: r.duplicate,
        spin_fs: r.spin_fs_deg,
        spin_bs: r.spin_bs_deg,
        distance: r.distance,
        time: r.trick_time,
    }
}

#[derive(Clone, Debug, Default)]
pub struct HudFeed {
    combo: Vec<TrickRecord>,
    record: TrickRecord,
    finish_posted: bool,
    ready_loop: bool,
    race_started: bool,
    event_state: EventState,
    pub explore: ExploreLeaderboard,
    pub camera: Option<Camera>,
    pub ghosts: Vec<Ghost>,
    pub rider: String,
    pub ghost_racers: Vec<RaceEntrant>,
    pub rider_titles: std::collections::BTreeMap<u32, String>,
    pub trick: TrickStandings,
}

impl HudFeed {
    pub fn new(rider: &str) -> Self {
        HudFeed { rider: rider.to_string(), ..Default::default() }
    }

    pub fn feed(&mut self, ui: &mut GameUi, s: &RiderState, x: &Session, world: &World, e: Option<&Execution>) {
        if let Some(c) = x.event.setup.course
            && !ui.values.written(id::START_ELEVATION)
        {
            ui.values.set_float(id::START_ELEVATION, c.start_y);
            ui.values.set_float(id::END_ELEVATION, c.end_y);
        }
        ui.gear = world.survival.slots;
        if e.is_some_and(|e| e.event.looped) {
            self.ready_loop = false;
            let (loops, descent) = (x.event.loops, x.event.descent_mm(s.score.descent));
            ui.fe_call(|fe, env| super::fe::flow::survive_loop(fe, env, loops, descent));
        }
        if let Some(o) = x.objectives.as_ref() {
            for (_, effect) in o.effects.get(x.effects_from..).unwrap_or(&[]) {
                if let ssx_physics::objectives::Effect::Fade { fade_out, seconds, delay } = effect {
                    let kind = if *fade_out { super::fe::fade::FADE_OUT } else { super::fe::fade::FADE_IN };
                    if let Some(fe) = ui.fe.as_mut() {
                        fe.fade_message(kind, *seconds, *delay);
                    }
                }
            }
        }
        let ready = s.start.as_ref().and_then(|l| l.intro.as_ref()).is_some_and(|i| i.in_ready_loop());
        if ready && !self.ready_loop {
            ready_loop_objectives(ui, x.event.loops);
            self.ready_loop = true;
        }
        if e.is_some_and(|e| e.race_start) {
            ui.values.set_bool(id::NIS_PROMPT_ACTIVE, false);
            ui.values.set_bool(id::BOOST_NOTIFICATION_ENABLED, false);
            ui.values.set_bool(id::METERS_ACTIVE, true);
            if ui.values.bool(id::HUDMESSAGE_IS_SURVIVAL_LOOP_MSG) {
                ui.values.set_bool(id::HUDMESSAGE_IS_SURVIVAL_LOOP_MSG, false);
                ui.values.set_bool(id::HUDMESSAGE_IS_ENABLED, false);
            }
            self.trick.reset(x.bots.as_ref().map_or(0, |b| b.bots.len()));
            if !self.race_started {
                self.race_started = true;
                ui.values.set_bool(id::RESULT_SCREEN_DATA_READY_FOR_PUBLISH, false);
                ui.values.set_bool(0x188, false);
            }
            let drop = ui.nav.drop_key;
            super::hud::show_drop_elements(&mut ui.values, drop);
            if let Some(fe) = ui.fe.as_mut() {
                super::fe::flow::race_start_camera(fe, &mut ui.values);
            }
        }
        let running = x.event.state == EventState::Running;
        self.messages(ui, s, x, world);
        if let Some((k, _ms)) = e.and_then(|e| e.event.split)
            && k >= 1
        {
            self.split_callout(ui, s, x, world);
        }
        if running {
            self.feed_hud(ui, s, x, world);
        }
        let started = matches!(
            x.event.state,
            EventState::Running
                | EventState::DelayedFinish
                | EventState::Finished
                | EventState::LoopOutro
                | EventState::LoopRestart
        );
        if started {
            let et = x.event.setup.event_type;
            if let Some(t @ (EventType::TrickGhosts | EventType::SurviveToDeath)) = et {
                let value = if t == EventType::TrickGhosts {
                    s.score.score
                } else {
                    ui.values.float(id::PLAYER_DESCENT_WITH_LAPS) * 1000.0
                };
                let player = self.player_name(ui);
                self.explore.update(&mut ui.values, &mut ui.loc, t, &player, value);
            }
        }
        let lb = matches!(x.event.state, EventState::Running | EventState::Finished) && !x.event.waiting;
        if lb && x.event.setup.event_type == Some(EventType::TrickBots) {
            self.trick_standings(ui, s, x, world);
        }
        if lb
            && let Some(t @ (EventType::RaceGhosts | EventType::RaceBots)) = x.event.setup.event_type
            && let Some(c) = x.event.setup.course
        {
            let player = RaceEntrant {
                name: self.player_name(ui),
                picture: ui.values.str(USER_GAMERPIC).to_string(),
                active: true,
                finish_ms: x.event.finish_ms,
                y: s.position.y,
            };
            let racers = self.racers(ui, x, world);
            race_update(&mut ui.values, &mut ui.loc, t, &player, &racers, c.start_y, c.end_y);
        }
        if x.event.finish_message.is_some() && !self.finish_posted {
            let key = if x.event.setup.event_type == Some(EventType::SurviveToDeath) {
                "TXT_HUDMESSAGE_SURVIVED"
            } else {
                "TXT_HUDMESSAGE_FINISHED"
            };
            let text = ui.loc.text(key);
            ui.values.set_str(id::FINISH_MESSAGE, text);
            ui.values.set_bool(id::FINISH_MESSAGE_ACTIVE, true);
            ui.values.set_bool(id::WINGSUIT_ACTIVE, false);
            self.finish_posted = true;
        }
        if x.event.state == EventState::LoopOutro && self.event_state != EventState::LoopOutro {
            super::hud::set_hud_shown(&mut ui.values, false, false, ui.nav.drop_key);
        }
        self.event_state = x.event.state;
        ui.values.set_bool(id::NIS_IS_ACTIVE, x.flow.nis_active || s.start.is_some());
    }

    fn player_name(&self, ui: &GameUi) -> String {
        match ui.values.str(USER_GAMERTAG) {
            "" => self.rider.clone(),
            t => t.to_string(),
        }
    }

    fn trick_standings(&mut self, ui: &mut GameUi, s: &RiderState, x: &Session, world: &World) {
        let player = RaceEntrant {
            name: self.player_name(ui),
            picture: ui.values.str(USER_GAMERPIC).to_string(),
            active: true,
            ..Default::default()
        };
        let drop = ui.nav.drop_key;
        let world_tour = ui
            .fe
            .as_ref()
            .and_then(|f| f.career.event(drop))
            .is_none_or(|e| e.drop_type == ssx_assets::career::DropType::Campaign);
        let racers = self.racers(ui, x, world);
        let bots: Vec<TrickBot> = x.bots.as_ref().map_or_else(Vec::new, |b| {
            b.bots
                .iter()
                .zip(racers)
                .enumerate()
                .map(|(k, (bot, r))| TrickBot {
                    name: r.name,
                    picture: r.picture,
                    live: x.event.racers.get(k).map_or(0.0, |p| p.points),
                    recorded: bot.replay.header.score,
                })
                .collect()
        });
        let progress = x.event.progress;
        self.trick.update(
            &mut ui.values,
            &mut ui.loc,
            EventType::TrickBots,
            &player,
            s.score.score,
            progress,
            world_tour,
            &bots,
        );
    }

    fn racers(&self, ui: &mut GameUi, x: &Session, world: &World) -> Vec<RaceEntrant> {
        let Some(b) = x.bots.as_ref() else { return self.ghost_racers.clone() };
        let inputs = b.racers(world);
        b.bots
            .iter()
            .enumerate()
            .map(|(k, bot)| {
                let key = self.rider_titles.get(&bot.request.character).cloned().unwrap_or_default();
                let name = ui.loc.text(&key);
                RaceEntrant {
                    picture: format!("assets\\gamertagIcons\\{}.dds", name.to_ascii_lowercase()),
                    name,
                    active: RACER_PRESENT,
                    finish_ms: x.event.racers.get(k).and_then(|r| r.finish_ms),
                    y: inputs.get(k).copied().flatten().map_or(bot.position.y, |r| r.y),
                }
            })
            .collect()
    }

    fn feed_hud(&mut self, ui: &mut GameUi, s: &RiderState, x: &Session, world: &World) {
        let sc = &s.score;
        let rider = self.rider.clone();
        let name = match self.record.kind {
            RecordKind::None | RecordKind::Rewind => String::new(),
            _ => trick_name(&mut ui.loc, &record_view(&self.record), &rider, false),
        };
        let tombstones = tombstone_points(x.tombstones.as_ref(), &mut ui.loc);
        let v = &mut ui.values;
        v.set_int(id::RACE_TIME, x.event.clock.ms() as i32);
        v.set_float(id::PLAYER_SPEED, s.player_speed);
        v.set_float(id::PLAYER_DESCENT_WITH_LAPS, x.event.descent_mm(sc.descent) as f32 * 0.001);
        v.set_float(id::PLAYER_DESCENT, ((sc.descent * 1000.0) as i64 as f32) * 0.001);
        v.set_float(id::SCORE, sc.score);
        // TODO(guess): derived from the boost at each feed (the game stores it when the scoring op runs, 0x82CF0230); settle: the frames the op is skipped (pause, NIS, rewind snapshots, 0x82CF1C70).
        v.set_float(id::BOOST_PERCENT, sc.boost / world.tuning.scoring.max_boost);
        v.set_bool(id::IS_BOOSTING, s.boost.is_boosting);
        v.set_bool(id::IS_REWINDING, sc.is_rewinding_backwards);
        let count = x.manager.count;
        v.set_int(id::REWIND_COUNT, count.rewinds_left);
        v.set_int(id::REWIND_COUNT_INITIAL, x.manager.tuning.max_rewind_count);
        v.set_bool(id::REWIND_COUNT_INFINITE, count.infinite);
        v.set_bool(id::REWIND_COUNT_ENABLED, count.enabled);
        v.set_float(id::REWIND_SCORE, if sc.is_rewinding_backwards { sc.rewind_bank } else { 0.0 });
        v.set_int(id::FLOW_LEVEL, sc.flow_level);
        v.set_bool(id::IS_AWESOME, sc.is_awesome);
        let timer = if sc.is_super_awesome { sc.super_timer } else { sc.awesome_timer };
        v.set_int(id::AWESOME_WARN_TIER, i32::from(timer > 0.0 && timer <= TRICKY_WARN_TICKS));
        v.set_bool(id::IS_SUPER_AWESOME, sc.is_super_awesome);
        v.set_float(id::SUPER_AWESOME_ACTIVATION_PERCENTAGE, sc.super_activation);
        v.set_float(id::TRICK_AIR_DIST, sc.flight_distance);
        v.set_int(id::TRICK_TIER, i32::from(self.record.tier));
        v.set_bool(id::TRICK_IS_DUPLICATE, self.record.duplicate);
        v.set_str(id::TRICK_GRAB_NAME, name);
        v.set_float(id::COMBO_CURRENT_TIMER_PERCENTAGE, sc.combo_pct);
        v.set_float(id::WINGSUIT_USAGE, s.wingsuit.use_percentage);
        v.set_bool(id::WINGSUIT_ACTIVE, s.wingsuit.signal && x.event.state == EventState::Running);
        let h = s.survival.hud();
        v.set_float(id::ARMOUR, h.armour);
        v.set_float(id::HEAD_LAMP_LEVEL, h.head_lamp_battery);
        v.set_float(id::PULSE_GOGGLE_LEVEL, h.pulse_goggles_battery);
        v.set_float(id::HEALTH, h.health);
        v.set_float(id::HEALTH_MAX, h.health_max);
        v.set_float(id::TEMPERATURE, h.temperature);
        v.set_float(id::TEMPERATURE_MAX, h.temperature_max);
        v.set_float(id::ICE_AXE_EFFECTIVENESS, h.ice_axe_effectiveness);
        v.set_float(id::WINGSUIT_EFFECTIVENESS, s.wingsuit.effectiveness);
        v.set_float(id::OXYGEN_TANK_LEVEL, h.oxygen_tank);
        v.set_bool(id::IS_OXYGEN_USE_REQUESTED, h.oxygen_use_requested);
        if let Some(cam) = &self.camera {
            if s.in_air() {
                let p = s.position;
                let [x, y] = wingsuit_timer_pos(cam, [p.x, p.y, p.z]);
                v.set_float(id::WINGSUIT_TIMER_POS_X, x);
                v.set_float(id::WINGSUIT_TIMER_POS_Y, y);
            }
            v.set(id::GHOST_LIST, UiValue::Markers(ghost_list(cam, &self.ghosts)));
            v.set(id::TOMBSTONE_LIST, UiValue::Markers(tombstone_list(cam, &tombstones)));
        } else {
            // TODO(guess): without a camera (the headless tools) the wingsuit timer sits at the stage centre (640, 541); settle: none (the viewer passes its camera, `HudFeed::camera`).
            v.set_float(id::WINGSUIT_TIMER_POS_X, 640.0);
            v.set_float(id::WINGSUIT_TIMER_POS_Y, 541.0);
        }
    }

    fn messages(&mut self, ui: &mut GameUi, s: &RiderState, x: &Session, world: &World) {
        let sc = &s.score;
        let rider = self.rider.clone();
        let mut list_changed = false;
        // TODO(guess): the messages are delivered after the step in the order sent (posted via 0x831A3AE0 / 0x82AB7AA0); settle: the entity message queue's dispatch point.
        if let Some(r) = sc.trick_update {
            self.record = r;
        }
        for f in sc.flushed.iter().flatten() {
            if f.is_bad_landing {
                self.combo.clear();
            } else if self.record.kind != RecordKind::None {
                self.combo.push(self.record);
            }
            self.record = TrickRecord::default();
            ui.values.set_int(id::COMBO_LEVEL, sc.combo_count as i32);
            ui.values.set_float(id::COMBO_TRICK_POINTS, sc.combo_points);
            ui.values.set_float(id::COMBO_TOTAL_POINTS, sc.combo_points);
            list_changed = true;
            ui.hud.messages.push(HudMessage::BanksFlushed { captured: f.is_points_captured });
        }
        if let Some(m) = sc.combo_ended {
            ui.values.set_float(id::COMBO_BONUS_POINTS, m.bonus_points);
            ui.values.set_float(id::COMBO_TRICK_POINTS, m.trick_points);
            ui.values.set_float(id::COMBO_TOTAL_POINTS, m.total_points);
            ui.values.set_int(id::COMBO_LEVEL, 0);
            ui.values.set_bool(id::COMBO_SUCCESS, m.success);
            self.combo.clear();
            self.record = TrickRecord::default();
            list_changed = true;
            if x.event.setup.event_type == Some(EventType::TrickBots) {
                self.trick_standings(ui, s, x, world);
            }
            ui.hud.messages.push(HudMessage::ScoreComboEnded(ComboEnded {
                success: m.success,
                trick_points: m.trick_points,
                bonus_points: m.bonus_points,
                total_points: m.total_points,
                combo_level: m.combo_level,
            }));
        }
        if list_changed {
            let names: Vec<String> =
                self.combo.iter().map(|r| trick_name(&mut ui.loc, &record_view(r), &rider, true)).collect();
            ui.values.set(id::COMBO_TRICK_LIST, UiValue::List(names));
        }
    }

    fn split_callout(&mut self, ui: &mut GameUi, s: &RiderState, x: &Session, world: &World) {
        let Some(course) = x.event.setup.course else { return };
        let racers = self.racers(ui, x, world);
        let Some(low) = racers.iter().fold(None::<&RaceEntrant>, |a, r| match a {
            Some(b) if b.y <= r.y => Some(b),
            _ => Some(r),
        }) else {
            return;
        };
        let (me, y, start) = (s.position.y, low.y, course.start_y);
        let clock = x.event.clock.ms() as f32;
        let ahead = y > me;
        let gap = if ahead { clock / (start - me) * (y - me) } else { clock / (start - y) * (me - y) };
        let kind = x.event.setup.event_type.map_or(2, super::leaderboard::value_kind);
        let gap = super::leaderboard::value_text(&mut ui.loc, kind, gap as u32 as i32, true);
        let key = if ahead { "TXT_SPLITS_YOU_ARE_AHEAD" } else { "TXT_SPLITS_YOU_ARE_BEHIND" };
        let body = ui.loc.format(key, &[&gap, &low.name]);
        let callout = SplitsCallout { ahead, show_time: false, kind: 2, body, ..Default::default() };
        ui.values.set(id::SPLITS_CALLOUT, UiValue::Splits(callout));
        ui.values.set_bool(id::SPLITS_CALLOUT_ENABLED, true);
    }
}

const NO_PROMPT_DROPS: [u32; 3] = [0x28F6_34F8, 0xC145_699A, 0xB0F3_51A2];

pub fn nis_prompt_shown(drop: u32, no_prompt: bool, tutorial: bool, loops: u32) -> bool {
    !no_prompt && !tutorial && loops == 0 && !NO_PROMPT_DROPS.contains(&drop)
}

fn ready_loop_objectives(ui: &mut GameUi, loops: u32) {
    let drop = ui.nav.drop_key;
    let tutorial = match ui.fe.as_ref() {
        Some(fe) => fe.profile.is_tutorial_drop(drop),
        None => ssx_assets::career::Profile::new().is_tutorial_drop(drop),
    };
    let v = &mut ui.values;
    if nis_prompt_shown(drop, v.bool(id::NO_NIS_PROMPT), tutorial, loops) {
        let text = ui.loc.text("TXT_NISPROMPT_MESSAGE");
        v.set_str(id::NIS_PROMPT_MESSAGE, text);
        v.set_int(id::NIS_PROMPT_CALLOUT, 4);
        v.set_bool(id::NIS_PROMPT_ACTIVE, true);
    }
    v.set_bool(id::NO_NIS_PROMPT, false);
    if !tutorial {
        v.set_bool(id::BOOST_NOTIFICATION_ENABLED, true);
    }
    v.set_str(id::NIS_TITLE_MESSAGE, "");
    for i in [id::NIS_TITLE_ACTIVE, id::NIS_TITLE_ENABLED, id::NIS_TITLE_INITIALIZED] {
        v.set_bool(i, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_assets::career::Profile;
    use ssx_assets::career::profile::DropState;
    use ssx_formats::attrib::fnv1;

    #[test]
    fn the_ready_loops_prompt_follows_the_profile_and_the_loops() {
        let (prologue, first, second) =
            (fnv1(b"chapter_1_drop_h2h"), fnv1(b"chapter_2_drop_h2h"), fnv1(b"chapter_2_drop_gen_1"));
        let (final_drop, other) = (fnv1(b"chapter_11_drop_h2h"), fnv1(b"exp_alps_vorassay_drop_3"));
        let mut p = Profile::new();
        assert!(p.is_tutorial_drop(prologue) && p.is_tutorial_drop(first) && p.is_tutorial_drop(second));
        assert!(!nis_prompt_shown(prologue, false, p.is_tutorial_drop(prologue), 0));
        p.drop_states.insert(second, DropState::Completed);
        assert!(nis_prompt_shown(prologue, false, p.is_tutorial_drop(prologue), 0));
        assert!(!p.is_tutorial_drop(first), "the boost notification shows on a replayed tutorial");
        assert!(!nis_prompt_shown(first, false, p.is_tutorial_drop(first), 0));
        assert!(!nis_prompt_shown(second, false, p.is_tutorial_drop(second), 0));
        assert!(!nis_prompt_shown(final_drop, false, false, 0));
        assert!(nis_prompt_shown(other, false, false, 0));
        assert!(!nis_prompt_shown(other, true, false, 0), "HUD_NONISPROMPT");
        assert!(!nis_prompt_shown(other, false, false, 1), "a survive loop's next intro");
    }
}
