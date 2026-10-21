use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use ssx_ui::game::input::Button;

use super::{
    GameUiRes, LAUNCH_BOOT_WAIT, PadSlots, Shell, Stage, UiPipelinesReady, WorldCheck, pad_presses, screenshot,
};
use crate::audio::music::frontend::FrontEndAudio;
use crate::shell::game_level;
use crate::ui;

pub(super) fn shell_input(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<(Entity, &Gamepad)>,
    slots: Res<PadSlots>,
    mut shell: ResMut<Shell>,
    mut ui: ResMut<GameUiRes>,
    level: Option<Res<game_level::LevelGame>>,
) {
    let s = &mut *shell;
    if !s.started || level.is_some_and(|l| l.takes_input()) {
        return;
    }
    for (b, device) in pad_presses(&keys, &pads, &slots, &mut s.stick_dpad) {
        s.host.press_from(&mut ui, b, device);
    }
    let mut stick = Vec2::ZERO;
    for (_, pad) in &pads {
        stick +=
            Vec2::new(pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0), pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0));
    }
    let key = |k: KeyCode| if keys.pressed(k) { 1.0 } else { 0.0 };
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

#[derive(SystemParam)]
pub(super) struct Frame<'w, 's> {
    commands: Commands<'w, 's>,
    time: Res<'w, Time>,
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
}

pub(super) fn shell_step(mut shell: ResMut<Shell>, mut ui: ResMut<GameUiRes>, mut f: Frame) {
    let s = &mut *shell;
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
        f.globe.frame = match s.stage {
            Stage::Level => None,
            _ => ui.fe.as_ref().and_then(|fe| fe.globe_frame(aspect)),
        };
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
    let dt = f.time.delta_secs_f64().min(0.1);
    if !f.level.as_ref().is_some_and(|l| l.takes_input()) {
        let shots = s.host.frame(&mut ui, dt);
        if !shots.is_empty() {
            s.pending_shots = shots;
            s.hold = 2;
        }
    }
    if f.level.as_ref().is_some_and(|l| l.results_shown()) {
        s.next_segment();
    }
    if s.stage == Stage::Menus
        && let Some((name, at)) = s.test_launch.clone()
        && s.host.time >= at
    {
        let screen = ui.fe.as_ref().map(|fe| fe.screen);
        let movie = ui.fe.as_ref().is_some_and(|fe| fe.movie.is_some());
        let menu = !ui.nav.is_active("backgroundcontainer_on") && screen != Some(ssx_ui::game::fe::screens::id::BOOT);
        if menu && !ui.views_loading() {
            s.test_launch = None;
            let key = ui.fe.as_ref().and_then(|fe| fe.career.event_named(&name)).map(|e| e.key);
            match key {
                Some(key) => ui.fe_call(|fe, env| {
                    if !ssx_ui::game::fe::globe::tutorial_launch(fe, env, key) {
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
        } else if screen == Some(ssx_ui::game::fe::screens::id::BOOT)
            && !movie
            && s.host.time >= s.launch_start_at
            && ui.fe.as_ref().is_some_and(|fe| fe.saveload.dialog == Some(0xC))
        {
            info!("shell: --launch {name}: A on the save library's autosave notice");
            s.host.press(&mut ui, Button::DigitalOne);
            s.launch_start_at = s.host.time + 1.0;
        } else if screen == Some(ssx_ui::game::fe::screens::id::BOOT) && !movie && s.host.time >= s.launch_start_at {
            info!("shell: --launch {name}: START on the boot screen first (the boot flow ends on the main menu)");
            s.host.press(&mut ui, Button::Start);
            s.launch_start_at = s.host.time + 1.0;
        }
    }
    let window = f.windows.iter().next().map_or(Vec2::new(1280.0, 720.0), |w| w.size());
    s.play_movie(&mut ui, dt, window, &mut f.images, &mut f.sprites, &mut f.cameras, f.audio.as_deref_mut());
    s.stage_step(&mut ui, f.level.as_deref(), f.ride.as_deref());
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
    for r in ui.take_requests() {
        use ssx_ui::game::fe::pause::GameRequest;
        info!("shell: game request {r:?}");
        match (r, f.level.as_mut()) {
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
    let asked = ui.fe.as_mut().map(|fe| std::mem::take(&mut fe.saveload.requests)).unwrap_or_default();
    for r in asked {
        use ssx_ui::game::fe::saveload::{self, LoadError, Request};
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
            Request::NoSaving => s.save.no_saving = true,
        }
    }
    if f.level.as_mut().is_some_and(|l| l.take_save()) {
        s.write_save(&mut ui, "results", f.audio.as_deref(), f.ride.as_deref());
    }
    let sounds = ui.take_ui_sounds();
    let messages = ui.take_fe_messages();
    let music_requests = ui.fe.as_mut().map(|fe| std::mem::take(&mut fe.music.requests)).unwrap_or_default();
    if let Some(audio) = f.audio.as_mut() {
        use ssx_ui::game::fe::music::MusicRequest;
        for r in music_requests {
            match r {
                MusicRequest::Preview(id) => audio.preview(&s.big_root, id),
                MusicRequest::StopPreview => audio.stop_preview(),
            }
        }
        if let Some(fe) = ui.fe.as_mut() {
            fe.music.controller_playing = audio.preview_playing();
        }
    }
    if let Some(audio) = f.audio.as_mut() {
        let boost = ui.values.float(ssx_ui::game::values::id::BOOST_PERCENT);
        for id in sounds {
            audio.ui_sfx(id, boost);
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
        audio.frame(fe.screen, kind, ui.values.bool(0x30), nis);
        audio.watch_ui(|id| ui.values.bool(id));
        audio.results(ui.values.bool(0x1ED), ui.values.bool(0x1D7));
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
    let rides_over = s.max_rides.is_some_and(|n| s.rides_done >= n) && s.stage == Stage::Menus;
    if rides_over || s.seconds.is_some_and(|x| t >= x) {
        let at = *s.done_at.get_or_insert(t);
        let pending = s.host.script.first().is_some_and(|(due, _)| *due <= at + 5.0);
        if (t - at >= 2.0 && !pending) || t - at >= 5.0 || s.seconds.is_some_and(|x| t >= x) {
            info!("shell: the scripted run ends at {t:.2} s (levels played {})", s.rides_done);
            let check = &*f.world_check;
            if check.covered.is_empty() {
                f.exit.write(AppExit::Success);
            } else {
                error!(
                    "world check: the level's world was covered in {} of {} ride screenshots: {}",
                    check.covered.len(),
                    check.shots,
                    check.covered.join(", ")
                );
                f.exit.write(AppExit::error());
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
