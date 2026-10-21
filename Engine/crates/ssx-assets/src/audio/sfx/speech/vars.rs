use std::collections::HashMap;

pub mod id {
    pub const GAME_MODE: i32 = 1;
    pub const RACE_IS_ACTIVE: i32 = 2;
    pub const BOTS_TOTAL_AT_START: i32 = 3;
    pub const BOTS_STILL_ALIVE_DURING_RUN: i32 = 4;
    pub const REGION: i32 = 5;
    pub const TRACK_WIN_CONDITION: i32 = 8;
    pub const RUN_COMPLETE_RESULT: i32 = 9;
    pub const CHOPPER_PILOT_CHARACTER_ID: i32 = 10;
    pub const IS_RIDING_IN_AVALANCHE: i32 = 12;
    pub const RIDER_CHARACTER_ID: i32 = 20;
    pub const AVALANCHE_SIZE: i32 = 26;
    pub const AVALANCHE_DISTANCE_TO_RIDER: i32 = 27;
    pub const AVALANCHE_SPEED: i32 = 28;
    pub const RIDER_CURRENT_TIME: i32 = 49;
    pub const RIDER_IS_TUMBLING: i32 = 50;
    pub const RIDER_IS_IN_A_TUNNEL: i32 = 51;
    pub const GOAL_RACE_TIME_TO_BEAT: i32 = 59;
    pub const GOAL_RACE_MINIMUM_TIME_TO_BEAT: i32 = 60;
    pub const TRACK_LENGTH_REMAINING: i32 = 61;
    pub const TRACK_PERCENT_REMAINING: i32 = 62;
    pub const RIDER_STANDSTILL_TIME_MS: i32 = 63;
    pub const RIDER_CURRENT_SPEED: i32 = 66;
    pub const RIDER_AVAILABLE_BOOST: i32 = 67;
    pub const RIDER_BOOST_ENABLED: i32 = 68;
    pub const TRICK_POINTS_TOTAL_FOR_RUN: i32 = 71;
    pub const TRICK_POINTS_TOTAL_FOR_TRICK: i32 = 73;
    pub const RIDER_CURRENT_RACE_POSITION: i32 = 74;
    pub const RIDER_ELAPSED_SPLIT_TIME: i32 = 75;
    pub const GOAL_TRICK_MINIMUM_POINTS_TO_BEAT: i32 = 76;
    pub const GOAL_TRICK_POINTS_TOTAL_TO_BEAT: i32 = 77;
    pub const RANDOM_NUMBER: i32 = 80;
    pub const IS_AWESOME: i32 = 87;
    pub const RIDER_IS_REWINDING: i32 = 88;
    pub const RIDER_IS_IN_DEATH_STATE: i32 = 89;
    pub const IS_DEADLY_DESCENT: i32 = 90;
    pub const PILOT_VOICE_MILLISECONDS_REMAINING: i32 = 101;
    pub const RIDER_VOICE_MILLISECONDS_REMAINING: i32 = 102;
    pub const RIDER_AIR_TIME_MS: i32 = 104;
    pub const RIDER_MILLISECONDS_TO_LANDING: i32 = 107;
    pub const RIDER_SCORING_FLIGHT_DISTANCE: i32 = 109;
    pub const TIME_IN_SUPER_AWESOME: i32 = 187;
    pub const DROP_AVALANCHE_PROBABILITY: i32 = 200;
    pub const IS_COLD_DROP: i32 = 201;
    pub const IS_LOW_OXYGEN_DROP: i32 = 202;
    pub const IS_NIGHT_DROP: i32 = 205;
    pub const WIND_SPEED: i32 = 207;
    pub const PRECIPITATION_INTENSITY: i32 = 208;
    pub const IS_AVALANCHE_DROP: i32 = 91;
    pub const RIDER_EFFECT_TEMPERATURE: i32 = 350;
    pub const RIDER_EFFECT_HEALTH: i32 = 351;
    pub const RIDER_EFFECT_OXYGEN: i32 = 352;
    pub const RIDER_CUMULATIVE_COLLISIONS: i32 = 353;
    pub const RIDER_EFFECT_TEMPERATURE_IS_IN_SUNLIGHT: i32 = 355;
    pub const RIDER_INTERNAL_TEMPERATURE: i32 = 356;
    pub const DEATH_REASON: i32 = 666;
    pub const PREVIOUS_DEATH_REASON: i32 = 667;
    pub const WIND_SPEED_MAX_THIS_RUN: i32 = 2007;
    pub const PRECIPITATION_INTENSITY_MAX_THIS_RUN: i32 = 2008;
}

fn readable(v: i32) -> bool {
    matches!(v,
        1..=5 | 8..=10 | 0xC | 0x14 | 0x1A..=0x1C | 0x31..=0x33 | 0x35..=0x3F | 0x42..=0x44 | 0x47
        | 0x49..=0x66 | 0x68..=0x6B | 0x6D | 0xBB | 0xBF..=0xC3 | 200..=0xD3 | 300..=0x13D
        | 0x15E..=0x161 | 0x163 | 0x164 | 0x29A | 0x29B | 0x7D7 | 0x7D8)
}

#[derive(Clone, Debug)]
pub struct Vars {
    map: HashMap<i32, i32>,
}

impl Default for Vars {
    fn default() -> Self {
        let mut map = HashMap::new();
        for v in [0x1B, 0x1A, 0x1C, 0xC, 0x43, 0x44, 0x42, 0x3F, 0x6D, 0x32, 0x33, 0x3D, 0x3E, 0x47, 0x49, 4, 3, 0x57]
            .into_iter()
            .chain([0xBB, 0xCF, 0x7D7, 0xD0, 0x7D8, 0x29A, 0x58, 0x59, 0x161, 0x31, 0x4B, 0x4A])
        {
            map.insert(v, 0);
        }
        for v in [0xD2, 0xD3, 0x15E, 0x15F, 0x160, 0x164, 0x163] {
            map.insert(v, -1);
        }
        for v in [10, 0x3B, 0x3C, 0x4D, 0x4C, 2, 5, 0x14] {
            map.insert(v, -1);
        }
        for v in [8, 0x4F, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x39, 0x3A, 0x37, 0x38, 0x35, 0x36, 200, 300, 0x29B, 9]
            .into_iter()
            .chain(0x5B..=0x64)
            .chain(0xBF..=0xC3)
            .chain(0x12D..=0x13D)
        {
            map.insert(v, 0);
        }
        Self { map }
    }
}

impl Vars {
    pub fn get(&self, v: i32) -> i32 {
        if readable(v) { self.map.get(&v).copied().unwrap_or(0) } else { -1 }
    }

    pub fn set(&mut self, v: i32, value: i32) {
        self.map.insert(v, value);
    }

    pub fn add(&mut self, v: i32, delta: i32) {
        *self.map.entry(v).or_insert(0) += delta;
    }

    pub fn raw(&self, v: i32) -> i32 {
        self.map.get(&v).copied().unwrap_or(0)
    }
}
