#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Int,
    Float,
}

pub const TRACK: usize = 0;
pub const TRICK: usize = 1;
pub const PERSISTENT: usize = 2;

pub const STATS: [(&str, Kind, &[usize]); 123] = [
    ("CurrentFunds", Kind::Int, &[2]),
    ("ComboScore_TrickCombo", Kind::Float, &[0]),
    ("ComboScore_Track", Kind::Float, &[0]),
    ("ComboScore", Kind::Float, &[2]),
    ("ComboScoreTrickEvent_Track", Kind::Float, &[0]),
    ("ComboScoreTrickEvent", Kind::Float, &[2]),
    ("CompletedTrackWhileTricky_Track", Kind::Int, &[0]),
    ("CompletedTrackWhileUberTricky_Track", Kind::Int, &[0]),
    ("CompletedTrackWhileWingsuitDeployed_Track", Kind::Int, &[0]),
    ("CompletedTrackWhileBoosting_Track", Kind::Int, &[0]),
    ("EquippedSpeedBoard_Track", Kind::Int, &[0]),
    ("GeoCacheCollectedByPlayerEarnings", Kind::Int, &[2]),
    ("GeoCacheCollectedByOtherEarnings", Kind::Int, &[2]),
    ("GeoCacheExpiryEarnings", Kind::Int, &[2]),
    ("GeoCacheNumCollected", Kind::Int, &[2]),
    ("GeoCacheNumCollectedFromFriend", Kind::Int, &[2]),
    ("GeoCacheNumPlaced", Kind::Int, &[2]),
    ("GeoCacheNumSurvived", Kind::Int, &[2]),
    ("GeoCachePlacementEarnings", Kind::Int, &[2]),
    ("GeoCacheTotalEarnings", Kind::Int, &[2]),
    ("GhostNumEarnings", Kind::Int, &[2]),
    ("GhostNumFriendsGhostBeat", Kind::Int, &[2]),
    ("GhostNumWins", Kind::Int, &[2]),
    ("GhostNumUploaded", Kind::Int, &[2]),
    ("NumCharactersTakenDown", Kind::Int, &[2]),
    ("NumCharactersTakenDown_Track", Kind::Int, &[0]),
    ("NumTimesFinishedInDiamondBracket", Kind::Int, &[2]),
    ("NumTimesFinishedInPlatinumBracket", Kind::Int, &[2]),
    ("NumTimesFinishedInGoldBracket", Kind::Int, &[2]),
    ("NumTimesFinishedInSilverBracket", Kind::Int, &[2]),
    ("NumTimesCompetedRace_Track", Kind::Int, &[0]),
    ("NumTimesCompetedTrick_Track", Kind::Int, &[0]),
    ("NumTimesCompetedSurvive_Track", Kind::Int, &[0]),
    ("NumTimesCompetedRaceLiveEvent", Kind::Int, &[2]),
    ("NumTimesCompetedTrickLiveEvent", Kind::Int, &[2]),
    ("NumTimesCompetedSurviveLiveEvent", Kind::Int, &[2]),
    ("NumTimesCompetedCustomLiveEvent", Kind::Int, &[2]),
    ("NumUniqueDeadlyDescentsBeatenWithoutRewind", Kind::Int, &[2]),
    ("NumUnlockDeadlyDescents", Kind::Int, &[2]),
    ("NumRidersInEvent_Track", Kind::Int, &[2]),
    ("EventDropCost_Track", Kind::Int, &[0]),
    ("NumParticipatingEvents_Track", Kind::Int, &[0]),
    ("NumEarningsFromGlobalEvents", Kind::Int, &[2]),
    ("LifetimeEarnings", Kind::Int, &[2]),
    ("LifetimeSpending", Kind::Int, &[2]),
    ("LifetimeSpentOnBoards", Kind::Int, &[2]),
    ("LifetimeSpentOnBoost", Kind::Int, &[2]),
    ("LifetimeSpentOnGear", Kind::Int, &[2]),
    ("MaxAvalancheMass_Track", Kind::Float, &[0]),
    ("MaxCharacterLevel", Kind::Int, &[2]),
    ("MaxSpeed", Kind::Float, &[2]),
    ("MaxSpeed_Track", Kind::Float, &[0]),
    ("MaxTimeTrickyEngaged", Kind::Float, &[2]),
    ("MaxTimeTrickyEngaged_Track", Kind::Float, &[0]),
    ("MaxTimeUberTrickyEngaged", Kind::Float, &[2]),
    ("MedalBronzeNumEarned_Track", Kind::Int, &[0]),
    ("MedalSilverNumEarned_Track", Kind::Int, &[0]),
    ("MedalGoldNumEarned_Track", Kind::Int, &[0]),
    ("NumAirTime_Trick", Kind::Float, &[1]),
    ("NumAirTime_Track", Kind::Float, &[0]),
    ("NumAirTime", Kind::Float, &[2]),
    ("NumDistanceDescended_Track", Kind::Float, &[0]),
    ("NumDistanceDescended", Kind::Float, &[2]),
    ("NumDistanceDescendedInRaceEvents", Kind::Float, &[2]),
    ("NumWipeouts_Track", Kind::Int, &[0]),
    ("NumWipeoutsUsingWingsuit_Track", Kind::Int, &[0]),
    ("NumCombosChained_TrickCombo", Kind::Int, &[0]),
    ("NumCombosChained_Track", Kind::Int, &[0]),
    ("NumCombosChained", Kind::Int, &[2]),
    ("NumCompletedDrops_Track", Kind::Int, &[0]),
    ("NumCompletedExploreDrops_Track", Kind::Int, &[0]),
    ("NumCompletedRaceEvent_Track", Kind::Int, &[0]),
    ("NumCompletedSurviveEvent_Track", Kind::Int, &[0]),
    ("NumCompletedTrickEvent_Track", Kind::Int, &[0]),
    ("NumCompletedPlayTillDeathSurviveEvent_Track", Kind::Int, &[0]),
    ("NumDeaths_Track", Kind::Int, &[0]),
    ("NumFlips_Trick", Kind::Int, &[1]),
    ("NumFlips_Track", Kind::Int, &[0]),
    ("NumFlips", Kind::Int, &[2]),
    ("NumFlipDegrees_Trick", Kind::Float, &[1]),
    ("NumFlipDegrees_Track", Kind::Float, &[0]),
    ("NumFlipDegrees", Kind::Float, &[2]),
    ("NumGrindTime_Trick", Kind::Float, &[1]),
    ("NumGrindTime_Track", Kind::Float, &[0]),
    ("NumGrindTime", Kind::Float, &[2]),
    ("NumLimitedTimeOfferItemsPurchased", Kind::Int, &[2]),
    ("NumRewindsUsed_Track", Kind::Int, &[0]),
    ("NumRewindsUsed", Kind::Int, &[2]),
    ("NumRewindsOutOfDeath", Kind::Int, &[2]),
    ("NumSurviveRewindsUsed", Kind::Int, &[2]),
    ("NumUniqueSignatureTricksPerformed", Kind::Int, &[2]),
    ("NumSignatureTricksPerformed_Track", Kind::Int, &[0]),
    ("NumSpins_Trick", Kind::Int, &[1]),
    ("NumSpins_Track", Kind::Int, &[0]),
    ("NumSpins", Kind::Int, &[2]),
    ("NumSpinDegrees_Trick", Kind::Float, &[1]),
    ("NumSpinDegrees_Track", Kind::Float, &[0]),
    ("NumSpinDegrees", Kind::Float, &[2]),
    ("NumSuperUberTricksPerformed_Track", Kind::Int, &[0]),
    ("NumTimeBoosting_Track", Kind::Float, &[0]),
    ("NumTimeBoosting", Kind::Float, &[2]),
    ("NumTimesEngagedTricky", Kind::Int, &[2]),
    ("NumTimesHelicopterUsed_Track", Kind::Int, &[0]),
    ("NumTimesHelicopterUsed", Kind::Int, &[2]),
    ("NumTimeInTricky", Kind::Float, &[2]),
    ("NumTimeInTricky_Track", Kind::Float, &[0]),
    ("NumTimesEngagedUberTricky", Kind::Int, &[2]),
    ("NumTimeInUberTricky", Kind::Float, &[2]),
    ("NumTimeInUberTricky_Track", Kind::Float, &[0]),
    ("NumTimesEngagedWingsuit", Kind::Int, &[2]),
    ("NumTimeUsingWingsuit", Kind::Float, &[2]),
    ("NumTimeUsingWingsuit_Track", Kind::Float, &[0]),
    ("NumTimesUsedBoost_Track", Kind::Int, &[0]),
    ("NumTimesUsedBoost", Kind::Int, &[2]),
    ("NumUberTricksPerformed_Track", Kind::Int, &[0]),
    ("NumWipeouts", Kind::Int, &[2]),
    ("SurviveNumLaps_Track", Kind::Int, &[0]),
    ("Time_Track", Kind::Float, &[0]),
    ("TutorialComplete", Kind::Int, &[2]),
    ("TimeExceedingTargetSpeed1", Kind::Float, &[0, 2]),
    ("TimeExceedingTargetSpeed2", Kind::Float, &[0, 2]),
    ("TimeExceedingTargetSpeed3", Kind::Float, &[0, 2]),
    ("TimeExceedingTargetSpeed4", Kind::Float, &[0, 2]),
];

