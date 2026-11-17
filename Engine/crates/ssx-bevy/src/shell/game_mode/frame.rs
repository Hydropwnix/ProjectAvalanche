use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{
    GameUiRes, LAUNCH_BOOT_WAIT, PadSlots, Shell, Stage, UiPipelinesReady, WorldCheck, pad_presses, screenshot,
};
use crate::audio::music::frontend::FrontEndAudio;
use crate::shell::game_level;
use crate::ui;

#[derive(SystemParam)]
pub(super) struct TextKeys<'w, 's> {
    typed: MessageReader<'w, 's, bevy::input::keyboard::KeyboardInput>,
    alt_enter: Res<'w, super::display::AltEnter>,
    ime: MessageReader<'w, 's, bevy::window::Ime>,
    windows: Query<'w, 's, &'static mut Window, With<bevy::window::PrimaryWindow>>,
    clipboard: Option<ResMut<'w, bevy::clipboard::Clipboard>>,
}

/// A held direction (the D-pad, the arrows or the left stick) on a list that scrolls on its own while it's held: which,
/// from which device, whether it came from the stick, and when it next moves the list again.
static HOLD_REPEAT: std::sync::Mutex<Option<(ssx_ui::game::input::Button, u8, bool, Instant)>> =
    std::sync::Mutex::new(None);
/// On the rider list: held this long, left or right starts moving the list on its own; then a card every this often
/// (seconds; a card's slide is about this long, so the list never outruns its cards).
const RIDER_REPEAT_DELAY: f32 = 0.4;
const RIDER_REPEAT_EVERY: f32 = 0.22;
/// On the equipment screen's lists (its options, its gear, the full list): held this long, then an item this often.
const LIST_REPEAT_DELAY: f32 = 0.4;
const LIST_REPEAT_EVERY: f32 = 0.13;
/// The left stick held this far over counts as held that way.
const REPEAT_STICK: f32 = 0.4;

/// The equipment screen past its rider step, and its full list (View More) (the Custom Event Menus mod): the mouse
/// and the keyboard work there.
pub(super) fn gear_screen(fe: &ssx_ui::game::fe::FrontEnd) -> bool {
    ssx_core::rider_showcase::enabled()
        && (fe.screen == ssx_ui::game::fe::screens::id::EQUIPMENT || fe.screen == 0x31)
        && !ssx_ui::game::fe::equipment::showcase::active(fe)
}

/// Where a held direction scrolls on its own (the Custom Event Menus mod): the rider list (left and right), and the
/// equipment screen's other lists and its full list (every way); the first delay and then how often.
fn hold_repeat(ui: &GameUiRes, b: ssx_ui::game::input::Button) -> Option<(f32, f32)> {
    use ssx_ui::game::input::Button;
    if !ssx_core::rider_showcase::enabled() {
        return None;
    }
    let fe = ui.fe.as_ref()?;
    let horizontal = matches!(b, Button::DpadLeft | Button::DpadRight);
    let vertical = matches!(b, Button::DpadUp | Button::DpadDown);
    if ssx_ui::game::fe::equipment::showcase::active(fe) {
        return horizontal.then_some((RIDER_REPEAT_DELAY, RIDER_REPEAT_EVERY));
    }
    let lists = fe.screen == ssx_ui::game::fe::screens::id::EQUIPMENT || fe.screen == 0x31;
    (lists && (horizontal || vertical)).then_some((LIST_REPEAT_DELAY, LIST_REPEAT_EVERY))
}

/// The rider list's left and right swapped (the 3D Rider Select mod: its list runs from the highlighted card rightwards,
/// so right moves the next rider in).
pub(super) fn rider_list_turned(b: ssx_ui::game::input::Button) -> ssx_ui::game::input::Button {
    use ssx_ui::game::input::Button;
    match b {
        Button::DpadLeft => Button::DpadRight,
        Button::DpadRight => Button::DpadLeft,
        other => other,
    }
}

