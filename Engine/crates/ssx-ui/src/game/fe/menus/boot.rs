use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{FeEnv, FrontEnd, MovieRequest, screens};

pub const ATTRACT_SECONDS: f64 = 30.0;
pub const BOOT_MOVIE: &str = "boot_movie_eng";
pub const ATTRACT_MOVIE: &str = "intromovie";

const BOOT_LOGO_STARTED: u16 = 0x2F;
const BOOT_FLOW_STARTED: u16 = 0x30;
const BOOT_SKIP_INTRO: u16 = 0x31;
const BACKGROUND_TRANSITION: u16 = 0x2E;

#[derive(Clone, Debug)]
pub struct BootFlow {
    pub attract_off: bool,
    pub attract: f64,
    pub play_intro: bool,
    pub handle_input: bool,
    pub signed_in: bool,
}

impl BootFlow {
    pub fn new() -> Self {
        BootFlow {
            attract_off: true,
            attract: ATTRACT_SECONDS,
            play_intro: false,
            handle_input: false,
            signed_in: false,
        }
    }
}

impl Default for BootFlow {
    fn default() -> Self {
        Self::new()
    }
}

pub fn activate(fe: &mut FrontEnd, env: &mut FeEnv) {
    fe.screen = screens::id::BOOT;
    if !env.values.bool(BOOT_LOGO_STARTED) {
        env.values.set_bool(BOOT_LOGO_STARTED, true);
        crate::game::fe::settings::init_values(env.values, &fe.audio_store);
        if !env.values.bool(BOOT_SKIP_INTRO) {
            fe.queue_movie(MovieRequest { can_stop: false, ..MovieRequest::new(BOOT_MOVIE, "boot") });
            fe.note(format!("FLOW_PlayMovie {BOOT_MOVIE}"));
            fe.boot.play_intro = false;
            fe.boot.handle_input = false;
        } else {
            fe.boot.play_intro = true;
            fe.boot.handle_input = true;
            fe.boot.attract_off = false;
        }
        fe.publish.push("dp_boot_logo_screen_data".into());
    }
    env.nav_events.push("backgroundcontainer_enable".into());
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv, dt: f64) {
    if fe.boot.attract_off || env.values.bool(BOOT_FLOW_STARTED) {
        return;
    }
    fe.boot.attract -= dt;
    if fe.boot.attract - dt <= 0.0 {
        fe.queue_movie(MovieRequest::new(ATTRACT_MOVIE, "attract"));
        fe.note(format!("FLOW_PlayMovie {ATTRACT_MOVIE} (attract)"));
        fe.boot.attract_off = true;
        fe.boot.handle_input = false;
        fe.boot.play_intro = false;
        fe.publish.push("dp_boot_logo_screen_data".into());
    }
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: f64) {
    match action {
        "A_BootStart" => {
            // TODO(guess): a port choice — a start without a device (the boot logo's own button `mcNextButton` clicked with the pointer: `OnStartPressed` gets no argument, the parameter is NaN, which the game's conversion makes controller 0) signs in the PC's keyboard and pointer (device 4), so that no absent pad 0 is taken for the player's; settle: none (the 360 has no pointer: its mouse device is never connected).
            let device = if param.is_nan() { i32::from(crate::game::input::KEYBOARD_DEVICE) } else { param as i32 };
            start(fe, env, device);
            env.values.set_int(BACKGROUND_TRANSITION, 1);
            env.values.set_bool(BOOT_FLOW_STARTED, true);
        }
        "A_GameMovie" => {
            fe.boot.play_intro = true;
            fe.boot.attract_off = false;
            fe.boot.handle_input = true;
            fe.boot.attract = ATTRACT_SECONDS;
            fe.publish.push("dp_boot_logo_screen_data".into());
        }
        _ => {}
    }
}

fn start(fe: &mut FrontEnd, env: &mut FeEnv, device: i32) {
    if env.values.bool(BOOT_FLOW_STARTED) {
        return;
    }
    env.values.set_int(0x229, device);
    env.values.set_bool(0x22A, true);
    fe.note(format!("message 0xDD9DA034; controller {device}"));
    // TODO(guess): the Xbox Live / profile sign-in (0x8287CBF8: `profile_signin`, a popup) is not ported, the local profile counts as signed in; settle: none (no Live in the port).
    fe.boot.signed_in = true;
    fe.note("FE manager event 10 (message 0xBA30389A)");
    crate::game::fe::flow::signed_in(fe, env, device);
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx) -> Value {
    let o = Value::Object(cx.new_object());
    let _ = cx.set_member(&o, "bShouldPlayScreenIntro", Value::Bool(fe.boot.play_intro));
    let _ = cx.set_member(&o, "bShouldScreenHandleInput", Value::Bool(fe.boot.handle_input));
    o
}
