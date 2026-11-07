pub const TRICK_POINTS_SIGNATURE: f32 = 2400.0;
pub const TRICK_POINTS_1000: f32 = 1000.0;
pub const TRICK_POINTS_1200: f32 = 1200.0;
pub const TRICK_POINTS_1400: f32 = 1400.0;
pub const TRICK_POINTS_GRIND_1_3: f32 = 1000.0;
pub const TRICK_POINTS_GRIND_4_6: f32 = 1200.0;
pub const TRICK_POINTS_MAX: f32 = TRICK_POINTS_SIGNATURE;

pub const TRICKY_MULTIPLIER: f32 = 1.5;
pub const SUPER_TRICKY_MULTIPLIER: f32 = 2.0;
pub const TRICKY_AFTER_REWIND_TICKS: f32 = 240.0;
pub const REWIND_MULTIPLIER: f32 = 0.10;
pub const DUPLICATE_MULTIPLIER: f32 = 0.25;
pub const TWEAK_MULT_PER_EXECUTION: f32 = 0.10;
pub const HELI_INTERACTION_POINTS: f32 = 20_000.0;
pub const HELI_PENALTY_FACTOR: f32 = 0.10;
pub const BONK_POINTS: f32 = 5_000.0;
pub const FLARE_POINTS: f32 = 1_000.0;
pub const UPHILL_POINTS_PER_SECOND: f32 = -5_000.0;
pub const REWIND_POINTS_PER_SECOND: f32 = -1000.0;
pub const GRIND_DISTANCE_POINTS: f32 = 25.0;
pub const GRIND_DISTANCE_MIN: f32 = 20.0;
pub const GRIND_TIME_MAX: f32 = 1200.0;
pub const GRIND_TERRAIN_MULTIPLIER: f32 = 1.2;
pub const SUPER_GRIND_TERRAIN_MULTIPLIER: f32 = 1.0;
pub const FLOW_LEVEL_MIN: i32 = -1;
pub const FLOW_LEVEL_MAX: i32 = 19;

pub const UPHILL_TIME_NO_AIR_TRICKS: f32 = 180.0;
pub const TIME_SINCE_REWIND_DEFAULT: f32 = 1.0e6;
pub const TRAILING_DESCENT_DEFAULT: f32 = -10_000.0;
pub const SPEED_EPS: f32 = 1.525_9e-5;
pub const COMBO_ROTATION_SPEED: f32 = 0.025;
pub const BOARDPRESS_MIN_DIST: f32 = 10.0;
pub const BOARDPRESS_MIN_SPEED: f32 = 2.0;
pub const BOARDPRESS_DIST_POINTS: f32 = 5.0;
pub const FLOW_LEVEL_UP_POINTS: [f32; 3] = [1500.0, 2250.0, 3000.0];

pub fn trick_time_level(ticks: f32) -> f32 {
    [(300.0, 2.5), (240.0, 2.0), (180.0, 1.75), (120.0, 1.5), (60.0, 1.25), (30.0, 1.0), (15.0, 0.5)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.0, |(_, l)| l)
}

pub fn grind_trick_time_level(ticks: f32) -> f32 {
    [
        (1200.0, 0.0),
        (540.0, 2.0),
        (480.0, 1.8),
        (420.0, 1.4),
        (360.0, 1.0),
        (240.0, 0.8),
        (120.0, 0.5),
        (60.0, 0.3),
        (30.0, 0.2),
        (15.0, 0.1),
    ]
    .into_iter()
    .find(|&(t, _)| ticks >= t)
    .map_or(0.1, |(_, l)| l)
}

pub fn grind_spin_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (deg / 180.0).floor();
    n * (150.0 + 37.5 * (n - 1.0))
}

pub fn spin_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (deg / 180.0).floor();
    n * 150.0 + 30.0 * n * (n - 1.0) / 2.0
}

pub fn flip_points(deg: f32) -> f32 {
    if deg < 180.0 {
        return 0.0;
    }
    let n = (1.0 + (deg - 180.0) / 360.0).floor();
    n * (600.0 + (n - 1.0) * 300.0)
}

pub fn combo_bonus(count: u32) -> f32 {
    const TABLE: [(u32, f32); 26] = [
        (2, 2_000.0),
        (5, 10_000.0),
        (7, 15_000.0),
        (10, 25_000.0),
        (20, 50_000.0),
        (25, 75_000.0),
        (30, 100_000.0),
        (40, 125_000.0),
        (50, 150_000.0),
        (60, 175_000.0),
        (70, 200_000.0),
        (80, 225_000.0),
        (90, 250_000.0),
        (100, 300_000.0),
        (125, 350_000.0),
        (150, 400_000.0),
        (175, 450_000.0),
        (200, 500_000.0),
        (225, 550_000.0),
        (250, 600_000.0),
        (275, 650_000.0),
        (300, 700_000.0),
        (350, 800_000.0),
        (400, 900_000.0),
        (450, 950_000.0),
        (500, 1_000_000.0),
    ];
    TABLE.iter().rev().find(|&&(c, _)| count >= c).map_or(0.0, |&(_, b)| b)
}

pub fn rewind_time_level(ticks: f32) -> f32 {
    [(300.0, 1.5), (240.0, 1.25), (180.0, 1.0), (120.0, 0.75), (60.0, 0.5), (30.0, 0.35)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.10, |(_, l)| l)
}

pub const UPHILL_TIME_LEVEL: [(f32, f32); 6] =
    [(180.0, 1.0), (240.0, 1.25), (300.0, 1.5), (360.0, 1.75), (420.0, 2.0), (640.0, 0.0)];

pub fn uphill_time_level(ticks: f32) -> f32 {
    UPHILL_TIME_LEVEL.iter().rev().find(|&&(t, _)| ticks >= t).map_or(UPHILL_TIME_LEVEL[0].1, |&(_, l)| l)
}

pub fn rewind_ramp(ticks: f32) -> f32 {
    [(240.0, 1.0), (180.0, 0.5), (120.0, 0.25), (60.0, 0.10)]
        .into_iter()
        .find(|&(t, _)| ticks >= t)
        .map_or(0.05, |(_, m)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_largest_trick_points() {
        for p in [
            TRICK_POINTS_SIGNATURE,
            TRICK_POINTS_1000,
            TRICK_POINTS_1200,
            TRICK_POINTS_1400,
            TRICK_POINTS_GRIND_1_3,
            TRICK_POINTS_GRIND_4_6,
        ] {
            assert!(p <= TRICK_POINTS_MAX);
        }
    }
}