pub(super) fn shell_input(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    slots: Res<PadSlots>,
    mut shell: ResMut<Shell>,
    mut ui: ResMut<GameUiRes>,
    level: Option<Res<game_level::LevelGame>>,
    mut text: TextKeys,
) {
    let s = &mut *shell;
    if s.exiting.is_some() {
        text.typed.clear();
        text.ime.clear();
        return;
    }
    let focused = s.started && ui.text_focused();
    if let Ok(mut w) = text.windows.single_mut()
        && w.ime_enabled != focused
    {
        w.ime_enabled = focused;
    }
    if focused {
        let ctrl = [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::SuperLeft, KeyCode::SuperRight]
            .iter()
            .any(|k| keys.pressed(*k));
        text_input(&mut text, &mut ui, ctrl);
        if ctrl
            && keys.just_pressed(KeyCode::KeyV)
            && let Some(clipboard) = text.clipboard.as_deref_mut()
        {
            paste(clipboard, &mut ui);
        }
    } else {
        text.typed.clear();
        text.ime.clear();
    }
    if !s.started || level.is_some_and(|l| l.takes_input()) {
        return;
    }
    let keyboard = ssx_ui::game::input::KEYBOARD_DEVICE;
    let search = ui.browser_active();
    let pad_only = ui.fe.as_ref().is_some_and(|fe| ssx_ui::game::fe::screens::pad_only(fe.screen));
    let rider_list = ssx_core::rider_showcase::enabled()
        && ui.fe.as_ref().is_some_and(ssx_ui::game::fe::equipment::showcase::active);
    // (a port addition: on a range's peaks and a peak's drops the keyboard's left and right arrows, Enter (A) and
    // Backspace (B) work; on the drops X too (Freeride), and for the test peak up, down and Y)
    let peak_keys = ui.fe.as_ref().is_some_and(ssx_ui::game::fe::globe::peak_screen);
    let drop_keys = ui.fe.as_ref().is_some_and(ssx_ui::game::fe::globe::drop_screen);
    let gear_keys = ui.fe.as_ref().is_some_and(gear_screen);
    for (b, device) in pad_presses(&keys, &pads, &slots, &mut s.stick_dpad, search) {
        use ssx_ui::game::input::Button;
        // (and on the rider step (the Custom Event Menus mod): left, right, Enter (A), Backspace (B) and X (a level);
        // on the gear steps every key)
        let globe_keys = (peak_keys || drop_keys || rider_list || gear_keys) && device == keyboard;
        let b = if globe_keys && b == Button::Start { Button::DigitalOne } else { b };
        let peak_key = globe_keys
            && (gear_keys
                || matches!(b, Button::DpadLeft | Button::DpadRight | Button::DigitalOne | Button::DigitalTwo)
                || ((drop_keys || rider_list) && b == Button::DigitalFour)
                || (drop_keys && matches!(b, Button::DigitalThree | Button::DpadUp | Button::DpadDown)));
        if (focused || pad_only) && device == keyboard && !(peak_key && !focused) {
            continue;
        }
        // (the 3D Rider Select mod: Y on a suit opens its menu (a favourite, its full portrait), which then takes the
        // buttons while it's up)
        if let Some(fe) = ui.fe.as_mut()
            && ssx_ui::game::fe::equipment::take_suit_press(fe, b)
        {
            continue;
        }
        // (the test peak: Y on its drop screen turns its events between Trick It and Race It)
        if let Some(fe) = ui.fe.as_mut()
            && ssx_ui::game::fe::globe::take_test_peak_press(fe, b)
        {
            continue;
        }
        if let Some((delay, _)) = hold_repeat(&ui, b)
            && let Ok(mut r) = HOLD_REPEAT.lock()
        {
            *r = Some((b, device, false, Instant::now() + std::time::Duration::from_secs_f32(delay)));
        }
        let b = if rider_list { rider_list_turned(b) } else { b };
        s.host.press_from(&mut ui, b, device);
    }
    // (the left stick's D-pad presses, as the D-pad's on the rider list)
    for (b, device) in s.stick_dpad.take() {
        if let Some((delay, _)) = hold_repeat(&ui, b)
            && let Ok(mut r) = HOLD_REPEAT.lock()
        {
            *r = Some((b, device, true, Instant::now() + std::time::Duration::from_secs_f32(delay)));
        }
        let b = if rider_list { rider_list_turned(b) } else { b };
        s.host.press_from_stick(&mut ui, b, device);
    }
    // (the Custom Event Menus mod: a direction held keeps the list going, an item at a time (on the rider list no
    // faster than the cards slide))
    if let Ok(mut r) = HOLD_REPEAT.lock() {
        use ssx_ui::game::input::Button;
        let held = |b: Button| {
            let (key, button, axis, sign) = match b {
                Button::DpadLeft => (KeyCode::ArrowLeft, GamepadButton::DPadLeft, GamepadAxis::LeftStickX, -1.0),
                Button::DpadRight => (KeyCode::ArrowRight, GamepadButton::DPadRight, GamepadAxis::LeftStickX, 1.0),
                Button::DpadUp => (KeyCode::ArrowUp, GamepadButton::DPadUp, GamepadAxis::LeftStickY, 1.0),
                _ => (KeyCode::ArrowDown, GamepadButton::DPadDown, GamepadAxis::LeftStickY, -1.0),
            };
            (keys.pressed(key) && !focused && (!pad_only || rider_list || gear_keys))
                || pads.iter().any(|(_, p)| p.pressed(button) || p.get(axis).unwrap_or(0.0) * sign > REPEAT_STICK)
        };
        match *r {
            Some((b, device, from_stick, next)) if held(b) && hold_repeat(&ui, b).is_some() => {
                let now = Instant::now();
                if now >= next {
                    let every = hold_repeat(&ui, b).map_or(LIST_REPEAT_EVERY, |(_, e)| e);
                    let turned = if rider_list { rider_list_turned(b) } else { b };
                    if from_stick {
                        s.host.press_from_stick(&mut ui, turned, device);
                    } else {
                        s.host.press_from(&mut ui, turned, device);
                    }
                    *r = Some((b, device, from_stick, now + std::time::Duration::from_secs_f32(every)));
                }
            }
            Some(_) => *r = None,
            None => {}
        }
    }
    let mut stick = Vec2::ZERO;
    for (_, pad) in &pads {
        stick +=
            Vec2::new(pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0), pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0));
    }
    let key = |k: KeyCode| if keys.pressed(k) && !focused && !pad_only { 1.0 } else { 0.0 };
    stick += Vec2::new(key(KeyCode::KeyD) - key(KeyCode::KeyA), key(KeyCode::KeyW) - key(KeyCode::KeyS));
    if (stick != Vec2::ZERO || s.last_stick != Vec2::ZERO)
        && let Some(fe) = ui.fe.as_mut()
    {
        ssx_ui::game::fe::globe::stick(fe, stick.x, stick.y);
    }
    s.last_stick = stick;
    if keys.just_pressed(KeyCode::F1) && s.movie.is_some() {
        s.movie = None;
        s.movie_ended = ui.fe.as_ref().map(|x| x.movie_serial);
        s.host.finish_movie(&mut ui);
    }
}