pub(super) mod id {
    pub const COMBO_SCORE_TRICK_COMBO: usize = 1;
    pub const COMBO_SCORE_TRACK: usize = 2;
    pub const COMBO_SCORE: usize = 3;
    pub const COMBO_SCORE_TRICK_EVENT_TRACK: usize = 4;
    pub const COMBO_SCORE_TRICK_EVENT: usize = 5;
    pub const COMPLETED_TRACK_WHILE_TRICKY_TRACK: usize = 6;
    pub const NUM_CHARACTERS_TAKEN_DOWN: usize = 24;
    pub const NUM_CHARACTERS_TAKEN_DOWN_TRACK: usize = 25;
    pub const MAX_AVALANCHE_MASS_TRACK: usize = 48;
    pub const NUM_TIMES_COMPETED_RACE_TRACK: usize = 30;
    pub const NUM_TIMES_COMPETED_TRICK_TRACK: usize = 31;
    pub const NUM_TIMES_COMPETED_SURVIVE_TRACK: usize = 32;
    pub const MAX_SPEED: usize = 50;
    pub const MAX_SPEED_TRACK: usize = 51;
    pub const MAX_TIME_TRICKY_ENGAGED: usize = 52;
    pub const MAX_TIME_TRICKY_ENGAGED_TRACK: usize = 53;
    pub const NUM_AIR_TIME_TRICK: usize = 58;
    pub const NUM_AIR_TIME_TRACK: usize = 59;
    pub const NUM_AIR_TIME: usize = 60;
    pub const NUM_DISTANCE_DESCENDED_TRACK: usize = 61;
    pub const NUM_DISTANCE_DESCENDED: usize = 62;
    pub const NUM_DISTANCE_DESCENDED_IN_RACE_EVENTS: usize = 63;
    pub const NUM_WIPEOUTS_TRACK: usize = 64;
    pub const NUM_WIPEOUTS_USING_WINGSUIT_TRACK: usize = 65;
    pub const NUM_COMBOS_CHAINED_TRICK_COMBO: usize = 66;
    pub const NUM_COMBOS_CHAINED_TRACK: usize = 67;
    pub const NUM_COMBOS_CHAINED: usize = 68;
    pub const NUM_COMPLETED_PLAY_TILL_DEATH_TRACK: usize = 74;
    pub const NUM_FLIPS_TRICK: usize = 76;
    pub const NUM_FLIP_DEGREES_TRICK: usize = 79;
    pub const NUM_GRIND_TIME_TRICK: usize = 82;
    pub const NUM_REWINDS_USED_TRACK: usize = 86;
    pub const NUM_REWINDS_USED: usize = 87;
    pub const NUM_REWINDS_OUT_OF_DEATH: usize = 88;
    pub const NUM_SURVIVE_REWINDS_USED: usize = 89;
    pub const NUM_UNIQUE_SIGNATURE_TRICKS: usize = 90;
    pub const NUM_SIGNATURE_TRICKS_TRACK: usize = 91;
    pub const NUM_SPINS_TRICK: usize = 92;
    pub const NUM_SPIN_DEGREES_TRICK: usize = 95;
    pub const NUM_SUPER_UBER_TRICKS_TRACK: usize = 98;
    pub const NUM_TIME_BOOSTING_TRACK: usize = 99;
    pub const NUM_TIME_BOOSTING: usize = 100;
    pub const NUM_TIMES_ENGAGED_TRICKY: usize = 101;
    pub const NUM_TIMES_HELICOPTER_USED_TRACK: usize = 102;
    pub const NUM_TIMES_HELICOPTER_USED: usize = 103;
    pub const NUM_TIME_IN_TRICKY: usize = 104;
    pub const NUM_TIME_IN_TRICKY_TRACK: usize = 105;
    pub const NUM_TIMES_ENGAGED_UBER_TRICKY: usize = 106;
    pub const NUM_TIMES_ENGAGED_WINGSUIT: usize = 109;
    pub const NUM_TIME_USING_WINGSUIT: usize = 110;
    pub const NUM_TIME_USING_WINGSUIT_TRACK: usize = 111;
    pub const NUM_TIMES_USED_BOOST_TRACK: usize = 112;
    pub const NUM_TIMES_USED_BOOST: usize = 113;
    pub const NUM_UBER_TRICKS_TRACK: usize = 114;
    pub const NUM_WIPEOUTS: usize = 115;
    pub const SURVIVE_NUM_LAPS_TRACK: usize = 116;
    pub const TIME_TRACK: usize = 117;
    pub const TIME_EXCEEDING_TARGET_SPEED1: usize = 119;
}

