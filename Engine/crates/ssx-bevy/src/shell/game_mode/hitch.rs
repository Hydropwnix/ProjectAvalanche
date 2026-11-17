//! The hitch log: a frame that takes much longer than the ones around it is written to the log with where the game
//! was and what was going on then (shaders being built, the rider showcase's riders loading or arriving), so a
//! stutter can be traced afterwards.

use std::sync::atomic::Ordering;

use bevy::prelude::{Res, ResMut, Resource, Time, info};
use bevy::time::Real;

use super::showcase::{SPAWNED_LAST_FRAME, ShowcaseState};
use super::{GameUiRes, Shell, Stage};

/// A frame this long (seconds) or longer is a hitch.
const HITCH_SECS: f32 = 0.05;
/// The game's first seconds (the window and the first screens being made): not logged.
const START_SECS: f64 = 3.0;
/// At most this many hitches logged a minute (a long stall is a run of them).
const PER_MINUTE: u32 = 30;

/// Pipelines (shaders) the GPU is still building, as the render world saw them last frame.
#[derive(Resource, Default)]
pub struct PipelinesWaiting(pub usize);

#[derive(Resource, Default)]
pub(super) struct HitchLog {
    minute: f64,
    logged: u32,
    stage: Option<Stage>,
    stage_since: f64,
}

pub(super) fn hitch_log(
    mut log: ResMut<HitchLog>,
    time: Res<Time<Real>>,
    shell: Res<Shell>,
    ui: Res<GameUiRes>,
    showcase: Res<ShowcaseState>,
    pipelines: Res<PipelinesWaiting>,
) {
    let spawned = SPAWNED_LAST_FRAME.swap(0, Ordering::Relaxed);
    let now = time.elapsed_secs_f64();
    if log.stage != Some(shell.stage) {
        log.stage = Some(shell.stage);
        log.stage_since = now;
    }
    let dt = time.delta_secs();
    if dt < HITCH_SECS || now < START_SECS {
        return;
    }
    if now - log.minute >= 60.0 {
        log.minute = now;
        log.logged = 0;
    }
    if log.logged >= PER_MINUTE {
        return;
    }
    log.logged += 1;
    let screen = ui.0.fe.as_ref().map_or(0, |fe| fe.screen);
    info!(
        "hitch: a {:.0} ms frame ({:?} for {:.1} s, screen {screen:#x}); shaders building {}, showcase riders loading {}, \
         put in the world {spawned}",
        dt * 1000.0,
        shell.stage,
        now - log.stage_since,
        pipelines.0,
        showcase.loading(),
    );
}
