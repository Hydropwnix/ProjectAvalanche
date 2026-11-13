pub const SCREENS: [(&str, u8, &str); 86] = [
    ("go_to_blank", 0, ""),
    ("go_to_boot", 0, ""),
    ("go_to_menu", 0, ""),
    ("go_to_explore_wall_menu", 2, "TXT_T_WALL"),
    ("go_to_global_wall_menu", 2, "TXT_T_WALL"),
    ("go_to_welcome_hub", 0, ""),
    ("go_to_customglobalevent", 2, "TXT_T_CUSTOM_EVENT"),
    ("go_to_settings", 0, ""),
    ("go_to_settings_menu", 0, ""),
    ("go_to_video_calibration", 0, ""),
    ("go_to_controller_settings", 0, ""),
    ("_", 0, ""),
    ("go_to_explorerange", 2, "TXT_T_GLOBE_RANGE"),
    ("go_to_explorepeak", 2, "TXT_T_GLOBE_PEAK"),
    ("go_to_exploredrop", 2, "TXT_T_GLOBE_DROP"),
    ("go_to_globalrange", 2, "TXT_T_GLOBE_RANGE"),
    ("go_to_globalpeak", 2, "TXT_T_GLOBE_PEAK"),
    ("go_to_globaldrop", 2, "TXT_T_GLOBE_DROP"),
    ("_", 0, ""),
    ("go_to_ssx_login", 0, ""),
    ("go_to_ssx_privacypolicy_tos_menu", 0, ""),
    ("go_to_ssx_privacypolicy_view_legal_document", 0, ""),
    ("go_to_ssx_create_account", 0, ""),
    ("go_to_info_sharing_option", 0, ""),
    ("go_to_ssx_login_error", 0, ""),
    ("go_to_ssx_login_info", 0, ""),
    ("complete_ssx_login", 0, ""),
    ("go_to_getonlinepass", 0, ""),
    ("complete_getonlinepass", 0, ""),
    ("_", 0, ""),
    ("go_to_pause", 0, ""),
    ("go_to_tutorial_pause", 0, ""),
    ("_", 0, ""),
    ("go_to_results", 0, ""),
    ("go_to_liveevent_results", 0, ""),
    ("go_to_explore_results", 0, ""),
    ("go_to_explore_awards", 0, ""),
    ("go_to_explore_postmenu", 0, ""),
    ("go_to_campaign_results", 0, ""),
    ("go_to_death_screen", 0, ""),
    ("go_to_unlocks_screen", 0, ""),
    ("_", 0, ""),
    ("_", 0, ""),
    ("go_to_quicktrack", 0, ""),
    ("go_to_quickboarder", 0, ""),
    ("go_to_quickboard", 0, ""),
    ("_", 0, ""),
    ("_", 0, ""),
    ("go_to_equipment", 2, "TXT_T_CHOOSE_GEAR"),
    ("go_to_equipment_inventory", 0, ""),
    ("_", 0, ""),
    ("go_to_equipment_loading", 0, ""),
    ("_", 0, ""),
    ("go_to_activityfeed_front", 2, "TXT_M_GLOBE_ACTIVITY_FEED"),
    ("go_to_activityfeed_content", 2, "TXT_M_GLOBE_ACTIVITY_FEED"),
    ("go_to_hud", 0, ""),
    ("_", 0, ""),
    ("_", 0, ""),
    ("go_to_activity_feed_music_menu", 0, ""),
    ("go_to_activity_feed_personalize_music", 0, ""),
    ("go_to_activity_feed_track_listing", 0, ""),
    ("go_to_tutorial01", 0, ""),
    ("go_to_objective", 0, ""),
    ("go_to_restart_stage", 0, ""),
    ("go_to_quit_stage", 0, ""),
    ("go_to_continue_stage", 0, ""),
    ("go_to_campaignrange", 2, "TXT_T_GLOBE_RANGE"),
    ("go_to_campaignpeak", 2, "TXT_T_GLOBE_PEAK"),
    ("go_to_campaigndrop", 2, "TXT_T_GLOBE_DROP"),
    ("go_to_fereconnect", 0, ""),
    ("hint_enable", 0, ""),
    ("hint_disable", 0, ""),
    ("hudmeters_enable", 0, ""),
    ("hudmeters_disable", 0, ""),
    ("nisprompt_enable", 0, ""),
    ("nisprompt_disable", 0, ""),
    ("nistitle_enable", 0, ""),
    ("nistitle_disable", 0, ""),
    ("boostnotification_enable", 0, ""),
    ("boostnotification_disable", 0, ""),
    ("dynamicleaderboard_enable", 0, ""),
    ("dynamicleaderboard_disable", 0, ""),
    ("finishmessage_enable", 0, ""),
    ("finishmessage_disable", 0, ""),
    ("backgroundcontainer_enable", 0, ""),
    ("backgroundcontainer_disable", 0, ""),
];

