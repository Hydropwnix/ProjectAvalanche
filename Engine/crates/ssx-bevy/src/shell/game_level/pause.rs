use bevy::prelude::*;
use ssx_ui::game::GameUi;

use super::LevelGame;
use crate::audio::music::frontend::FrontEndAudio;
use crate::ride::Ride;
use crate::shell::game_mode::GameUiRes;

pub(super) fn pause_now(l: &mut LevelGame, ride: &mut Ride, ui: &mut GameUi, audio: Option<&mut FrontEndAudio>) {
    let Some(fe) = ui.fe.as_mut() else { return };
    let Some(paused) = fe.pause_step() else { return };
    let sound = fe.game_pause.audio_event();
    l.paused = paused;
    if paused {
        ride.pause();
    } else {
        ride.resume();
    }
    if let Some(a) = audio.filter(|_| !paused || sound.is_some()) {
        a.game_pause_event(paused, sound == Some(true));
    }
    info!(
        "level: the game {} at ride time {:.2} s, UI frame {}{}",
        if paused { "pauses" } else { "runs again" },
        ride.seconds(),
        l.ui_frames,
        if sound == Some(true) { " (a tutorial popup)" } else { "" }
    );
}

pub fn pause_frame(
    level: Option<ResMut<LevelGame>>,
    ride: Option<ResMut<Ride>>,
    mut ui: ResMut<GameUiRes>,
    mut audio: Option<ResMut<FrontEndAudio>>,
) {
    let by_ride = match (level, ride) {
        (Some(mut level), Some(mut ride)) => {
            pause_now(&mut level, &mut ride, &mut ui.0, audio.as_deref_mut());
            level.up && !ride.hold && !level.paused
        }
        _ => {
            _ = ui.0.fe.as_mut().and_then(|fe| fe.pause_step());
            false
        }
    };
    if let Some(fe) = ui.0.fe.as_mut() {
        fe.fader_by_ride = by_ride;
    }
}
