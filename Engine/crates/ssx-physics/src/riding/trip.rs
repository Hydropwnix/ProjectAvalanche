use glam::Vec3;

use crate::rider::grind::surface_analysis::{GRID, SurfaceAnalysis};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TripTuning {
    pub short_look_ahead: f32,
    pub long_look_ahead: f32,
    pub min_traj_height: f32,
    pub max_traj_height: f32,
    pub min_obstacle_height: f32,
    pub max_obstacle_height: f32,
    pub min_obstacle_grade: f32,
    pub energy_preservation: f32,
    pub min_obstacle_clearance: f32,
    pub min_speed: f32,
    pub max_damage_speed: f32,
    pub damage_amount: f32,
}

impl TripTuning {
    pub fn load(a: &crate::tuning::AssetView<'_>) -> anyhow::Result<Self> {
        Ok(Self {
            short_look_ahead: a.f32("ShortTripLookAheadTicks")?,
            long_look_ahead: a.f32("LongTripLookAheadTicks")?,
            min_traj_height: a.f32("MinTripTrajHeight")?,
            max_traj_height: a.f32("MaxTripTrajHeight")?,
            min_obstacle_height: a.f32("MinTripObstacleHeight")?,
            max_obstacle_height: a.f32("MaxTripObstacleHeight")?,
            min_obstacle_grade: a.f32("MinTripObstacleGrade")?,
            energy_preservation: a.f32("TripEnergyPreservation")?,
            min_obstacle_clearance: a.f32("MinTripObstacleClearance")?,
            min_speed: a.f32("MinTripSpeed")?,
            max_damage_speed: a.f32("MaxTripDamageSpeed")?,
            damage_amount: a.f32("TripDamageAmount")?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TripInputs<'a> {
    pub allowed: bool,
    pub sa: Option<&'a SurfaceAnalysis>,
    pub traj: Vec3,
    pub velocity: Vec3,
    pub speed: f32,
    pub in_rock_bail: bool,
    pub grinding: bool,
    pub dt: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trip {
    pub dp: Vec3,
    pub v: Vec3,
    pub a: Vec3,
    pub damage: f32,
    pub speed: f32,
    pub clearance: i32,
    pub obstacle: i32,
    pub max_height: i32,
    pub grade: i32,
}

fn round_cell(x: f32) -> i32 {
    (x + 0.5).floor() as i32
}

fn to_grid_point(sa: &SurfaceAnalysis, p: Vec3) -> Vec3 {
    let m = &sa.to_grid;
    let f = |k: usize| p.z.mul_add(m.z_axis[k], p.y.mul_add(m.y_axis[k], p.x.mul_add(m.x_axis[k], m.w_axis[k])));
    Vec3::new(f(0), f(1), f(2))
}

fn rotate(rows: &glam::Mat4, v: Vec3) -> Vec3 {
    let f = |k: usize| v.z.mul_add(rows.z_axis[k], v.y.mul_add(rows.y_axis[k], v.x * rows.x_axis[k]));
    Vec3::new(f(0), f(1), f(2))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TripWalk {
    pub start: (i32, i32, i32),
    pub end_row: i32,
    pub clearance: i32,
    pub obstacle: i32,
    pub max_height: i32,
    pub grade: i32,
    pub first: i32,
    pub limits: [i32; 5],
    pub trips: bool,
}

pub fn trip_walk(t: &TripTuning, i: &TripInputs) -> Option<TripWalk> {
    let sa = i.sa.filter(|_| i.allowed)?;
    if i.speed.partial_cmp(&t.min_speed) != Some(std::cmp::Ordering::Greater) || i.in_rock_bail || i.grinding {
        return None;
    }
    let l = i.velocity.mul_add(Vec3::splat(t.long_look_ahead), i.traj);
    let cells = |p: Vec3| {
        let g = to_grid_point(sa, p);
        let c = |v: f32, lo: f32| (v + 0.5).floor().clamp(lo, 255.0);
        Vec3::new(c(g.x, 1.0), c(g.y, 1.0), c(g.z, 0.0))
    };
    let (ft, fl) = (cells(i.traj), cells(l));
    let y_short = round_cell((fl.y - ft.y).mul_add(t.short_look_ahead / t.long_look_ahead, ft.y));
    let (tx, ty, tz) = (round_cell(ft.x), round_cell(ft.y), round_cell(ft.z));
    let (lx, ly, lz) = (round_cell(fl.x), round_cell(fl.y), round_cell(fl.z));
    let (dx, dy, adz) = (lx - tx, ly - ty, (lz - tz).abs());
    if ly <= ty || dy.abs() <= dx.abs() || dy.abs() <= adz {
        return None;
    }
    let len_z = sa.to_world.z_axis.length();
    let len_y = sa.to_world.y_axis.length();
    let per_m = if len_z == 0.0 { f32::INFINITY } else { 1.0 / len_z };
    let limits = [
        round_cell(per_m * t.min_traj_height),
        round_cell(per_m * t.max_traj_height),
        round_cell(per_m * t.min_obstacle_height),
        round_cell(per_m * t.max_obstacle_height),
        round_cell(per_m * (t.min_obstacle_grade * len_y)),
    ];
    let height = |x: i32, y: i32| i32::from(sa.heights[(y as usize).min(GRID - 1) * GRID + (x as usize).min(GRID - 1)]);
    let h0 = height(tx, ty);
    let clearance = tz - h0;
    let (mut max_rel, mut prev_rel, mut obstacle, mut grade, mut first) = (0, 0, 0, 0, -1);
    for row in ty + 1..=ly {
        let k = row - ty - 1;
        let x = tx + (k * dx).div_euclid(dy);
        let reference = h0 + (k * adz).div_euclid(dy);
        let rel = height(x, row) - reference;
        max_rel = max_rel.max(rel);
        if row < y_short {
            grade = grade.max(rel - prev_rel);
            obstacle = max_rel;
            prev_rel = rel;
        }
        if rel > clearance && first < 0 {
            first = row;
        }
    }
    let trips = clearance >= limits[0]
        && clearance <= limits[1]
        && obstacle >= limits[2]
        && max_rel <= limits[3]
        && grade >= limits[4];
    Some(TripWalk {
        start: (tx, ty, tz),
        end_row: ly,
        clearance,
        obstacle,
        max_height: max_rel,
        grade,
        first,
        limits,
        trips,
    })
}

pub fn trip_test(t: &TripTuning, i: &TripInputs) -> Option<Trip> {
    let w = trip_walk(t, i).filter(|w| w.trips)?;
    let sa = i.sa?;
    let per_m = 1.0 / sa.to_world.z_axis.length();
    let v = i.velocity * t.energy_preservation;
    let lift_cells = per_m.mul_add(t.min_obstacle_clearance, (w.max_height - w.clearance) as f32).max(0.0);
    let lift = Vec3::new(sa.to_world.z_axis.x, sa.to_world.z_axis.y, sa.to_world.z_axis.z) * lift_cells;
    let mut dp = v.mul_add(Vec3::splat(i.dt), lift);
    if w.first >= 0 {
        let rows = (w.first - w.start.1) as f32;
        let mut g = rotate(&sa.to_grid, dp);
        if g.y - rows >= 0.0 {
            g.y = rows;
        }
        dp = rotate(&sa.to_world, g);
    }
    let a = (v - i.velocity) / i.dt;
    let f = (i.speed - t.min_speed) / (t.max_damage_speed - t.min_speed);
    let damage = if f - 1.0 >= 0.0 { 1.0 } else { f } * t.damage_amount;
    Some(Trip {
        dp,
        v,
        a,
        damage,
        speed: i.speed,
        clearance: w.clearance,
        obstacle: w.obstacle,
        max_height: w.max_height,
        grade: w.grade,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    fn grid(heights: impl Fn(usize) -> u8) -> SurfaceAnalysis {
        let (s, h) = (15.9375, 31.875);
        let rows = [
            Vec4::new(s, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, h, 0.0),
            Vec4::new(0.0, s, 0.0, 0.0),
            Vec4::new(127.5, 0.0, 100.0, 1.0),
        ];
        let mut sa = SurfaceAnalysis::from_rows(rows);
        for y in 0..GRID {
            for x in 0..GRID {
                sa.heights[y * GRID + x] = heights(y);
            }
        }
        sa
    }

    fn inputs(sa: &SurfaceAnalysis) -> TripInputs<'_> {
        TripInputs {
            allowed: true,
            sa: Some(sa),
            traj: Vec3::new(0.0, 1.0, 1.0),
            velocity: Vec3::new(0.0, 0.0, 0.5),
            speed: 0.5,
            in_rock_bail: false,
            grinding: false,
            dt: 2.0,
        }
    }

    #[test]
    fn a_step_ahead_trips() {
        let t = crate::testutil::trip();
        let sa = grid(|y| if (32..=40).contains(&y) { 122 } else { 100 });
        let w = trip_walk(&t, &inputs(&sa)).expect("walk");
        assert_eq!((w.start, w.end_row, w.clearance), ((128, 16, 132), 72, 32));
        assert_eq!((w.obstacle, w.max_height, w.grade, w.first), (22, 22, 22, -1));
        assert_eq!(w.limits, [5, 40, 15, 57, 4]);
        let trip = trip_test(&t, &inputs(&sa)).expect("trip");
        assert!((trip.v - Vec3::new(0.0, 0.0, 0.375)).length() < 1e-6);
        let up = (19.125 - 10.0) / 31.875;
        assert!((trip.dp - Vec3::new(0.0, up, 0.75)).length() < 1e-5, "{:?}", trip.dp);
        assert!((trip.damage - 0.3 * (0.5 - t.min_speed) / (0.6 - t.min_speed)).abs() < 1e-6);
        for f in [
            |i: &mut TripInputs| i.allowed = false,
            |i: &mut TripInputs| i.speed = 0.2,
            |i: &mut TripInputs| i.in_rock_bail = true,
            |i: &mut TripInputs| i.grinding = true,
        ] {
            let mut i = inputs(&sa);
            f(&mut i);
            assert!(trip_test(&t, &i).is_none());
        }
    }

    #[test]
    fn walls_and_ramps_do_not_trip() {
        let t = crate::testutil::trip();
        let wall = grid(|y| if y >= 32 { 164 } else { 100 });
        assert!(!trip_walk(&t, &inputs(&wall)).expect("walk").trips, "64 cells > 57");
        let ramp = grid(|y| (100 + y.saturating_sub(16)).min(255) as u8);
        let w = trip_walk(&t, &inputs(&ramp)).expect("walk");
        assert!(!w.trips && w.grade == 1, "{w:?}");
        assert!(trip_test(&t, &inputs(&grid(|_| 100))).is_none());
    }
}