fn paste(clipboard: &mut bevy::clipboard::Clipboard, ui: &mut GameUiRes) {
    use ssx_ui::game::fe::ava::keyboard::TextInput as T;
    match clipboard.fetch_text().poll_result() {
        Some(Ok(text)) => {
            let typed = text.chars().filter(|c| !c.is_control()).take(MAX_PASTE);
            typed.for_each(|c| ui.text_input(T::Char(c)));
        }
        Some(Err(e)) => warn!("shell: the clipboard: {e}"),
        None => {}
    }
}

const MAX_PASTE: usize = ssx_ui::game::fe::ava::keyboard::MAX_LEN;

fn text_input(text: &mut TextKeys, ui: &mut GameUiRes, ctrl: bool) {
    use bevy::input::ButtonState;
    use ssx_ui::game::fe::ava::keyboard::TextInput as T;
    for e in text.alt_enter.typed(text.typed.read_with_id()) {
        if ctrl {
            continue;
        }
        if e.state == ButtonState::Pressed {
            for i in key_text(e.key_code, e.text.as_deref()) {
                ui.text_input(i);
            }
        }
    }
    for e in text.ime.read() {
        if let bevy::window::Ime::Commit { value, .. } = e {
            value.chars().filter(|c| !c.is_control()).for_each(|c| ui.text_input(T::Char(c)));
        }
    }
}

// PORT(choice): Enter confirms and Escape cancels a text field, Backspace / Delete / the arrows / Home / End edit it;
// the game has no text entry and no keyboard text.
pub(super) fn key_text(key: KeyCode, text: Option<&str>) -> Vec<ssx_ui::game::fe::ava::keyboard::TextInput> {
    use ssx_ui::game::fe::ava::keyboard::TextInput as T;
    let edit = match key {
        KeyCode::Backspace => T::Backspace,
        KeyCode::Delete => T::Delete,
        KeyCode::ArrowLeft => T::Left,
        KeyCode::ArrowRight => T::Right,
        KeyCode::Home => T::Home,
        KeyCode::End => T::End,
        KeyCode::Enter | KeyCode::NumpadEnter => T::Enter,
        KeyCode::Escape => T::Escape,
        _ => return text.unwrap_or("").chars().filter(|c| !c.is_control()).map(T::Char).collect(),
    };
    vec![edit]
}

#[derive(SystemParam)]
pub(super) struct Frame<'w, 's> {
    commands: Commands<'w, 's>,
    time: Res<'w, Time>,
    real: Res<'w, Time<Real>>,
    scene: ResMut<'w, ui::UiScene>,
    images: ResMut<'w, Assets<Image>>,
    sprites: Query<'w, 's, (&'static mut Sprite, &'static mut Visibility)>,
    cameras: Query<'w, 's, &'static mut Camera>,
    windows: Query<'w, 's, &'static Window, With<bevy::window::PrimaryWindow>>,
    ready: Res<'w, UiPipelinesReady>,
    ui_frame: Res<'w, ui::UiFrame>,
    globe: ResMut<'w, ui::globe::GlobeView>,
    fade: ResMut<'w, ui::fade::ScreenFade>,
    level: Option<ResMut<'w, game_level::LevelGame>>,
    ride: Option<Res<'w, crate::ride::Ride>>,
    audio: Option<ResMut<'w, FrontEndAudio>>,
    world_check: Res<'w, WorldCheck>,
    exit: MessageWriter<'w, AppExit>,
    closes: MessageReader<'w, 's, bevy::window::WindowCloseRequested>,
    display: Option<ResMut<'w, super::display::DisplayState>>,
    exit_wait: ResMut<'w, super::exit::ExitWait>,
    live: Option<ResMut<'w, crate::shell::live::LiveSession>>,
    render_scale: Option<Res<'w, super::display::RenderScale>>,
    fe_camera: Query<'w, 's, Entity, With<super::FrontEndCamera>>,
    render_stopped: Option<Res<'w, crate::shell::device_loss::RenderStopped>>,
    render_stop_end: Option<ResMut<'w, crate::shell::device_loss::ShellEndsOnRenderStop>>,
}