pub mod id {
    pub const BOOT: u32 = 1;
    pub const MAIN_MENU: u32 = 2;
    pub const EXPLORE_WALL_MENU: u32 = 3;
    pub const WELCOME_HUB: u32 = 5;
    pub const EXPLORE_RANGE: u32 = 0xC;
    pub const EXPLORE_PEAK: u32 = 0xD;
    pub const EXPLORE_DROP: u32 = 0xE;
    pub const GLOBAL_RANGE: u32 = 0xF;
    pub const GLOBAL_PEAK: u32 = 0x10;
    pub const GLOBAL_DROP: u32 = 0x11;
    pub const SETTINGS: u32 = 7;
    pub const SETTINGS_MENU: u32 = 8;
    pub const PAUSE: u32 = 0x1E;
    pub const TUTORIAL_PAUSE: u32 = 0x1F;
    pub const RESULTS: u32 = 0x21;
    pub const EXPLORE_RESULTS: u32 = 0x23;
    pub const EXPLORE_POST_MENU: u32 = 0x25;
    pub const DEATH_SCREEN: u32 = 0x27;
    pub const UNLOCKS: u32 = 0x28;
    pub const QUICKTRACK: u32 = 0x2B;
    pub const EQUIPMENT: u32 = 0x30;
    pub const LOADING: u32 = 0x33;
    pub const ACTIVITY_FEED_CONTENT: u32 = 0x36;
    pub const HUD: u32 = 0x37;
    pub const OBJECTIVE: u32 = 0x3E;
    pub const RESTART_STAGE: u32 = 0x3F;
    pub const QUIT_STAGE: u32 = 0x40;
    pub const CONTINUE_STAGE: u32 = 0x41;
    pub const CAMPAIGN_RANGE: u32 = 0x42;
    pub const CAMPAIGN_PEAK: u32 = 0x43;
    pub const CAMPAIGN_DROP: u32 = 0x44;
    pub const FE_RECONNECT: u32 = 0x45;
}

pub fn nav_event(screen: u32) -> Option<&'static str> {
    SCREENS.get(screen as usize).map(|s| s.0).filter(|e| e.len() > 1)
}

pub fn screen_of(event: &str) -> Option<u32> {
    SCREENS.iter().position(|s| s.0 == event && s.0.len() > 1).map(|i| i as u32)
}

pub fn writes_breadcrumb(screen: u32) -> bool {
    let peak_or_drop = matches!(screen, 0xD | 0x43 | 0x10 | 0xE | 0x44 | 0x11);
    SCREENS.get(screen as usize).is_some_and(|s| s.1 == 2) && !peak_or_drop
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        assert_eq!(nav_event(id::LOADING), Some("go_to_equipment_loading"));
        assert_eq!(nav_event(id::CAMPAIGN_RANGE), Some("go_to_campaignrange"));
        assert_eq!(nav_event(0xB), None);
        assert_eq!(screen_of("go_to_welcome_hub"), Some(id::WELCOME_HUB));
        assert_eq!(screen_of("go_to_equipment"), Some(id::EQUIPMENT));
        assert!(writes_breadcrumb(id::CAMPAIGN_RANGE) && !writes_breadcrumb(id::CAMPAIGN_DROP));
        assert!(writes_breadcrumb(id::EQUIPMENT) && !writes_breadcrumb(id::MAIN_MENU));
    }
}
