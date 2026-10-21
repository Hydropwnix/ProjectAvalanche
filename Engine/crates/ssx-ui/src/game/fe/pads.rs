use super::{FeEnv, FrontEnd, pause, screens};
use crate::game::input::PADS;

pub const USER_CONTROLLER_ID: u16 = 0x229;
pub const USER_CONTROLLER_SELECTED: u16 = 0x22A;
pub const PAUSE_IS_IN_SYS_MENU: u16 = 0x1B8;

#[derive(Clone, Debug)]
pub struct Pads {
    pub connected: [bool; PADS],
    pub active: Option<i32>,
    pub lost: bool,
    pub system_ui: bool,
    pub disconnect_updates: u32,
}

impl Default for Pads {
    fn default() -> Self {
        Pads { connected: [true; PADS], active: None, lost: false, system_ui: false, disconnect_updates: 0 }
    }
}

impl Pads {
    pub fn active_pad(&self) -> Option<usize> {
        self.active.and_then(|d| usize::try_from(d).ok()).filter(|&d| d < PADS)
    }

    pub fn read(&mut self, pad: usize, connected: bool) -> Option<bool> {
        let was = self.connected.get_mut(pad)?;
        (*was != connected).then(|| {
            *was = connected;
            connected
        })
    }

    pub fn disconnect_device(&self, user: i32) -> Option<u8> {
        let pad = usize::try_from(user).ok().filter(|&p| p < PADS)?;
        let filtered = self.active.is_some_and(|a| a != user);
        (!self.connected[pad] && !filtered).then_some(pad as u8)
    }

    pub fn poll_change(&self, selected: bool, user: i32) -> Option<bool> {
        if !selected || self.system_ui {
            return None;
        }
        let pad = usize::try_from(user).ok().filter(|&p| p < PADS);
        let lost = pad.is_some_and(|p| !self.connected[p]);
        (lost != self.lost).then_some(lost)
    }
}

pub fn sign_in(fe: &mut FrontEnd, device: i32) {
    fe.pads.active = Some(device);
    fe.note(format!("the UI manager's and the game's input read device {device} only (0x82575770, 0x82BD6718)"));
}

pub fn connection(fe: &mut FrontEnd, env: &mut FeEnv, pad: usize, connected: bool) {
    if fe.pads.read(pad, connected).is_none() {
        return;
    }
    fe.note(format!("message 0x9304AE5D {{pad {pad}, connected {connected}}}"));
    if !connected && env.values.int(USER_CONTROLLER_ID) == pad as i32 {
        fe.note("the player's pad: view event ControllerPull (0x82407758)");
        fe.game_actions.push(("ControllerPull".into(), super::ActionParam::default()));
    }
}

pub fn disconnect_input(fe: &mut FrontEnd, values: &crate::game::values::UiValues) -> Option<u8> {
    let device = fe.pads.disconnect_device(values.int(USER_CONTROLLER_ID));
    match (device, fe.pads.disconnect_updates) {
        (Some(d), 0) => fe.note(format!("the pad handler posts DISCONNECT (device {d}) every update")),
        (None, n) if n > 0 => fe.note(format!("the pad handler's DISCONNECT ends after {n} updates")),
        _ => {}
    }
    fe.pads.disconnect_updates = if device.is_some() { fe.pads.disconnect_updates + 1 } else { 0 };
    device
}

pub fn poll(fe: &mut FrontEnd, env: &mut FeEnv) {
    if let Some(lost) = poll_change(fe, env.values) {
        fe.pads.lost = lost;
        controller_message(fe, env, lost);
    }
}

pub fn poll_change(fe: &FrontEnd, values: &crate::game::values::UiValues) -> Option<bool> {
    fe.pads.poll_change(values.bool(USER_CONTROLLER_SELECTED), values.int(USER_CONTROLLER_ID))
}

fn controller_message(fe: &mut FrontEnd, env: &mut FeEnv, lost: bool) {
    fe.note(format!(
        "message 0x748C6DE4 {{{}}}: the player's pad {}",
        u8::from(lost),
        if lost { "lost" } else { "back" }
    ));
    if fe.simulation {
        fe.game_pause.set(pause::reason::PAD_LOST, lost);
    }
    if lost && fe.screen == screens::id::HUD {
        pause::pause_menu(fe, env);
    }
}

pub fn system_ui(fe: &mut FrontEnd, env: &mut FeEnv, shown: bool) {
    if fe.pads.system_ui == shown {
        return;
    }
    fe.pads.system_ui = shown;
    env.values.set_bool(PAUSE_IS_IN_SYS_MENU, shown);
    fe.note(format!("XN_SYS_UI {{{}}}: UI 0x1B8, message 0x302B4E17", u8::from(shown)));
    if fe.simulation {
        fe.game_pause.system_ui_message(shown);
    }
    if shown && fe.screen == screens::id::HUD {
        pause::pause_menu(fe, env);
    }
}

pub fn hud_ready(fe: &mut FrontEnd, env: &mut FeEnv) {
    if fe.simulation && fe.game_pause.hud_ready(env.values.bool(PAUSE_IS_IN_SYS_MENU)) {
        fe.note("0xF19EE57A with UI 0x1B8 set: pause reason 3");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_player_pads_loss_is_posted_once_each_way_after_the_selection() {
        let mut p = Pads::default();
        p.connected[1] = false;
        assert_eq!(p.poll_change(false, 1), None);
        assert_eq!(p.poll_change(true, 1), Some(true));
        p.lost = true;
        assert_eq!(p.poll_change(true, 1), None);
        p.connected[1] = true;
        p.system_ui = true;
        assert_eq!(p.poll_change(true, 1), None);
        p.system_ui = false;
        assert_eq!(p.poll_change(true, 1), Some(false));
        let k = Pads { connected: [false; PADS], ..Default::default() };
        assert_eq!(k.poll_change(true, 4), None);
    }

    #[test]
    fn disconnect_is_the_player_pads_and_passes_the_sign_ins_filter_only_for_it() {
        let mut p = Pads::default();
        assert_eq!(p.disconnect_device(0), None);
        assert_eq!(p.read(0, false), Some(false));
        assert_eq!(p.read(0, false), None);
        assert_eq!(p.disconnect_device(0), Some(0));
        assert_eq!(p.disconnect_device(2), None);
        p.active = Some(2);
        assert_eq!(p.disconnect_device(0), None);
        p.read(2, false);
        assert_eq!(p.disconnect_device(2), Some(2));
        assert_eq!(p.active_pad(), Some(2));
        p.active = Some(4);
        assert_eq!(p.active_pad(), None);
    }

    #[test]
    fn the_system_uis_reason_ignores_an_open_while_active() {
        let mut m = pause::PauseManager::default();
        m.system_ui_message(true);
        assert_eq!(m.update(), Some(true));
        m.system_ui_message(false);
        m.system_ui_message(true);
        assert_eq!(m.update(), Some(false));
        assert!(m.hud_ready(true));
        assert_eq!(m.update(), Some(true));
        assert!(!m.hud_ready(true));
        assert!(!pause::PauseManager::default().hud_ready(false));
    }
}
