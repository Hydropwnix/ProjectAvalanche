use glam::Vec3;
use ssx_formats::vmx;

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
    pub damage_share: f32,
    pub damage: f32,
    pub speed: f32,
    pub clearance: i32,
    pub obstacle: i32,
    pub max_height: i32,
    pub grade: i32,
}

impl Trip {
    pub fn apply_damage(&self, t: &TripTuning, one_off: f32) -> f32 {
        self.damage_share.mul_add(t.damage_amount, one_off)
    }
}

fn round_cell(x: f32) -> i32 {
    vmx::floor(vmx::add(x, 0.5)) as i32
}

fn to_grid_point(sa: &SurfaceAnalysis, p: Vec3) -> Vec3 {
    let m = &sa.to_grid;
    let f =
        |k: usize| vmx::madd(m.z_axis[k], p.z, vmx::madd(m.y_axis[k], p.y, vmx::madd(m.x_axis[k], p.x, m.w_axis[k])));
    Vec3::new(f(0), f(1), f(2))
}

fn rotate(rows: &glam::Mat4, v: Vec3) -> Vec3 {
    let f = |k: usize| vmx::madd(rows.z_axis[k], v.z, vmx::madd(rows.y_axis[k], v.y, vmx::mul(rows.x_axis[k], v.x)));
    Vec3::new(f(0), f(1), f(2))
}

fn row_length(r: glam::Vec4) -> f32 {
    let a = r.to_array();
    crate::intercept::sqrt(vmx::dot4(a, a))
}

