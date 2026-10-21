use super::*;

impl FrontEnd {
    pub fn activate(&mut self, env: &mut FeEnv, provider: &str, view_model: &str) {
        match provider {
            "bootflow" => boot::activate(self, env),
            "menuhub" => menu::activate(self, env),
            "globe" => globe::activate(self, env, view_model),
            "gamemodes" => globe::activate_modes(self, env, view_model),
            "equipment" => equipment::activate(self, env),
            "equipmentloading" => loading::activate(self, env, view_model),
            "userdata" => self.publish.push("dp_user_data".into()),
            "activityfeed" => user::activate_feed(self, env),
            "pausemenu" => pause::activate(self, env),
            "postracemenu" => postrace::activate(self, env),
            "settings" => settings::activate(self, env),
            "unlocks" => unlocks::activate(self, env),
            "raceresults" => self.results_hint_wait = false,
            "tutorialpausemenu" => tutorial_pause::activate(self),
            "tutorial" => self.tutorial_ui = true,
            "badges" => badges::activate(self),
            "credits" => self.publish.push(credits::DP.into()),
            "music" => music::activate(self, env),
            "notification" => {
                self.notifications.request = 0;
                self.notifications.current = None;
            }
            "wallmenu" if env.values.int(0x22F) != 2 => {
                let dest = env.values.int(0x232) as u32;
                self.note(format!("wall menu bypassed to screen {dest:#x}"));
                self.go_to_screen(env, dest);
            }
            _ => {}
        }
    }

    pub fn deactivate(&mut self, env: &mut FeEnv, provider: &str, _view_model: &str) {
        match provider {
            "pausemenu" => pause::deactivate(self, env),
            "postracemenu" => postrace::deactivate(self),
            "settings" => settings::deactivate(self),
            "tutorialpausemenu" => tutorial_pause::deactivate(self),
            "tutorial" => self.tutorial_ui = false,
            "badges" => badges::deactivate(self),
            "music" => music::deactivate(self, env),
            "unlocks" => self.unlocks_hint_wait = false,
            "raceresults" => self.results_hint_wait = false,
            _ => {}
        }
    }

    pub fn action(&mut self, env: &mut FeEnv, provider: &str, action: &str, param: &ActionParam) {
        self.note(format!("action {action} ({provider}, {})", param.number));
        match provider {
            "bootflow" => boot::action(self, env, action, param.number),
            "menuhub" => menu::action(self, env, action),
            "equipment" => equipment::action(self, env, action, param),
            "activityfeed" => user::feed_action(self, env, action),
            "pausemenu" => pause::action(self, env, action),
            "postracemenu" => postrace::action(self, env, action),
            "settings" => settings::action(self, env, action, param),
            "unlocks" => unlocks::action(self, env, action),
            "notification" => notify::action(self, action, param),
            "tutorial" => hint::tutorial_action(self, env, action, param),
            "tutorialpausemenu" => tutorial_pause::action(self, env, action, param),
            "badges" => badges::action(self, action, param),
            "music" => music::action(self, env, action, param),
            "gameactions" if action == "A_AudioPlaySFX" => self.play_sound(param.number as i32),
            "gameactions" if action == "A_GameQuit" => flow::game_quit(self, env),
            "gameactions" if action == "A_GameQuitComplete" => flow::game_quit_complete(self, env),
            "gameactions" if action == "A_GameGameplayStarted" => self.go_to_screen(env, screens::id::HUD),
            "gameactions" if action == "ControllerPull" => {
                self.note("nav event globalpopup_open_controllerpull (no transition of ssxgame.nav takes it)");
                env.nav_events.push("globalpopup_open_controllerpull".into());
            }
            "raceresults" if action == "A_GameLevelUp" => hint::tutorial_action(self, env, action, param),
            "raceresults" if action == "A_GameAdvanceScreen" => {
                let success = self.last_success.unwrap_or(false);
                flow::results_advance(self, env, success)
            }
            "gamemodes" if action == "A_TrackProgressionAnimComplete" => {
                env.values.set_bool(progression::ANIM_COMPLETE, true);
            }
            _ => {}
        }
    }

    pub fn data_action(&mut self, env: &mut FeEnv, provider: &str, dp: &str, action: &str, param: &ActionParam) {
        self.note(format!("data action {action} on {dp} ({provider})"));
        match (provider, action) {
            ("settings", _) => settings::data_action(self, env, dp, action, param),
            ("music", _) => music::data_action(self, env, dp, action, param),
            ("globe", "A_GameAdvanceScreen") => globe::advance(self, env),
            ("globe", "A_GamePreviousScreen") => globe::previous(self, env),
            ("equipmentloading", "A_EquipmentLoadingBackgroundLoaded") => flow::loading_screen_loaded(self),
            ("equipmentloading", _) => {}
            _ => {}
        }
    }

    pub fn nav_action(&mut self, env: &mut FeEnv, name: &str, args: &[String]) -> bool {
        let arg = args.first().map(String::as_str).unwrap_or("");
        match name {
            "set_dimmer_state" | "push_dimmer_state" | "pop_dimmer_state" => self.dimmer.nav(name, arg),
            "show_selected_drop_info" | "hide_selected_drop_info" => {
                self.user.drop_info_shown = name == "show_selected_drop_info";
                self.publish.push("dp_drop_data".into());
            }
            "doContinue" | "doQuit" => flow::do_continue(self, env),
            "doRestart" => pause::do_restart(self, env),
            "do_skip_race" => flow::do_skip_race(self, env),
            "do_event_quit" | "do_event_restart" | "do_restart_never_show_again" | "do_close_popup" => {
                pause::nav_action(self, env, name);
            }
            "do_event_equipment_quit_yes" => crate::game::hud::nis::equipment_quit_yes(self, env),
            "do_event_equipment_quit_no" => crate::game::hud::nis::equipment_quit_no(self, env),
            "do_exit_settings_without_save"
            | "do_reset_gameplay_settings"
            | "do_reset_audio_settings"
            | "do_reset_tutorial_messages" => {
                settings::nav_action(self, env, name);
            }
            "sell_confirm" | "advance_to_game" | "advance_to_game_confirmed" | "never_show_low_survive_again" => {
                equipment::nav_action(self, env, name);
            }
            "evaluatePrevious" => flow::evaluate_previous(self, env, args),
            "do_reset_music_selection" => music::do_reset(self),
            "enterFE" | "enterGameplay" | "activityfeedwelcomestart" | "setGlobeInputEnabled" => {
                flow::misc_nav(self, env, name, arg)
            }
            "saveload_popup_open" | "saveload_popup_close" => {}
            _ => return false,
        }
        true
    }
}
