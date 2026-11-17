use std::collections::BTreeSet;
use std::time::Instant;

use bevy::ecs::system::SystemParam;
use bevy::input::gamepad::Gamepad;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseButtonInput, MouseWheel};
use bevy::prelude::{
    AppExit, ButtonInput, KeyCode, MessageWriter, Messages, MouseButton, Query, Res, ResMut, Resource, error, info,
};
use bevy::window::Ime;
use ssx_ui::game::GameUi;

use super::Shell;
use super::display::DisplayState;

// PORT(choice): an exit asked while a writer still writes (the profile's, the ghosts', `settings.json`'s, the server
// lists') waits for it with the frames going on, the dock's "Saving" icon up (the save library's 0x10AD2001 …
// 0x10AD1001 form, +0xA4 `TXT_SAVING`) and no input taken, then the app ends; before (main 407047ec) the exit joined
// the writers on the frame's thread and the window stood still on its last frame (39.2 s once under another program's
// 3 GB write, btrfs). The console has no exit of its own: the game runs until the dashboard ends it, and its saves
// are jobs the frame loop runs over frames with the icon up (`FlowAutoSave` 0x83173E60).
pub(super) struct Exiting {
    asked: Instant,
    code: AppExit,
    why: &'static str,
    display_kept: bool,
    said: bool,
    done: bool,
}

#[derive(Resource, Default)]
pub(crate) struct ExitWait {
    asked: bool,
    holding: BTreeSet<&'static str>,
}

impl ExitWait {
    pub(crate) fn ask(&mut self) -> bool {
        !std::mem::replace(&mut self.asked, true)
    }

    pub(crate) fn holding(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.holding.iter().copied()
    }

    #[cfg(feature = "scripts")]
    pub(crate) fn asked(&self) -> bool {
        self.asked
    }

    #[cfg(feature = "scripts")]
    pub(crate) fn hold(&mut self, what: &'static str) {
        self.holding.insert(what);
    }

    #[cfg(feature = "scripts")]
    pub(crate) fn release(&mut self, what: &'static str) {
        self.holding.remove(what);
    }
}

impl Shell {
    pub(super) fn ask_exit(&mut self, code: AppExit, why: &'static str) {
        if let Some(x) = &self.exiting {
            if !x.done {
                info!("shell: exit ({why}) asked while the exit ({}) waits for the writers", x.why);
            }
            return;
        }
        info!("shell: exit asked ({why})");
        self.exiting =
            Some(Exiting { asked: Instant::now(), code, why, display_kept: false, said: false, done: false });
    }

    pub(super) fn exit_written(&self) -> bool {
        self.exiting.as_ref().is_some_and(|x| x.done)
    }

    pub(super) fn exit_frame(
        &mut self,
        ui: &GameUi,
        mut display: Option<&mut DisplayState>,
        parts: &mut ExitWait,
        exit: &mut MessageWriter<AppExit>,
    ) {
        let Some(x) = self.exiting.as_mut() else { return };
        if x.done {
            return;
        }
        if !std::mem::replace(&mut x.display_kept, true)
            && let Some(d) = display.as_deref_mut()
        {
            d.keep_at_exit();
        }
        if parts.ask() {
            self.exit_writing = true;
            return;
        }
        let mut waiting: Vec<&str> = [
            ("the profile", self.save.writing()),
            ("the ghosts", self.ghosts.writing()),
            ("the window's state", display.as_deref().is_some_and(DisplayState::writing)),
            ("the port's settings and server lists", ui.fe.as_ref().is_some_and(|fe| fe.files_writing())),
        ]
        .into_iter()
        .filter_map(|(what, busy)| busy.then_some(what))
        .collect();
        waiting.extend(parts.holding());
        self.exit_writing = !waiting.is_empty();
        if !waiting.is_empty() {
            if !std::mem::replace(&mut x.said, true) {
                info!(
                    "shell: Exit: waiting for the writers of {} (the frames go on, the Saving icon up, no input taken)",
                    waiting.join(", ")
                );
            }
            return;
        }
        x.done = true;
        let (why, waited, code) = (x.why, x.asked.elapsed(), x.code.clone());
        info!(
            "shell: Exit: the game closes (the profile {}) — {why}, {} ms after it was asked",
            self.save.describe(),
            waited.as_millis()
        );
        if code != AppExit::Success {
            error!("shell: the app ends with {code:?}");
        }
        exit.write(code);
    }
}

#[derive(SystemParam)]
pub(super) struct Devices<'w, 's> {
    keys: ResMut<'w, ButtonInput<KeyCode>>,
    mouse: ResMut<'w, ButtonInput<MouseButton>>,
    pads: Query<'w, 's, &'static mut Gamepad>,
    typed: ResMut<'w, Messages<KeyboardInput>>,
    ime: ResMut<'w, Messages<Ime>>,
    clicks: ResMut<'w, Messages<MouseButtonInput>>,
    wheel: ResMut<'w, Messages<MouseWheel>>,
}

pub(super) fn exit_input(shell: Res<Shell>, mut d: Devices) {
    if shell.exiting.is_none() {
        return;
    }
    d.keys.reset_all();
    d.mouse.reset_all();
    for mut pad in &mut d.pads {
        pad.digital_mut().reset_all();
        let axes: Vec<_> = pad.analog().all_axes().copied().collect();
        for axis in axes {
            pad.analog_mut().set(axis, 0.0);
        }
    }
    d.typed.clear();
    d.ime.clear();
    d.clicks.clear();
    d.wheel.clear();
}