fn scale(v: Vec3, k: f32) -> Vec3 {
    Vec3::new(vmx::mul(v.x, k), vmx::mul(v.y, k), vmx::mul(v.z, k))
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
    let (v, p) = (i.velocity, i.traj);
    let l = Vec3::new(
        vmx::madd(v.x, t.long_look_ahead, p.x),
        vmx::madd(v.y, t.long_look_ahead, p.y),
        vmx::madd(v.z, t.long_look_ahead, p.z),
    );
    let cells = |p: Vec3| {
        let g = to_grid_point(sa, p);
        let c = |v: f32, lo: f32| vmx::min(vmx::max(vmx::floor(vmx::add(v, 0.5)), lo), 255.0);
        Vec3::new(c(g.x, 1.0), c(g.y, 1.0), c(g.z, 0.0))
    };
    let (ft, fl) = (cells(i.traj), cells(l));
    let ratio = vmx::mul(vmx::recip(t.long_look_ahead), t.short_look_ahead);
    let y_short = round_cell(vmx::madd(vmx::sub(fl.y, ft.y), ratio, ft.y));
    let (tx, ty, tz) = (round_cell(ft.x), round_cell(ft.y), round_cell(ft.z));
    let (lx, ly, lz) = (round_cell(fl.x), round_cell(fl.y), round_cell(fl.z));
    let (dx, dy, adz) = (lx - tx, ly - ty, (lz - tz).abs());
    if ly <= ty || dy.abs() <= dx.abs() || dy.abs() <= adz {
        return None;
    }
    let len_y = row_length(sa.to_world.y_axis);
    let per_m = vmx::recip(row_length(sa.to_world.z_axis));
    let limit = |x: f32| vmx::floor(vmx::madd(per_m, x, 0.5)) as i32;
    let limits = [
        limit(t.min_traj_height),
        limit(t.max_traj_height),
        limit(t.min_obstacle_height),
        limit(t.max_obstacle_height),
        limit(vmx::mul(t.min_obstacle_grade, len_y)),
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
    let per_m = vmx::recip(row_length(sa.to_world.z_axis));
    let v = scale(i.velocity, t.energy_preservation);
    let lift = vmx::max(vmx::madd(per_m, t.min_obstacle_clearance, (w.max_height - w.clearance) as f32), 0.0);
    let lift = rotate(&sa.to_world, Vec3::new(vmx::mul(0.0, lift), vmx::mul(0.0, lift), lift));
    let mut dp = Vec3::new(vmx::madd(v.x, i.dt, lift.x), vmx::madd(v.y, i.dt, lift.y), vmx::madd(v.z, i.dt, lift.z));
    if w.first >= 0 {
        let rows = (w.first - w.start.1) as f32;
        let mut g = rotate(&sa.to_grid, dp);
        if g.y - rows >= 0.0 {
            g.y = rows;
        }
        dp = rotate(&sa.to_world, g);
    }
    let a = scale(
        Vec3::new(vmx::sub(v.x, i.velocity.x), vmx::sub(v.y, i.velocity.y), vmx::sub(v.z, i.velocity.z)),
        vmx::recip(i.dt),
    );
    let f = (i.speed - t.min_speed) / (t.max_damage_speed - t.min_speed);
    let share = if f - 1.0 >= 0.0 { 1.0 } else { f };
    Some(Trip {
        dp,
        v,
        a,
        damage_share: share,
        damage: share * t.damage_amount,
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

    pub(super) struct Case {
        pub t: TripTuning,
        pub sa: SurfaceAnalysis,
        pub traj: Vec3,
        pub velocity: Vec3,
        pub speed: f32,
        pub dt: f32,
        pub one_off: f32,
        pub flags: [i32; 4],
        pub tripped: bool,
        pub walk: Option<Vec<i32>>,
        pub out: [f32; 10],
    }

    pub(super) fn parse_case(line: &str) -> Case {
        let f = |h: &str| f32::from_bits(u32::from_str_radix(h, 16).unwrap());
        let (ins, outs) = line.split_once(" | ").unwrap();
        let (fl, ints) = ins.split_once(" ; ").unwrap();
        let x: Vec<f32> = fl.split(' ').map(f).collect();
        let k: Vec<i32> = ints.split(' ').map(|v| v.parse().unwrap()).collect();
        let mut o = outs.split(" ; ");
        let tripped = o.next().unwrap() == "1";
        let w = o.next().unwrap();
        let walk = (w != "-").then(|| w.split(' ').map(|v| v.parse().unwrap()).collect());
        let out: Vec<f32> = o.next().unwrap().split(' ').map(f).collect();
        let t = TripTuning {
            short_look_ahead: x[0],
            long_look_ahead: x[1],
            min_traj_height: x[2],
            max_traj_height: x[3],
            min_obstacle_height: x[4],
            max_obstacle_height: x[5],
            min_obstacle_grade: x[6],
            energy_preservation: x[7],
            min_obstacle_clearance: x[8],
            min_speed: x[9],
            max_damage_speed: x[10],
            damage_amount: x[11],
        };
        let m = |o: usize| {
            let r = |i: usize| Vec4::new(x[o + 4 * i], x[o + 4 * i + 1], x[o + 4 * i + 2], x[o + 4 * i + 3]);
            glam::Mat4::from_cols(r(0), r(1), r(2), r(3))
        };
        let mut sa = SurfaceAnalysis::default();
        (sa.to_grid, sa.to_world) = (m(12), m(28));
        let [b0, sn, sd, cx, cm, r0, r1, c0, c1, hs] = std::array::from_fn(|i| k[4 + i]);
        for y in 0..GRID as i32 {
            for xx in 0..GRID as i32 {
                let step = if (r0..=r1).contains(&y) && (c0..=c1).contains(&xx) { hs } else { 0 };
                let h = b0 + (y * sn).div_euclid(sd) + (xx * cx) % cm + step;
                sa.heights[(y * GRID as i32 + xx) as usize] = h.clamp(0, 255) as u8;
            }
        }
        Case {
            t,
            sa,
            traj: Vec3::new(x[44], x[45], x[46]),
            velocity: Vec3::new(x[47], x[48], x[49]),
            speed: x[50],
            dt: x[51],
            one_off: x[52],
            flags: [k[0], k[1], k[2], k[3]],
            tripped,
            walk,
            out: std::array::from_fn(|i| out[i]),
        }
    }

    const CASES: [&str; 12] = [
        "402b460c 40c8910c 3e54ca49 3f92033b 3f103f80 400a6b47 3fec8e57 3f4a40f4 3f00f8a6 3e5d1066 3f13ff03 3ead18ae \
         c13ea2bd bf6a3650 3efe346b 00000000 00000000 00000000 42039afd 00000000 3f1d1731 c18e1ce6 bf898543 00000000 \
         c617e7c3 c68a8c27 47089f12 3f800000 bdab6eef 3ab80505 3b8d4466 00000000 bb3d8050 baea9587 bd65f7a0 00000000 \
         00000000 3cf8fc98 80000000 00000000 c4584523 c48723c9 c46e705e 3f800000 c45b6143 c4865f9d c46f1a87 be5dac92 \
         bcede993 beca2328 3ee70278 3f000000 3fe4eb42 ; 1 1 0 0 156 1 3 0 7 77 88 0 255 19 | 1 ; 147 59 198 104 23 24 \
         27 19 77 7 38 19 71 3 ; bdaf226a 3f1d285e be1fb307 be2f226a bcbbf6ab be9fb307 3dba28a0 3c47cba0 3e29c084 \
         400083e6",
        "40200000 40e00000 3e23d70a 3fa147ae 3ef5c28f 3fe66666 40000000 3f400000 3f19999a 3e6eeeef 3f19999a 3e99999a \
         c182725c 410446b8 c0241251 00000000 00000000 00000000 41e11237 00000000 c11f7cf6 c1586139 bfdf4b19 00000000 \
         c596ddc3 c69d79e3 46d6e779 3f800000 bd36dc9c bbbcc0e5 bcdf9285 00000000 3d06c868 b9b1da37 bd5c7ae7 00000000 \
         80000000 3d1196eb 00000000 00000000 43dfe0a5 c47d18ff c498188a 3f800000 43dd9423 c47cb168 c498b4f4 3d97953f \
         bbd4b66b be99afe3 3ec1e05a 3fd00ec4 3f4ae14c ; 1 1 0 0 35 1 2 1 5 39 47 0 255 33 | 1 ; 124 28 66 61 13 36 41 \
         33 39 5 35 14 51 4 ; 3db8cb03 3fcb2d8b bebb5ba8 3d635fde bb9f88d0 be6687d4 bc3a8311 3a82dd1f 3d3d19d3 \
         3f695206",
        "40200000 40e00000 3e23d70a 3fa147ae 3ef5c28f 3fe66666 40000000 3f400000 3f19999a 3e6eeeef 3f19999a 3e99999a \
         3f17db75 c166a627 c0dc6cc1 00000000 00000000 00000000 42333bc5 00000000 417d56dc 3f0a4196 c0e81496 00000000 \
         46a343d1 c5d0f119 4740f9b1 3f800000 3b1ad8b0 3c2d32a0 3d8129bd 00000000 bd8dde9a bc279782 3b2a145f 00000000 \
         80000000 3cb6d2b0 80000000 00000000 c400229a c4adfa2b c4a29452 3f800000 c40102b2 c4ad832b c4a2469b bea8a1a3 \
         bd752189 bcce01f1 3eabe027 3f800000 3f674473 ; 1 1 0 0 130 0 6 1 3 56 74 0 255 43 | 1 ; 36 52 173 85 43 45 \
         45 43 56 7 56 22 81 6 ; be7cf273 3f198934 bc9a8175 be7cf274 bd37d927 bc9a8175 3da8a1a4 3c752188 3bce01f0 \
         3f7cb518",
        "402b40ec 40c93ad5 3e0d4f2e 3fce3964 3f1424d5 3fac3d97 401eeca0 3f1a27e1 3f0a1610 3e90e6db 3f2475b4 3e78f75f \
         413c5eaf 4109e1e5 40a96b78 00000000 00000000 00000000 4218bf98 00000000 c0c30c64 41852947 40cbae52 00000000 \
         c59e1f6e c560364e 466f1799 3f800000 3d892f3c bb65cd87 bd0e0c5e 00000000 3cc8f106 bc391a4c 3d420fa9 00000000 \
         80000000 3cd685cf 80000000 00000000 43d5775e c3e57d8c c0b068e0 3f800000 43dcf395 c3e3ddd3 c123c522 baa02f9d \
         bd873c7d 3e9e2c11 3ea1bf0e 3f000000 3f1024f3 ; 1 1 0 0 86 1 1 1 3 61 62 0 255 24 | 1 ; 205 50 173 83 36 34 \
         34 25 -1 5 62 22 51 5 ; b9c0eb31 3eef2d78 3dbe7e4c ba40eb31 bd22dee9 3e3e7e4c 3a7ee812 3d573422 be7bb3ac \
         3f15d77e",
        "403cb5a2 410a06a5 3e0d7d60 3faed350 3f07c70d 400506dd 3ff33060 3f70a814 3f17dfaa 3e92aaa9 3f172811 3ead98e4 \
         4012063f 41819a93 3df4cbe5 00000000 00000000 00000000 420ee965 00000000 c148affe 403c9ad9 40ec6aef 00000000 \
         c5899882 453f6e6f c7134a3a 3f800000 3c65ffdf 3c825849 bd9e0c8b 00000000 3d74bbe4 bb2064a5 3c3212d4 00000000 \
         80000000 3ce549d7 80000000 00000000 c2f26492 448d9d9f c3ba8ad7 3f800000 c2ea63b4 448ebf3c c3c0d09c 3ee0a30e \
         bd845cc3 3dd8e1dc 3ee971f1 3f800000 3ed9e859 ; 1 1 0 0 182 0 3 0 2 44 71 104 178 48 | 1 ; 166 28 231 92 49 \
         45 45 47 -1 5 49 19 74 4 ; 3ed32c5a 3ed75168 3dcbe223 3ed32c5a bd78dbbc 3dcbe223 bcd76b40 3b7ddca0 bbcffb90 \
         3f1d58df",
        "40159a3b 40ce7727 3e142636 3f80f2b1 3f0336a0 3fe7809c 40223b22 3f50d4cf 3f252f7b 3e43c44b 3ef2ca3d 3e96d3be \
         40fa3635 41184778 4066dd53 00000000 00000000 00000000 41a4b5df 00000000 c0cf575f 4137c3cd 405da1f4 00000000 \
         c5e33771 4618a4dd c4e29259 3f800000 3d9b49d0 bb320581 bd80ae86 00000000 3d2f361a bc8485ea 3d537042 00000000 \
         80000000 3d46f16d 80000000 00000000 43056c7e 436252f8 c4704970 3f800000 430a14aa 436acf7e c47078c8 3eb482d8 \
         be1eb774 3eb67ed5 3f06567b 400bcb13 3fd59a79 ; 1 1 0 0 176 -1 4 1 4 44 73 0 255 22 | 1 ; 41 36 189 84 21 20 \
         20 22 -1 3 21 11 37 4 ; 3f20d184 3ea419fc 3f229616 3e93405e be0178ff 3e94dec2 bcf3a16b 3c563711 bcf64f07 \
         3ffb4f68",
        "402fb00b 40e44108 3e084210 3fa72c86 3f0cacc8 3fe06284 400c43c1 3f747dc2 3f2656a3 3e5a602f 3f4604fd 3e881f87 \
         41608884 c0d59a7d 40124001 00000000 00000000 00000000 41ca1205 00000000 40ef1973 4148972f c0a406f5 00000000 \
         4350db6a 4675e629 c4f346e7 3f800000 3d6368a5 3a7fda36 3cf22955 00000000 bd078876 3c7facda 3d7e8d72 00000000 \
         80000000 3d22294c 80000000 00000000 43fe92a8 c328bd6c c4760d5d 3f800000 4400101a c3239fd2 c475283d 3d584e74 \
         3e175f66 3ea0dcd2 3eb3d2a7 3fbd722d 3fd05526 ; 1 1 0 0 100 0 7 0 1 75 105 41 216 16 | 0 ; 70 24 118 50 18 0 \
         0 0 -1 3 33 14 44 4 ; 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
         3fd05526",
        "404e3039 40da23f7 3de5ff29 3fae8bbf 3ed1929a 40039639 3ff3eb2d 3f2ebfbc 3f426db7 3e83c613 3f45e0b3 3ebc0ee7 \
         c0103a73 c1ab34b3 40e801ab 00000000 00000000 00000000 421b1ec4 00000000 4138126c c08625ad bf555974 00000000 \
         c647d26c 468f39d9 45958f77 3f800000 bc865471 3b9f6778 3dab7063 00000000 bd385298 3c06bcea bc106cbb 00000000 \
         00000000 3cd33e1f 80000000 00000000 4419d28c c353f742 449a049f 3f800000 4418cc01 c3511425 449a972a be742c4b \
         3d57ca43 3d591d56 3f3ae319 3f800000 3fb070b8 ; 1 1 0 0 51 1 7 0 3 97 108 0 255 61 | 0 ; 62 69 78 102 18 3 65 \
         1 97 4 53 16 80 3 ; 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
         3fb070b8",
        "3fe69e93 40b32bff 3e10e223 3fc24cc4 3ebcfc32 3fb70c1b 401897a3 3f537cdf 3eeb7d98 3e8fbb83 3f30d780 3e7b4b35 \
         41099825 417a37be 4103d696 00000000 00000000 00000000 420c0fac 00000000 c17f42a8 4106e04c c0c57e00 00000000 \
         c69366e1 c5c19e53 46f177c4 3f800000 3cd67839 bc7131e0 bd46f03d 00000000 3d4af298 bbe3cff6 3cdacab5 00000000 \
         80000000 3ce9f470 80000000 00000000 44483c6d c49670d3 c43bb943 3f800000 44493456 c4965347 c43d33d4 3e9bdfc1 \
         bd0d174d 3e785b28 3ec811e1 40000000 3f2baee2 ; 1 1 0 0 36 -1 2 4 3 62 84 16 162 56 | 0 ; 128 11 101 49 69 0 \
         0 0 -1 5 53 13 50 5 ; 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 \
         3f2baee2",
        "40200000 40e00000 3e23d70a 3fa147ae 3ef5c28f 3fe66666 40000000 3f400000 3f19999a 3e6eeeef 3f19999a 3e99999a \
         412e5614 4037b490 3e6cfdec 00000000 00000000 00000000 41f43feb 00000000 bfdacdea 41925ed7 40894b0e 00000000 \
         463ee97b c64df4a6 c4e5581a 3f800000 3db77190 3aa9d59b bc663c18 00000000 3c091ca4 bbf7b57c 3d5a7e1d 00000000 \
         80000000 3d062866 80000000 00000000 c4760771 c25d8535 445ab471 3f800000 c471c6b5 c24afd63 445a9e33 3d904b1e \
         bd4a174c 3eb5a642 3ebae95e 3f800000 3e49f0ff ; 0 1 0 0 111 0 1 1 1 64 74 0 255 38 | 0 ; - ; 00000000 \
         00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 3e49f0ff",
        "402430b4 410505e6 3e0b90a7 3f99b543 3f186f73 3fdec119 3fe7d9b6 3f6d3240 3f368408 3e93cb15 3f2ae70b 3e918c19 \
         c0dfc864 c1846dcc c04c1c08 00000000 00000000 00000000 41cfaf51 00000000 41958a46 c0c62d20 bfb43459 00000000 \
         4680049d c4e5a70d c561f26b 3f800000 bc8fd3a8 39cb7cfa 3d403894 00000000 bd590edb bbf88d34 bca26927 00000000 \
         00000000 3d1dc70c 80000000 00000000 433e55b7 42edeb5f c4495aa3 3f800000 433a1058 42fae3b6 c446b087 be11ec1b \
         bce58eee bcb4bc9f 3e166c50 3f800000 3f2d2493 ; 0 1 0 0 162 1 1 2 2 18 39 110 173 -9 | 0 ; - ; 00000000 \
         00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 3f2d2493",
        "40200000 40e00000 3e23d70a 3fa147ae 3ef5c28f 3fe66666 40000000 3f400000 3f19999a 3e6eeeef 3f19999a 3e99999a \
         412e5614 4037b490 3e6cfdec 00000000 00000000 00000000 41f43feb 00000000 bfdacdea 41925ed7 40894b0e 00000000 \
         463ee97b c64df4a6 c4e5581a 3f800000 3db77190 3aa9d59b bc663c18 00000000 3c091ca4 bbf7b57c 3d5a7e1d 00000000 \
         80000000 3d062866 80000000 00000000 c4760771 c25d8535 445ab471 3f800000 c471c6b5 c24afd63 445a9e33 3d904b1e \
         bd4a174c 3eb5a642 3ebae95e 3f800000 3e49f0ff ; 0 1 0 0 111 0 1 1 1 64 74 0 255 38 | 0 ; - ; 00000000 \
         00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 3e49f0ff",
    ];

    #[test]
    fn trip_test_matches_the_executable() {
        for line in CASES {
            let c = parse_case(line);
            let i = TripInputs {
                allowed: c.flags[0] != 0,
                sa: (c.flags[1] != 0).then_some(&c.sa),
                traj: c.traj,
                velocity: c.velocity,
                speed: c.speed,
                in_rock_bail: c.flags[2] != 0,
                grinding: c.flags[3] != 0,
                dt: c.dt,
            };
            let got = trip_walk(&c.t, &i).map(|w| {
                let mut v = vec![w.start.0, w.start.1, w.start.2, w.end_row];
                v.extend([w.clearance, w.obstacle, w.max_height, w.grade, w.first]);
                v.extend(w.limits);
                v
            });
            assert_eq!(got, c.walk, "{line}");
            let trip = trip_test(&c.t, &i);
            assert_eq!(trip.is_some(), c.tripped, "{line}");
            let mut o = [0.0f32; 10];
            o[9] = c.one_off;
            if let Some(tr) = trip {
                o[..3].copy_from_slice(&tr.dp.to_array());
                o[3..6].copy_from_slice(&tr.v.to_array());
                o[6..9].copy_from_slice(&tr.a.to_array());
                o[9] = tr.apply_damage(&c.t, c.one_off);
            }
            assert_eq!(o.map(f32::to_bits), c.out.map(f32::to_bits), "{line}");
        }
    }
}
