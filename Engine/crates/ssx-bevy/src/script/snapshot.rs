use std::collections::VecDeque;
use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::gamepad::{GamepadAxisChangedEvent, GamepadButtonChangedEvent, GamepadInput};
use bevy::input::keyboard::{KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use ssx_physics::event::EventType;
use ssx_physics::rider::{RideState, World as PhysicsWorld};
use ssx_script::host::game::{
    GameFrame, Ground, InstanceView, KeyEvent, LevelView, LocalState, MarkerBudget, RideEvent, RiderView, StandingView,
};

use crate::ride::Ride;
use crate::shell::game_level::LevelGame;
use crate::shell::game_mode::GameUiRes;

const MAX_EVENTS: usize = 64;
const MARKERS_PER_LEVEL: usize = 1024;

pub(super) const BOT_KEYS: u64 = 1 << 16;
pub(super) const GHOST_KEYS: u64 = 1 << 32;

const KEYS: &[(KeyCode, &str)] = &[
    (KeyCode::F2, "f2"),
    (KeyCode::F3, "f3"),
    (KeyCode::F4, "f4"),
    (KeyCode::F5, "f5"),
    (KeyCode::F6, "f6"),
    (KeyCode::F7, "f7"),
    (KeyCode::F8, "f8"),
    (KeyCode::F10, "f10"),
    (KeyCode::F11, "f11"),
    (KeyCode::F12, "f12"),
    (KeyCode::Digit0, "0"),
    (KeyCode::Digit1, "1"),
    (KeyCode::Digit2, "2"),
    (KeyCode::Digit3, "3"),
    (KeyCode::Digit4, "4"),
    (KeyCode::Digit5, "5"),
    (KeyCode::Digit6, "6"),
    (KeyCode::Digit7, "7"),
    (KeyCode::Digit8, "8"),
    (KeyCode::Digit9, "9"),
    (KeyCode::Numpad0, "num_0"),
    (KeyCode::Numpad1, "num_1"),
    (KeyCode::Numpad2, "num_2"),
    (KeyCode::Numpad3, "num_3"),
    (KeyCode::Numpad4, "num_4"),
    (KeyCode::Numpad5, "num_5"),
    (KeyCode::Numpad6, "num_6"),
    (KeyCode::Numpad7, "num_7"),
    (KeyCode::Numpad8, "num_8"),
    (KeyCode::Numpad9, "num_9"),
    (KeyCode::NumpadAdd, "num_add"),
    (KeyCode::NumpadSubtract, "num_sub"),
    (KeyCode::NumpadMultiply, "num_mul"),
    (KeyCode::NumpadDivide, "num_div"),
    (KeyCode::NumpadDecimal, "num_dec"),
    (KeyCode::KeyB, "b"),
    (KeyCode::KeyM, "m"),
    (KeyCode::KeyO, "o"),
    (KeyCode::KeyP, "p"),
    (KeyCode::KeyU, "u"),
    (KeyCode::KeyA, "a"),
    (KeyCode::KeyC, "c"),
    (KeyCode::KeyD, "d"),
    (KeyCode::KeyE, "e"),
    (KeyCode::KeyF, "f"),
    (KeyCode::KeyG, "g"),
    (KeyCode::KeyH, "h"),
    (KeyCode::KeyI, "i"),
    (KeyCode::KeyJ, "j"),
    (KeyCode::KeyK, "k"),
    (KeyCode::KeyL, "l"),
    (KeyCode::KeyN, "n"),
    (KeyCode::KeyQ, "q"),
    (KeyCode::KeyR, "r"),
    (KeyCode::KeyS, "s"),
    (KeyCode::KeyT, "t"),
    (KeyCode::KeyV, "v"),
    (KeyCode::KeyW, "w"),
    (KeyCode::KeyX, "x"),
    (KeyCode::KeyY, "y"),
    (KeyCode::KeyZ, "z"),
    (KeyCode::Space, "space"),
    (KeyCode::Comma, ","),
    (KeyCode::Period, "."),
    (KeyCode::Slash, "/"),
    (KeyCode::Semicolon, ";"),
    (KeyCode::Quote, "'"),
    (KeyCode::BracketLeft, "["),
    (KeyCode::BracketRight, "]"),
    (KeyCode::Minus, "-"),
    (KeyCode::Equal, "="),
    (KeyCode::Backquote, "`"),
    (KeyCode::Backslash, "\\"),
    (KeyCode::Insert, "insert"),
    (KeyCode::Delete, "del"),
    (KeyCode::Pause, "pause"),
    (KeyCode::ScrollLock, "scroll"),
    (KeyCode::ArrowUp, "up"),
    (KeyCode::ArrowDown, "down"),
    (KeyCode::ArrowLeft, "left"),
    (KeyCode::ArrowRight, "right"),
    (KeyCode::Enter, "enter"),
    (KeyCode::NumpadEnter, "enter"),
    (KeyCode::Backspace, "bksp"),
];

/// A port addition for mods' menus: the gamepad buttons a script may bind, any pad's.
const PAD_BUTTONS: &[(GamepadButton, &str)] = &[
    (GamepadButton::DPadUp, "dp_up"),
    (GamepadButton::DPadDown, "dp_down"),
    (GamepadButton::DPadLeft, "dp_left"),
    (GamepadButton::DPadRight, "dp_right"),
    (GamepadButton::South, "pad_a"),
    (GamepadButton::East, "pad_b"),
    (GamepadButton::West, "pad_x"),
    (GamepadButton::North, "pad_y"),
    (GamepadButton::LeftTrigger, "pad_lb"),
    (GamepadButton::RightTrigger, "pad_rb"),
    (GamepadButton::Select, "pad_view"),
    (GamepadButton::Start, "pad_menu"),
    (GamepadButton::LeftThumb, "pad_ls"),
    (GamepadButton::RightThumb, "pad_rs"),
];

/// The gamepad buttons a package holds with the menu keys (the left stick, which scripts see as the D-pad, is held
/// too).
const PAD_MENU: [GamepadButton; 7] = [
    GamepadButton::DPadUp,
    GamepadButton::DPadDown,
    GamepadButton::DPadLeft,
    GamepadButton::DPadRight,
    GamepadButton::South,
    GamepadButton::East,
    GamepadButton::RightThumb,
];

/// A button's value at or past this is a press, and at or under [`BUTTON_RELEASE`] its release.
const BUTTON_PRESS: f32 = 0.75;
const BUTTON_RELEASE: f32 = 0.25;

/// The left stick past this is a D-pad press, and back under [`STICK_RELEASE`] its release.
const STICK_PRESS: f32 = 0.6;
const STICK_RELEASE: f32 = 0.35;
const STICK_KEYS: [&str; 4] = ["dp_up", "dp_down", "dp_left", "dp_right"];

/// The keys a package may hold for its menu (`captureMenuKeys`): while held, the game does not see them. (Port
/// addition: the letters and the space bar too, for typing in a menu.)
const MENU_CODES: &[KeyCode] = &[
    KeyCode::ArrowUp,
    KeyCode::ArrowDown,
    KeyCode::ArrowLeft,
    KeyCode::ArrowRight,
    KeyCode::Enter,
    KeyCode::NumpadEnter,
    KeyCode::Backspace,
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
    KeyCode::Space,
];

/// A package holds the menu keys (the last frame report said so). (Port addition: the game's pointer leaves the
/// mouse's clicks and wheel to it then, too.)
#[derive(Resource, Default)]
pub(crate) struct MenuCapture(pub bool);

/// Runs after [`keys`] (the scripts have their events) and before the game reads its keys.
pub(super) fn hold_menu_keys(
    capture: Res<MenuCapture>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut pads: Query<&mut Gamepad>,
) {
    if capture.0 {
        for &code in MENU_CODES {
            keys.reset(code);
        }
        for mut pad in &mut pads {
            for b in PAD_MENU {
                pad.digital_mut().reset(b);
            }
            for axis in [GamepadAxis::LeftStickX, GamepadAxis::LeftStickY] {
                pad.analog_mut().set(GamepadInput::Axis(axis), 0.0);
            }
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct Inbox {
    events: VecDeque<RideEvent>,
    keys: VecDeque<KeyEvent>,
    pub dropped: u64,
    seen_steps: u64,
    in_air: bool,
    air_steps: u32,
    level: u64,
    last_trick: Option<String>,
    seen_pushed: Option<ssx_physics::scoring::TrickRecord>,
    ride_seen: bool,
    level_markers: Option<Arc<Vec<ssx_script::host::game::LevelMarker>>>,
    held: std::collections::BTreeSet<&'static str>,
    /// Pixel-unit wheel scrolling not yet a whole notch (40 pixels a notch, as the game's pointer).
    wheel_px: f32,
    /// Each pad's left stick (from its own events, which the menu hold leaves alone) and the D-pad directions it
    /// holds down for the scripts (up, down, left, right).
    sticks: std::collections::HashMap<Entity, (Vec2, [bool; 4])>,
    /// The pad buttons down, each pad's own, from their values: the pads' pressed / released events can't be used, as
    /// bevy sends a release only for a button its state has as pressed, and the menu hold takes them out of it.
    pad_buttons: std::collections::HashSet<(Entity, GamepadButton)>,
}

impl Inbox {
    fn push_event(&mut self, e: RideEvent) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
            self.dropped += 1;
        }
        self.events.push_back(e);
    }
}

fn airborne(s: RideState) -> bool {
    !matches!(s, RideState::Riding)
}

pub(super) fn ride_events(
    ride: Res<Ride>,
    mut inbox: ResMut<Inbox>,
    mut ui: ResMut<GameUiRes>,
    level: Option<Res<LevelGame>>,
) {
    if ride.steps == inbox.seen_steps {
        return;
    }
    if ride.steps < inbox.seen_steps {
        (inbox.in_air, inbox.air_steps, inbox.last_trick) = (false, 0, None);
    }
    inbox.seen_steps = ride.steps;
    let s = &ride.state;
    let rider = level.as_ref().map_or_else(String::new, |l| l.script_view().0.rider.clone());
    let mut name_of = |r: &ssx_physics::scoring::TrickRecord| {
        let name = ssx_ui::game::tricks::trick_name(&mut ui.0.loc, &ssx_ui::game::feed::record_view(r), &rider, false);
        (!name.trim().is_empty()).then_some(name)
    };
    let pushed = s.score.last_pushed;
    if s.score.flushed.iter().any(Option::is_some) {
        let captured: Vec<_> =
            s.score.flushed.iter().flatten().filter(|f| f.is_points_captured && !f.is_bad_landing).collect();
        let record = pushed.filter(|r| Some(*r) != inbox.seen_pushed);
        if !captured.is_empty()
            && let Some(name) = record.as_ref().and_then(&mut name_of)
        {
            let points = captured
                .iter()
                .map(|f| f64::from(f.air_bank + f.flow_bank + f.grind_bank + f.board_press_bank + f.interactions_bank))
                .sum();
            inbox.push_event(RideEvent::Trick { name, points });
        }
        inbox.last_trick = None;
    }
    inbox.seen_pushed = pushed;
    if let Some(r) = s.score.trick_update {
        inbox.last_trick = name_of(&r);
    }
    let air = airborne(s.state);
    if air {
        inbox.air_steps = inbox.air_steps.saturating_add(1);
    } else if inbox.in_air {
        let air_time = inbox.air_steps as f32 / ssx_physics::SIM_HZ;
        inbox.push_event(RideEvent::Land { air_time, bail: s.landing.is_bailing() || s.landing.bail_on_landing });
        inbox.air_steps = 0;
        inbox.last_trick = None;
    }
    inbox.in_air = air;
}

impl Inbox {
    fn push_key(&mut self, key: &'static str, down: bool) {
        if self.keys.len() == MAX_EVENTS {
            self.keys.pop_front();
            self.dropped += 1;
        }
        self.keys.push_back(KeyEvent { key, down });
    }

    /// Every key a script was told is down goes up (focus lost, typing started).
    fn let_go(&mut self) {
        for key in std::mem::take(&mut self.held) {
            self.push_key(key, false);
        }
        for (_, dirs) in self.sticks.values_mut() {
            *dirs = [false; 4];
        }
        self.pad_buttons.clear();
    }

    fn press(&mut self, name: &'static str, down: bool) {
        if down {
            if self.held.insert(name) {
                self.push_key(name, true);
            }
        } else if self.held.remove(name) {
            self.push_key(name, false);
        }
    }
}

/// The scripts' keys come from the keyboard's own events, not from `ButtonInput`: [`hold_menu_keys`] takes the menu
/// keys out of `ButtonInput` while a package holds them, which would hide their releases. The OS's auto-repeat is
/// left out (a script repeats on its own), and every press a script saw gets its release.
pub(super) fn keys(
    mut typed: MessageReader<KeyboardInput>,
    mut lost: MessageReader<KeyboardFocusLost>,
    (mut buttons, mut axes): (MessageReader<GamepadButtonChangedEvent>, MessageReader<GamepadAxisChangedEvent>),
    mut inbox: ResMut<Inbox>,
    ui: Res<GameUiRes>,
    (mut mouse, mut wheel): (MessageReader<MouseButtonInput>, MessageReader<MouseWheel>),
) {
    let focus_lost = lost.read().count() > 0;
    if ui.text_focused() {
        typed.clear();
        buttons.clear();
        axes.clear();
        mouse.clear();
        wheel.clear();
        inbox.let_go();
        return;
    }
    // The mouse (a port addition): its buttons as keys, and each wheel notch as a press of wheel_up / wheel_dn.
    for e in mouse.read() {
        let name = match e.button {
            MouseButton::Left => "mouse1",
            MouseButton::Right => "mouse2",
            MouseButton::Middle => "mouse3",
            _ => continue,
        };
        inbox.press(name, e.state == ButtonState::Pressed);
    }
    let mut notches = 0i32;
    for e in wheel.read() {
        match e.unit {
            MouseScrollUnit::Line => notches = notches.saturating_add(e.y.round().clamp(-16.0, 16.0) as i32),
            MouseScrollUnit::Pixel => {
                inbox.wheel_px += e.y.clamp(-4096.0, 4096.0);
                let n = (inbox.wheel_px / 40.0).trunc();
                inbox.wheel_px -= n * 40.0;
                notches = notches.saturating_add(n as i32);
            }
        }
    }
    for _ in 0..notches.unsigned_abs().min(16) {
        let name = if notches > 0 { "wheel_up" } else { "wheel_dn" };
        inbox.push_key(name, true);
        inbox.push_key(name, false);
    }
    for e in typed.read() {
        let Some(&(_, name)) = KEYS.iter().find(|(code, _)| *code == e.key_code) else { continue };
        match e.state {
            ButtonState::Pressed if !e.repeat && inbox.held.insert(name) => inbox.push_key(name, true),
            ButtonState::Released if inbox.held.remove(name) => inbox.push_key(name, false),
            _ => {}
        }
    }
    // Gamepads, from their own events too (the menu hold resets the pads' state, which would hide releases), each
    // button by its value.
    for e in buttons.read() {
        let Some(&(_, name)) = PAD_BUTTONS.iter().find(|(b, _)| *b == e.button) else { continue };
        let key = (e.entity, e.button);
        let was = inbox.pad_buttons.contains(&key);
        if !was && e.value >= BUTTON_PRESS {
            inbox.pad_buttons.insert(key);
            inbox.press(name, true);
        } else if was && e.value <= BUTTON_RELEASE {
            inbox.pad_buttons.remove(&key);
            if !inbox.pad_buttons.iter().any(|(_, b)| *b == e.button) {
                inbox.press(name, false);
            }
        }
    }
    for e in axes.read() {
        let (stick, dirs) = inbox.sticks.entry(e.entity).or_insert((Vec2::ZERO, [false; 4]));
        match e.axis {
            GamepadAxis::LeftStickX => stick.x = e.value,
            GamepadAxis::LeftStickY => stick.y = e.value,
            _ => continue,
        }
        let values = [stick.y, -stick.y, -stick.x, stick.x];
        let mut changes = Vec::new();
        for (k, v) in values.into_iter().enumerate() {
            let on = if dirs[k] { v > STICK_RELEASE } else { v >= STICK_PRESS };
            if on != dirs[k] {
                dirs[k] = on;
                changes.push((STICK_KEYS[k], on));
            }
        }
        for (name, on) in changes {
            inbox.press(name, on);
        }
    }
    if focus_lost {
        inbox.let_go();
    }
}

struct LevelGround(Arc<PhysicsWorld>);

impl Ground for LevelGround {
    fn height(&self, x: f32, y: f32, z: f32) -> Option<f32> {
        let from = Vec3::new(x, y, z);
        self.0.collision.ray_cast(from, Vec3::NEG_Y, y + 1.0e5).map(|h| h.point.y)
    }
}

fn mode_name(t: Option<EventType>) -> &'static str {
    match t {
        Some(EventType::TrickScore) => "trick",
        Some(EventType::TrickBots) => "trick_bots",
        Some(EventType::TrickGhosts) => "trick_ghosts",
        Some(EventType::SurviveToBottom) => "survive",
        Some(EventType::SurviveToDeath) => "survive_to_death",
        Some(EventType::RaceClock) => "race",
        Some(EventType::RaceGhosts) => "race_ghosts",
        Some(EventType::RaceBots) => "race_bots",
        Some(EventType::FreeRide) => "freeride",
        None => "none",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Seen {
    Own,
    Session,
    Blind,
}

#[derive(SystemParam)]
pub(super) struct FrameView<'w> {
    time: Res<'w, Time<Real>>,
    ride: Option<Res<'w, Ride>>,
    level: Option<Res<'w, LevelGame>>,
    live: Option<Res<'w, crate::shell::live::LiveSession>>,
    #[cfg(feature = "net")]
    poses: Option<Res<'w, ssx_net::client::RemotePoses>>,
    #[cfg(feature = "net")]
    roster: Option<Res<'w, ssx_net::client::ClientRoster>>,
}

impl FrameView<'_> {
    pub(super) fn ride(&self) -> Option<&Ride> {
        self.ride.as_deref()
    }

    pub(super) fn live(&self) -> Option<&crate::shell::live::LiveSession> {
        self.live.as_deref()
    }

    /// A network session is up: its profile is the server's, so the scripts get none.
    pub(super) fn in_session(&self) -> bool {
        self.live.is_some()
    }

    pub(super) fn rider_name(&self) -> Option<String> {
        self.level.as_deref().map(|l| l.script_view().0.rider.clone())
    }

    pub(super) fn build(
        &self,
        inbox: &mut Inbox,
        state: &mut super::orders::OrderState,
        content: &ssx_assets::content::GameContent,
        values: &ssx_ui::game::values::UiValues,
        loc: &mut ssx_ui::game::loc::Localization,
        seen: Seen,
    ) -> GameFrame {
        let mut f = GameFrame {
            now: self.time.elapsed(),
            events: inbox.events.drain(..).collect(),
            keys: inbox.keys.drain(..).collect(),
            ..GameFrame::default()
        };
        f.hud_busy = state.hud.game_busy(values);
        if seen == Seen::Blind {
            f.events.clear();
            inbox.ride_seen = false;
            return f;
        }
        let Some(ride) = self.ride.as_deref() else {
            inbox.ride_seen = false;
            return f;
        };
        if !inbox.ride_seen {
            inbox.ride_seen = true;
            inbox.level += 1;
            inbox.level_markers = ride.map.as_ref().map(|m| Arc::new(m.markers.iter().map(level_marker).collect()));
        }
        f.level_markers = inbox.level_markers.clone();
        let s = &ride.state;
        let (feed, launch) = match self.level.as_deref() {
            Some(l) => {
                let (feed, launch) = l.script_view();
                (Some(feed), Some(launch))
            }
            None => (None, None),
        };
        let drop = launch.map_or_else(String::new, |l| format!("ssx:drop/{}", l.event.to_ascii_lowercase()));
        let used = launch
            .and_then(|l| content.decisions().into_iter().find(|d| d.track == l.track))
            .map(|d| (d.used.vertices, d.used.triangles))
            .unwrap_or((0, 0));
        let (own, own_markers) = ride.map.as_ref().map_or(((0, 0), 0), |m| (m.drawn, m.markers.len()));
        let budget = MarkerBudget {
            markers: MARKERS_PER_LEVEL.saturating_sub(own_markers),
            vertices: ssx_assets::content::track_packages::MAX_VERTICES.saturating_sub(used.0).saturating_sub(own.0),
            triangles: ssx_assets::content::track_packages::MAX_TRIANGLES.saturating_sub(used.1).saturating_sub(own.1),
        };
        f.level = Some(LevelView { id: inbox.level, drop: drop.clone(), budget });
        f.ground = Some(Arc::new(LevelGround(ride.world_arc())));
        let v = s.velocity * ssx_physics::TICKS_PER_SECOND;
        let health = if s.survival.health_max > 0.0 { s.survival.health / s.survival.health_max } else { 1.0 };
        let local = LocalState {
            velocity: v.to_array(),
            speed: v.length(),
            air: airborne(s.state),
            grind: s.score.prev_is_grinding,
            trick: inbox.last_trick.clone(),
            score: f64::from(s.score.score),
            health,
            finished: ride.session.event.result.is_some(),
        };
        let name = feed.map_or_else(String::new, |f| f.rider.clone());
        f.riders.push(RiderView { key: 0, name, position: s.traj.to_array(), local: Some(local) });
        if let Some(race) = &ride.session.bots {
            for (k, b) in race.bots.iter().enumerate().filter(|(_, b)| !b.removed) {
                let name = feed
                    .and_then(|f| f.rider_titles.get(&b.request.character))
                    .map_or_else(|| format!("Rival {}", k + 1), |key| loc.text(key));
                f.riders.push(RiderView {
                    key: BOT_KEYS + k as u64,
                    name,
                    position: b.position.to_array(),
                    local: None,
                });
            }
        }
        #[cfg(feature = "net")]
        if seen == Seen::Session
            && let (Some(poses), Some(roster)) = (self.poses.as_deref(), self.roster.as_deref())
        {
            let own = roster.own.map(|(r, _)| r);
            for (rider, p) in &roster.players {
                let Some(pose) = poses.riders.get(rider).filter(|_| Some(*rider) != own) else { continue };
                f.riders.push(RiderView {
                    key: ssx_script::host::game::SESSION_KEYS + u64::from(rider.0),
                    name: p.name.clone(),
                    position: pose.pose.block().trajectory_position,
                    local: None,
                });
            }
        }
        if let Some(feed) = feed {
            for g in &feed.ghosts {
                let key = GHOST_KEYS | (g.id & 0xFFFF_FFFF);
                if !f.riders.iter().any(|r| r.key == key) {
                    f.riders.push(RiderView { key, name: g.name.clone(), position: g.position, local: None });
                }
            }
        }
        let e = &ride.session.event;
        let state_name = if e.result.is_some() {
            "results"
        } else if ride.go_step.is_none() {
            "countdown"
        } else {
            "running"
        };
        let mode = mode_name(e.setup.event_type).to_string();
        f.instance = Some(InstanceView { mode, drop, state: state_name, clock_ms: f64::from(e.clock.ms()) });
        if values.bool(ssx_ui::game::values::id::LEADERBOARD_IS_ACTIVE) {
            for row in values.rows(ssx_ui::game::values::id::LEADERBOARD) {
                let rider =
                    if row.is_player { Some(0) } else { f.riders.iter().find(|r| r.name == row.name).map(|r| r.key) };
                let finished = row.is_player && ride.session.event.result.is_some();
                let place = u32::try_from(row.rank).ok().filter(|&p| p >= 1).unwrap_or(1);
                f.standings.push(StandingView {
                    rider,
                    name: row.name.clone(),
                    place,
                    value: f64::from(row.value),
                    finished,
                });
            }
        }
        f
    }
}

fn level_marker(m: &crate::ride::MapMarker) -> ssx_script::host::game::LevelMarker {
    let d = &m.look.def;
    ssx_script::host::game::LevelMarker {
        node: m.node.clone(),
        id: m.look.id.clone(),
        position: m.position,
        heading: m.heading,
        size: d.size,
        colour: d.colour,
        hit: super::worker::hit_shape(d.hit),
        sound: d.sound.clone(),
    }
}
