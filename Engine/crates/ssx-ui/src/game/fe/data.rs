use crate::game::hud::{DataField, DataObject};

const fn f(value: u16, kind: u8, fmt: u8, name: &'static str) -> DataField {
    DataField { value, kind, name, fmt }
}

pub const FE_DATA: &[DataObject] = &[
    DataObject {
        name: "dp_busy_icon_data",
        statics: &[],
        fields: &[f(0x9A, 1, 0, "iconType"), f(0x9B, 4, 0, "message"), f(0x9C, 0, 0, "inLoadingScreen")],
    },
    DataObject { name: "dp_fedock_data", statics: &[], fields: &[f(0x11, 1, 0, "animationState")] },
    DataObject { name: "dp_fedock_busy_icon_status", statics: &[], fields: &[f(0x9A, 1, 0, "busyIconStatus")] },
    DataObject { name: "dp_bread_crumb_data", statics: &[], fields: &[f(0x32, 4, 0, "mode"), f(0x33, 4, 0, "screen")] },
    DataObject { name: "dp_background_transition", statics: &[], fields: &[f(0x2E, 1, 0, "transitionState")] },
    DataObject { name: "dp_globe_transition", statics: &[], fields: &[f(0xBB, 0, 0, "isGlobeTransitioning")] },
    DataObject {
        name: "dp_mode_exploredata",
        statics: &[("progressionHeading", "TXT_EXPLORE"), ("progressionText", "TXT_MAIN_MENU_EXPLORE_DESC")],
        fields: &[f(0x89, 0, 0, "isEnabled")],
    },
    DataObject {
        name: "dp_mode_campaigndata",
        statics: &[("progressionHeading", "TXT_CAMPAIGN"), ("progressionText", "TXT_MAIN_MENU_CAMPAIGN_DESC")],
        fields: &[f(0x40, 0, 0, "isEnabled"), f(0x45, 1, 0, "percentComplete")],
    },
    DataObject {
        name: "dp_range_exploredata",
        statics: &[],
        fields: &[
            f(0x8E, 0, 0, "isEnabled"),
            f(0x8D, 4, 3, "peakTitle"),
            f(0x8A, 4, 0, "peakAsset"),
            f(0x8C, 4, 3, "peakName"),
            f(0x8B, 4, 3, "peakDescription"),
        ],
    },
    DataObject {
        name: "dp_range_campaigndata",
        statics: &[],
        fields: &[
            f(0x46, 0, 0, "isEnabled"),
            f(0x44, 4, 3, "peakTitle"),
            f(0x41, 4, 0, "peakAsset"),
            f(0x43, 4, 3, "peakName"),
            f(0x42, 4, 3, "peakDescription"),
        ],
    },
    DataObject {
        name: "dp_drop_metadata",
        statics: &[],
        fields: &[
            f(0xB8, 10, 0, "dropPanel"),
            f(0xB5, 3, 0, "dropXPos"),
            f(0xB6, 3, 0, "dropYPos"),
            f(0xC6, 0, 0, "isRecommended"),
            f(0xCA, 4, 0, "recommendationTitle"),
            f(0xC7, 4, 0, "recommendationGamerpic"),
        ],
    },
    DataObject {
        name: "dp_campaign_drop_info",
        statics: &[("goalRaceHeading", "TXT_RIDERS"), ("rewardLabel", "TXT_EXPLORE_DROP_REWARD_LABEL")],
        fields: &[
            f(0x36, 0, 0, "isEnabled"),
            f(0x34, 1, 2, "bestScore"),
            f(0x35, 1, 1, "bestTime"),
            f(0x3E, 1, 0, "goalType"),
            f(0x4F, 4, 0, "dropDescription"),
            f(0x50, 0, 0, "completed"),
            f(0x53, 4, 0, "dropName"),
            f(0x48, 4, 0, "boardName"),
            f(0x4A, 4, 0, "kitName"),
            f(0x4C, 4, 0, "riderName"),
            f(0x4E, 1, 1, "requiredTime"),
            f(0x4D, 1, 2, "requiredScore"),
            f(0x54, 5, 4, "unlockItemList"),
            f(0x39, 5, 0, "goalDescTitles"),
            f(0x3A, 5, 0, "goalImages"),
            f(0x37, 4, 0, "dropDetailImage"),
            f(0x3C, 1, 2, "completionReward"),
            f(0x3B, 0, 0, "isDropH2H"),
            f(0x51, 0, 0, "isLocked"),
            f(0x71, 4, 0, "goalDescBody"),
            f(0x73, 4, 0, "goalHeading"),
        ],
    },
    DataObject {
        name: "dp_drop_exploredata",
        statics: &[],
        fields: &[
            f(0x7A, 8, 0, "leaderboard"),
            f(0x7B, 8, 0, "leaderboardGhosts"),
            f(0x90, 9, 0, "rivalPanel"),
            f(0x1C3, 0, 0, "recommendsIsRival"),
            f(0x7C, 4, 0, "leaderboardHeading"),
            f(0x80, 4, 0, "leaderboardRiderLabel"),
            f(0x81, 4, 0, "leaderboardScoreLabel"),
            f(0x7F, 0, 0, "resetLeaderboard"),
            f(0x82, 1, 0, "userRow"),
            f(0x7D, 4, 0, "leaderboardNextPage"),
            f(0x7E, 4, 0, "leaderboardPrevPage"),
            f(0xCA, 4, 0, "recommendationTitle"),
            f(0xC4, 4, 0, "recommendationBody"),
            f(0x22F, 1, 0, "onlineStatus"),
        ],
    },
    DataObject { name: "dp_live_event_feed", statics: &[], fields: &[f(0x1AC, 4, 0, "message")] },
];

pub fn object(dp: &str) -> Option<&'static DataObject> {
    FE_DATA.iter().find(|o| o.name == dp)
}