pub const TARGET_SPEEDS_KMH: [f32; 4] = [100.0, 130.0, 160.0, 175.0];

pub(super) const GEAR_ITEMS: [&str; 15] = [
    "Board",
    "Wingsuit",
    "IceAxe",
    "Armour",
    "Thermal",
    "Oxygen",
    "HeadLamp",
    "PulseGoggles",
    "Boost1",
    "Boost2",
    "Boost3",
    "Boost4",
    "Jacket",
    "Pants",
    "SmokeCanister",
];

pub const TRICK_FAMILIES: [&str; 3] = ["TrickNumPerformed_", "TrickNumUberPerformed_", "TrickNumSuperUberPerformed_"];

pub const TRICK_NAMES: [&str; 32] = [
    "None",
    "Grab_Signature",
    "Grab_FlyingSquirrel",
    "Grab_Indy",
    "Grab_Melon",
    "Grab_Mute",
    "Grab_Nose",
    "Grab_Rocket",
    "Grab_Seatbelt",
    "Grab_Stalefish",
    "Grab_Stiffy",
    "Grab_Tail",
    "Grab_Truckdriver",
    "Grab_Wheelman",
    "Grind_1",
    "Grind_1_Uber",
    "Grind_1_SUber",
    "Grind_2",
    "Grind_2_Uber",
    "Grind_2_SUber",
    "Grind_3",
    "Grind_3_Uber",
    "Grind_3_SUber",
    "Grind_4",
    "Grind_4_Uber",
    "Grind_4_SUber",
    "Grind_5",
    "Grind_5_Uber",
    "Grind_5_SUber",
    "Grind_6",
    "Grind_6_Uber",
    "Grind_6_SUber",
];

pub(super) fn trick_name(trick: crate::tricks::TrickType) -> Option<&'static str> {
    TRICK_NAMES.get(usize::from(trick.value())).copied()
}