fn frame_clocks(time: &Time, real: &Time<Real>) -> (f64, f64) {
    (time.delta_secs_f64().min(0.1), real.delta_secs_f64())
}

/// The globe counts as zoomed out while its radius on the screen is under this (in screen heights).
const GLOBE_OUT_RADIUS: f32 = 0.75;

pub(super) fn shell_step(mut shell: ResMut<Shell>, mut ui: ResMut<GameUiRes>, mut f: Frame) {
    let s = &mut *shell;
    if f.closes.read().count() > 0 {
        s.ask_exit(AppExit::Success, "the window's close");
    }
    if let Some(stopped) = f.render_stopped.as_ref().filter(|r| r.is_added()) {
        let why = if stopped.device_lost { "the graphics device was lost" } else { "the renderer stopped on an error" };
        s.ask_exit(AppExit::error(), why);
    }
    s.exit_frame(&ui, f.display.as_deref_mut(), &mut f.exit_wait, &mut f.exit);
    if s.exit_written()
        && let Some(end) = f.render_stop_end.as_deref_mut()
        && !end.exit_written
    {
        end.exit_written = true;
    }
    if let Some(ms) = s.frame_sleep {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    for n in std::mem::take(&mut s.notes) {
        if n.contains("ERROR") {
            error!("{n}");
        } else {
            info!("{n}");
        }
    }
    if f.ui_frame.viewport.y > 0 {
        let aspect = f.ui_frame.viewport.x as f32 / f.ui_frame.viewport.y as f32;
        let globe = match s.stage {
            Stage::Level => None,
            _ => ui.fe.as_ref().and_then(|fe| fe.globe_frame(aspect)),
        };
        // A mod's background movie shows behind the globe, in place of its backdrop (a port addition).
        s.background_wanted = globe.is_some() && s.background_available();
        s.globe_shown = globe.is_some();
        // Zoomed out (the main menu's view): the whole globe on the screen with room around it, not a range up close.
        let radius = globe
            .as_ref()
            .filter(|g| g.visible && g.midground.is_none())
            .map_or(0.0, |g| ui::globe::screen_circle(g).2);
        let out = radius > 0.0 && radius < GLOBE_OUT_RADIUS;
        if out != ssx_core::menu_background::globe_out() {
            info!("shell: the globe zoomed {} (its radius {radius:.2} of the screen's height)", if out { "out" } else { "in" });
            ssx_core::menu_background::set_globe_out(out);
        }
        f.globe.frame = globe;
    }
    if let Some(window) = f.windows.iter().next() {
        let now = f.real.elapsed_secs_f64();
        super::display::display_class_frame(&mut s.display_pending, s.started, &mut ui, &mut f.scene, window, now);
    }
    let drawn = f.ready.0 && f.ui_frame.viewport.x > 0;
    s.ready_frames = if drawn { s.ready_frames + 1 } else { 0 };
    if !s.started {
        s.started = s.ready_frames >= 5;
        if !s.started {
            return;
        }
    }
    if s.hold > 0 {
        s.hold -= 1;
        if s.hold == 0 && !s.pending_shots.is_empty() {
            for name in std::mem::take(&mut s.pending_shots) {
                screenshot(&mut f.commands, s.shot_dir.join(format!("{name}.png")), false);
            }
            s.hold = 3;
        }
        return;
    }
    if let Some(n) = s.stats_in.as_mut() {
        *n = n.saturating_sub(1);
        if *n == 0 {
            s.stats_in = None;
            s.op_stats = true;
        }
    }
    let (dt, movie_dt) = frame_clocks(&f.time, &f.real);
    if !f.level.as_ref().is_some_and(|l| l.takes_input()) && s.exiting.is_none() {
        let shots = s.host.frame(&mut ui, dt);
        if !shots.is_empty() {
            s.pending_shots = shots;
            s.hold = 2;
        }
        if !std::mem::take(&mut s.host.displays).is_empty() {
            warn!(
                "shell: --script display=…: the game's display follows its window (resize it or Alt+Enter); not taken"
            );
        }
    }
    if f.level.as_ref().is_some_and(|l| l.results_shown()) {
        s.next_segment();
    }
    if let Some(live) = f.live.as_deref_mut()
        && s.live_frame(&mut ui, f.level.as_deref(), live)
    {
        f.commands.remove_resource::<crate::shell::live::LiveSession>();
        #[cfg(feature = "net")]
        crate::net::finished(&mut f.commands);
    }
    #[cfg(feature = "net")]
    if s.link.is_some()
        && ui.fe.as_ref().is_some_and(|fe| fe.ava.hub.active)
        && !ui.views_loading()
        && !ui.name_prompt_waiting()
        && let Some(address) = s.link.take()
    {
        info!("shell: the link's server {address}: the browser asks the player");
        ui.open_link(address);
        ui.nav.raise(ssx_ui::game::fe::ava::browser::OPEN);
    }
    if s.stage == Stage::Menus
        && let Some((name, at)) = s.test_launch.clone()
        && s.host.time >= at
    {
        let screen = ui.fe.as_ref().map(|fe| fe.screen);
        let menu = !ui.nav.is_active("backgroundcontainer_on") && screen != Some(ssx_ui::game::fe::screens::id::BOOT);
        if menu && !ui.views_loading() {
            s.test_launch = None;
            let session = std::mem::take(&mut s.session_launch);
            let key = ui.fe.as_ref().and_then(|fe| fe.career.event_named(&name)).map(|e| e.key);
            let free = !session && s.test_map_mode.as_deref() == Some("freeride");
            match key {
                Some(key) => ui.fe_call(move |fe, env| {
                    ssx_ui::game::fe::progression::go_to_node(fe, key, 2);
                    if session {
                        ssx_ui::game::fe::flow::launch_session(fe, env, key);
                    } else if free {
                        ssx_ui::game::fe::flow::launch_free_ride(fe, env, key, true);
                    } else if !ssx_ui::game::fe::globe::tutorial_launch(fe, env, key) {
                        ssx_ui::game::fe::flow::launch(fe, env, key, true)
                    }
                }),
                None => error!("shell: --launch {name}: no such ui_drop"),
            }
        } else if s.host.time - at > LAUNCH_BOOT_WAIT {
            s.test_launch = None;
            error!(
                "shell: --launch {name}: the boot flow did not end within {LAUNCH_BOOT_WAIT} s (screen {screen:?}, the \
                 boot's background loaded {}, views still loading {:?}): not launched",
                ui.nav.is_active("backgroundcontainer_on"),
                ui.loading_view_names()
            );
        } else {
            s.boot_press(&mut ui, &format!("--launch {name}"));
        }
    }
    s.maps_frame(&mut ui);
    #[cfg(feature = "net")]
    if s.name_prompt_after_launch && s.test_launch.is_none() && s.test_map.is_none() {
        s.name_prompt_after_launch = false;
        ui.arm_name_prompt();
    }
    let window = f
        .render_scale
        .as_deref()
        .filter(|r| r.enabled)
        .map_or_else(|| f.windows.iter().next().map_or(Vec2::new(1280.0, 720.0), |w| w.size()), |r| r.size.as_vec2());
    s.play_movie(&mut ui, movie_dt, window, &mut f.images, &mut f.sprites, &mut f.cameras, f.audio.as_deref_mut());
    let backdrop_shown = s.movie.is_none() && f.globe.frame.as_ref().is_some_and(|g| g.visible);
    f.globe.backdrop_movie = s.backdrop.frame(backdrop_shown, movie_dt, &mut f.images);
    let fe_camera = f.fe_camera.iter().next();
    f.globe.background =
        s.play_background(movie_dt, window, &mut f.images, &mut f.sprites, &mut f.cameras, fe_camera);
    if s.stage == Stage::Level
        && let (Some(l), Some(ride)) = (f.level.as_mut(), f.ride.as_deref())
    {
        let leaving = ui.fe.as_ref().is_some_and(|fe| !fe.in_drop);
        let gamertag = ui.values.str(0x22B).to_string();
        for g in l.take_ghost(ride, &gamertag, leaving) {
            s.keep_ghost(g);
        }
    }
    s.stage_step(&mut ui, f.level.as_deref(), f.ride.as_deref(), f.live.as_deref_mut());
    let sc = &mut *f.scene;
    let t0 = Instant::now();
    if let Some(vm) = sc.vm.as_mut() {
        ui.update(vm, &mut sc.lib, &mut sc.player, dt);
    }
    sc.hidden = ui.fe.as_ref().is_some_and(ssx_ui::game::fe::FrontEnd::movie_queued);
    if let Some(fe) = ui.fe.as_ref() {
        f.fade.set(fe.screen_fade());
    }
    let fe_time = ui.fe.as_ref().map_or(0.0, |fe| fe.time);
    s.fade_shot(&mut f.commands, f.fade.alpha, fe_time);
    if let Some(l) = f.level.as_mut() {
        l.ui_cpu(t0.elapsed() + sc.last_cpu);
    }
    let presses = f.level.as_ref().map_or(0, |l| l.popup_presses());
    if presses >= 3 && !std::mem::replace(&mut s.popup_reported, true) {
        report_popup(sc);
        screenshot(&mut f.commands, s.shot_dir.join("popup_not_closed.png"), false);
    } else if presses == 0 {
        s.popup_reported = false;
    }
    let mut requests = ui.take_requests();
    if std::mem::take(&mut s.host.exit_asked) {
        requests.push(ssx_ui::game::fe::pause::GameRequest::ExitGame);
    }
    for r in requests {
        use ssx_ui::game::fe::pause::GameRequest;
        info!("shell: game request {r:?}");
        match (r, f.level.as_mut()) {
            (GameRequest::ExitGame, _) => s.ask_exit(AppExit::Success, "the menu's Exit"),
            (GameRequest::SaveSettings, _) => {
                s.save.front_end_from(&ui.values);
                s.write_save(&mut ui, "settings", f.audio.as_deref(), f.ride.as_deref());
                s.notes.push("shell: the audio settings applied".into());
                s.op_audio = true;
            }
            (GameRequest::Save, _) => {
                s.write_save(&mut ui, "the front end's save", f.audio.as_deref(), f.ride.as_deref())
            }
            (r, Some(l)) => l.request(r),
            _ => {}
        }
    }
    #[cfg(feature = "net")]
    for r in ui.take_net_requests() {
        info!("shell: the browser asks {r:?}");
        if f.live.is_some() && matches!(r, ssx_ui::game::fe::ava::browser::NetRequest::Connect(_)) {
            let reason = Some("a network session is already running".to_string());
            ui.net_progress(ssx_ui::game::fe::ava::browser::NetProgress::Ended { reason, refusal: None });
            continue;
        }
        crate::net::request(&mut f.commands, r);
    }
    let asked = ui.fe.as_mut().map(|fe| std::mem::take(&mut fe.saveload.requests)).unwrap_or_default();
    for r in asked {
        use ssx_ui::game::fe::saveload::{self, GhostsLoad, LoadError, Request};
        info!("shell: save library request {r:?}");
        match r {
            Request::Load => {
                if let Some(profile) = s.save.retry() {
                    s.op_audio |= s.save.profile_into(&mut ui, profile).is_some();
                }
                let error = s.save.damaged.then_some(LoadError::Corrupt);
                ui.fe_call(|fe, env| saveload::loaded(fe, env, error));
            }
            Request::SaveNew => {
                s.save.replace_damaged();
                s.write_save(&mut ui, "a new profile for the damaged one", f.audio.as_deref(), f.ride.as_deref());
            }
            Request::NoSaving => {
                s.save.no_saving = true;
                s.ghosts.not_loaded();
            }
            Request::LoadGhosts(load) => {
                let read = s.ghosts.reread(&s.save.schema, load == GhostsLoad::SignIn);
                info!("shell: the ghosts file read again: {read:?}");
                if load == GhostsLoad::SignIn {
                    ui.fe_call(|fe, env| saveload::ghosts_loaded(fe, env, read));
                }
            }
            Request::NewGhosts => s.new_ghosts(),
            Request::NoOwnGhost => s.own_ghost_cancelled = true,
        }
    }
    if f.level.as_mut().is_some_and(|l| l.take_save()) {
        s.write_save(&mut ui, "results", f.audio.as_deref(), f.ride.as_deref());
    }
    s.update_save_icon(&mut ui);
    let sounds = ui.take_ui_sounds();
    let posts = ui.take_ui_posts();
    let messages = ui.take_fe_messages();
    let music_requests = ui.fe.as_mut().map(|fe| std::mem::take(&mut fe.music.requests)).unwrap_or_default();
    if let Some(audio) = f.audio.as_mut() {
        use ssx_ui::game::fe::music::MusicRequest;
        for r in music_requests {
            match r {
                MusicRequest::Preview { id, key } => audio.preview(&s.big_root, id, key.as_ref(), false),
                MusicRequest::ModSong { id, key } => audio.preview(&s.big_root, id, key.as_ref(), true),
                MusicRequest::StopPreview => audio.stop_preview(),
            }
        }
        audio.set_level_loading(s.loading.is_some());
        if let Some(fe) = ui.fe.as_mut() {
            fe.music.controller_playing = audio.preview_playing();
            if fe.music.take_switches_changed() {
                audio.set_song_switches(&fe.music.switches());
            }
        }
    }
    if let Some(audio) = f.audio.as_mut() {
        let boost = ui.values.float(ssx_ui::game::values::id::BOOST_PERCENT);
        for id in sounds {
            audio.ui_sfx(id, boost);
        }
        let now_ms = s.host.time * 1000.0;
        for p in posts {
            audio.ui_post(p.id, p.delay_ms, now_ms);
        }
        use ssx_ui::game::fe::FeMessage;
        for m in messages {
            match m {
                FeMessage::HintShown(kind) => audio.hint_shown(kind),
                FeMessage::HintClosed => audio.hint_closed(),
                FeMessage::PopupState(v) => audio.popup_state(v),
                FeMessage::PauseMenuAudio { opened, leaving, closed } => {
                    audio.pause_menu_event(opened, leaving, closed)
                }
                FeMessage::BootFlowLoaded => audio.boot_flow_loaded(),
                FeMessage::LoadoutRefresh => audio.launch_requested(),
            }
        }
    }
    if let (Some(audio), Some(fe)) = (f.audio.as_mut(), ui.fe.as_ref()) {
        let kind = s.launch.as_ref().map_or(1, |l| ssx_assets::sfx_game::event_kind(l.event_type as u32));
        let nis = f.ride.as_deref().map_or(0, game_level::nis_state);
        audio.set_setting_67(ui.values.bool(0x67));
        audio.frame(fe.screen, kind, ui.values.bool(0x30), nis, ui.fe_audio_wind(), s.host.time * 1000.0);
        audio.watch_ui(|id| ui.values.bool(id));
        audio.results(ui.values.bool(0x1ED), ui.values.bool(0x1D7));
    }
    if let Some(audio) = f.audio.as_ref() {
        let msg = audio.state();
        let beat = audio.beat().map(|b| ssx_ui::game::fe::globe::audio::Beat {
            beat: b.beat,
            to_beat: b.to_beat,
            bar: b.bar,
            to_bar: b.to_bar,
        });
        ui.fe_audio_message(msg.screen, msg.state, beat);
    }
    let presses = s.host.log[s.log_seen.1.min(s.host.log.len())..].iter().filter(|l| l.contains("host: press")).count();
    if presses > 0 && s.input_log {
        s.notes.push(format!(
            "shell: input at {:.2} s: views loading {:?}; nav {:?}; {}",
            s.host.time,
            ui.loading_view_names(),
            ui.nav.active_leaves(),
            input_report(sc)
        ));
    }
    s.print_logs(&ui);
    let t = s.host.time;
    let results_held = s.max_rides.is_some_and(|n| s.rides >= n)
        && s.stage == Stage::Level
        && !s.follow_prompts
        && s.segment_started
        && s.segments.is_empty()
        && s.host.script.is_empty()
        && f.level.as_ref().is_some_and(|l| l.results_shown() && l.test_run_done() && !l.ghost_pending());
    if results_held && s.done_at.is_none() {
        info!("shell: level {}'s results are up and nothing scripted leaves them: the run ends", s.rides);
    }
    let rides_over = s.max_rides.is_some_and(|n| s.rides_done >= n) && s.stage == Stage::Menus || results_held;
    if s.exiting.is_some() {
    } else if !rides_over && !s.seconds.is_some_and(|x| t >= x) {
        s.done_at = None;
    } else {
        let at = *s.done_at.get_or_insert(t);
        let pending = s.host.script.first().is_some_and(|(due, _)| *due <= at + 5.0);
        if (t - at >= 2.0 && !pending) || t - at >= 5.0 || s.seconds.is_some_and(|x| t >= x) {
            info!("shell: the scripted run ends at {t:.2} s (levels played {})", s.rides_done);
            let check = &*f.world_check;
            if check.covered.is_empty() {
                s.ask_exit(AppExit::Success, "the scripted run's end");
            } else {
                error!(
                    "world check: the level's world was covered in {} of {} ride screenshots: {}",
                    check.covered.len(),
                    check.shots,
                    check.covered.join(", ")
                );
                s.ask_exit(AppExit::error(), "the scripted run's end");
            }
        }
    }
}

fn input_report(sc: &mut ui::UiScene) -> String {
    const VIEW: &str = "_level0.instance1.mc10.mView";
    let Some(vm) = sc.vm.as_mut() else { return String::new() };
    let mut cx = vm.ctx(&mut sc.lib, &mut sc.player);
    let text = |cx: &mut ssx_ui::avm1::Ctx, path: &str| {
        let v = ssx_ui::game::resolve_path(cx, path);
        v.and_then(|v| cx.to_string(&v).ok()).map_or_else(|| "?".to_string(), |s| s.to_string())
    };
    let input = format!("{VIEW}.mInputManager");
    let mut out = Vec::new();
    for m in ["mMinPriority", "mInputLockManager.mLockCount", "mInputLockManager.mAllInputLocked"] {
        out.push(format!("{m} = {}", text(&mut cx, &format!("{input}.{m}"))));
    }
    if let Some(locks) = ssx_ui::game::resolve_path(&mut cx, &format!("{input}.mInputLockManager.mLocks")) {
        let labels: Vec<String> = cx.enumerate(&locks).iter().map(|k| k.to_string()).collect();
        out.push(format!("locks {labels:?}"));
    }
    let n = text(&mut cx, &format!("{input}.mListenerLists.length")).parse::<usize>().unwrap_or(0);
    for i in 0..n.min(32) {
        let at = format!("{input}.mListenerLists.{i}");
        let (priority, who) = (text(&mut cx, &format!("{at}.priority")), text(&mut cx, &format!("{at}.listener")));
        out.push(format!("listener {i}: priority {priority} {who}"));
    }
    if let Some(root) = ssx_ui::game::resolve_path(&mut cx, VIEW) {
        let layers: Vec<String> =
            cx.enumerate(&root).iter().map(|k| k.to_string()).filter(|k| k.starts_with("ssx_")).collect();
        for layer in layers {
            let Some(clip) = ssx_ui::game::resolve_path(&mut cx, &format!("{VIEW}.{layer}")) else { continue };
            let views: Vec<String> = cx.enumerate(&clip).iter().map(|k| k.to_string()).collect();
            if !views.is_empty() {
                out.push(format!("{layer}: {views:?}"));
            }
        }
    }
    out.join("; ")
}

fn report_popup(sc: &mut ui::UiScene) {
    const VIEW: &str = "_level0.instance1.mc10.mView.ssx_tutorial.tutorialpausemenu.mView.mcTutorialPopup";
    let Some(vm) = sc.vm.as_mut() else { return };
    let mut traces: Vec<String> =
        vm.trace_log.iter().rev().filter(|l| l.contains("Tutorial_Popup")).take(6).cloned().collect();
    traces.reverse();
    let mut cx = vm.ctx(&mut sc.lib, &mut sc.player);
    let mut state = Vec::new();
    let members = [
        "mIsPopupEnabled",
        "mIsPopupVisible",
        "mHasAnimatedIn",
        "_visible",
        "mcUnblurred._currentframe",
        "mcUnblurred._totalframes",
        "mcUnblurred._visible",
    ];
    for m in members {
        let v = ssx_ui::game::resolve_path(&mut cx, &format!("{VIEW}.{m}"));
        let text = v.and_then(|v| cx.to_string(&v).ok()).map_or_else(|| "?".to_string(), |s| s.to_string());
        state.push(format!("{m} = {text}"));
    }
    const INPUT: &str = "_level0.instance1.mc10.mView.mInputManager";
    let text = |cx: &mut ssx_ui::avm1::Ctx, path: &str| {
        let v = ssx_ui::game::resolve_path(cx, path);
        v.and_then(|v| cx.to_string(&v).ok()).map_or_else(|| "?".to_string(), |s| s.to_string())
    };
    let mut input = Vec::new();
    for m in ["mMinPriority", "mInputLockManager.mLockCount", "mInputLockManager.mAllInputLocked"] {
        input.push(format!("{m} = {}", text(&mut cx, &format!("{INPUT}.{m}"))));
    }
    if let Some(locks) = ssx_ui::game::resolve_path(&mut cx, &format!("{INPUT}.mInputLockManager.mLocks")) {
        let labels: Vec<String> = cx.enumerate(&locks).iter().map(|k| k.to_string()).collect();
        input.push(format!("locks {labels:?}"));
    }
    let n = text(&mut cx, &format!("{INPUT}.mListenerLists.length")).parse::<usize>().unwrap_or(0);
    for i in 0..n.min(32) {
        let at = format!("{INPUT}.mListenerLists.{i}");
        let (priority, who) = (text(&mut cx, &format!("{at}.priority")), text(&mut cx, &format!("{at}.listener")));
        input.push(format!("listener {i}: priority {priority} {who}"));
    }
    let list_path = format!("{VIEW}.mcUnblurred.mcCallout.mcListAnim.mcList");
    let mut list = Vec::new();
    if let Some(l) = ssx_ui::game::resolve_path(&mut cx, &list_path) {
        let names: Vec<String> = cx.enumerate(&l).iter().map(|k| k.to_string()).collect();
        for n in names.iter().filter(|n| n.starts_with('m') && !n.starts_with("mc")).take(40) {
            list.push(format!("{n} = {}", text(&mut cx, &format!("{list_path}.{n}"))));
        }
    }
    error!(
        "shell: the tutorial popup did not close on A (autoplay, 3 presses): {}; its last traces: {traces:?}; the \
         input manager: {}; the list: {}",
        state.join(", "),
        input.join("; "),
        list.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_ui::game::fe::ava::keyboard::TextInput as T;

    #[test]
    fn keys_become_text() {
        assert_eq!(key_text(KeyCode::KeyA, Some("a")), [T::Char('a')]);
        assert_eq!(key_text(KeyCode::KeyA, Some("A")), [T::Char('A')]);
        assert_eq!(key_text(KeyCode::Backspace, Some("\u{8}")), [T::Backspace]);
        assert_eq!(key_text(KeyCode::Enter, Some("\r")), [T::Enter]);
        assert_eq!(key_text(KeyCode::Tab, Some("\t")), []);
        assert_eq!(key_text(KeyCode::ShiftLeft, None), []);
        assert_eq!(key_text(KeyCode::Digit2, Some("\u{e9}")), [T::Char('\u{e9}')]);
    }

    #[test]
    fn a_slow_frame_counts_in_full_for_the_movies() {
        let mut time = Time::<()>::default();
        let mut real = Time::<Real>::default();
        real.update_with_duration(std::time::Duration::ZERO);
        for ms in [16, 800, 33] {
            let d = std::time::Duration::from_millis(ms);
            time.advance_by(d);
            real.update_with_duration(d);
            let (fe, movie) = frame_clocks(&time, &real);
            assert_eq!(fe, (ms as f64 / 1000.0).min(0.1));
            assert_eq!(movie, ms as f64 / 1000.0);
        }
    }
}
