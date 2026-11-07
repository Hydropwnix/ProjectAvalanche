use super::*;

fn pose(p: [f32; 3], v: [f32; 3], angle: f32) -> GhostPose {
    let q = glam::Quat::from_rotation_y(angle).to_array();
    GhostPose {
        trajectory_rotation: [0.0, 0.0, 0.0, 1.0],
        trajectory_position: p,
        rotations: [q; 29],
        translations: [[0.0; 3]; 12],
        flags: 0,
        classic_rig: false,
        velocity: v,
        snow_spray_carving: [0.0; 3],
        snow_spray_speed: 0.0,
        landing_impact: 0.0,
        snow_coverage: 0.0,
    }
}

fn st(tick: f64) -> Stamp {
    Stamp { tick, execution: tick as u32 }
}

fn flags(epoch: u8) -> SampleFlags {
    SampleFlags { epoch, ..Default::default() }
}

#[test]
fn order_and_epochs() {
    let mut l = LivePose::default();
    assert_eq!(l.push(st(10.0), flags(4), pose([0.0; 3], [0.0; 3], 0.0), 13.0), Push::Snapped, "the first sample");
    assert_eq!(l.push(st(12.0), flags(4), pose([1.0, 0.0, 0.0], [0.0; 3], 0.0), 15.0), Push::Accepted);
    assert_eq!(l.push(st(12.0), flags(4), pose([9.0; 3], [0.0; 3], 0.0), 15.0), Push::Stale);
    assert_eq!(l.push(st(11.0), flags(4), pose([9.0; 3], [0.0; 3], 0.0), 15.0), Push::Stale);
    assert_eq!(l.push(st(9.0), flags(5), pose([500.0, 0.0, 0.0], [0.0; 3], 0.0), 16.0), Push::Snapped);
    assert_eq!(l.push(st(14.0), flags(4), pose([2.0, 0.0, 0.0], [0.0; 3], 0.0), 17.0), Push::OlderEpoch);
    assert_eq!(l.view(16.0).map(|v| v.root), Some(Vec3::new(500.0, 0.0, 0.0)), "no blend across the snap");
    let mut w = LivePose::default();
    w.push(st(1.0), flags(255), pose([0.0; 3], [0.0; 3], 0.0), 1.0);
    assert_eq!(w.push(st(3.0), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 3.0), Push::Snapped);
    assert_eq!(w.push(st(5.0), flags(200), pose([0.0; 3], [0.0; 3], 0.0), 5.0), Push::OlderEpoch);
}

#[test]
fn the_root_extrapolates_by_its_age_and_holds_past_the_cap() {
    let mut l = LivePose::default();
    l.push(st(98.0), flags(0), pose([-2.0, 0.0, 0.0], [0.5, 0.0, 0.0], 0.0), 98.0);
    l.push(st(100.0), flags(0), pose([0.0; 3], [0.5, 0.0, 0.0], 0.0), 100.0);
    assert_eq!(l.cap_ticks(), 4.0);
    let at = |l: &LivePose, now: f64| l.view(now).map(|v| v.root.x);
    assert_eq!(at(&l, 100.0), Some(0.0));
    assert_eq!(at(&l, 103.0), Some(3.0), "3 ticks (100 ms) old: 3 m ahead");
    assert_eq!(at(&l, 104.0), Some(4.0), "the cap: the arrivals' 4 ticks");
    assert_eq!(at(&l, 130.0), Some(4.0), "held past the cap");
    l.push(st(102.0), flags(0), pose([2.0, 0.0, 0.0], [0.5, 0.0, 0.0], 0.0), 105.0);
    assert_eq!(l.cap_ticks(), 7.0);
    l.fixed_cap_s = Some(S02_EXTRAPOLATION_S);
    assert_eq!(l.cap_ticks(), 6.0);
    assert_eq!(at(&l, 130.0), Some(8.0), "the sample at 2 m, held 6 ticks ahead");
    l.mode = LiveMode::Interpolate;
    assert_eq!(at(&l, 103.0), Some(-2.0));
}

#[test]
fn the_trajectory_rotation_turns_on_with_the_root() {
    let mut l = LivePose::default();
    let turned = |p: GhostPose, angle: f32| GhostPose {
        trajectory_rotation: glam::Quat::from_rotation_y(angle).to_array(),
        ..p
    };
    l.push(st(98.0), flags(0), turned(pose([0.0; 3], [0.0; 3], 0.0), 0.0), 98.0);
    l.push(st(100.0), flags(0), turned(pose([0.0; 3], [0.0; 3], 0.0), 0.2), 100.0);
    let yaw = |l: &LivePose, now: f64| {
        let q = glam::Quat::from_array(l.view(now).unwrap().rotation);
        let (axis, angle) = q.to_axis_angle();
        angle * axis.y.signum()
    };
    assert!(yaw(&l, 100.0).abs() < 1e-5);
    assert!((yaw(&l, 102.0) - 0.4).abs() < 1e-5, "{}", yaw(&l, 102.0));
    assert!((yaw(&l, 104.0) - 0.6).abs() < 1e-5, "{}", yaw(&l, 104.0));
    assert!((yaw(&l, 110.0) - 0.6).abs() < 1e-5, "held past the cap");
    assert!(glam::Quat::from_array(l.view(104.0).unwrap().pose.trajectory_rotation).w > 0.99);
}

#[test]
fn a_new_samples_jump_is_blended_out_over_one_interval() {
    let mut l = LivePose::default();
    l.push(st(100.0), flags(0), pose([0.0; 3], [0.5, 0.0, 0.0], 0.0), 102.0);
    let before = l.view(104.0).map(|v| v.root.x);
    assert_eq!(before, Some(4.0));
    assert_eq!(l.push(st(102.0), flags(0), pose([1.5, 0.0, 0.0], [0.5, 0.0, 0.0], 0.0), 104.0), Push::Accepted);
    assert_eq!(l.view(104.0).map(|v| v.root.x), before, "no jump at the arrival");
    let mid = l.view(105.0).map(|v| v.root.x).unwrap();
    assert!((mid - 4.75).abs() < 1e-5, "{mid}");
    assert_eq!(l.view(106.0).map(|v| v.root.x), Some(5.5), "the correction is gone after the interval");
}

#[test]
fn the_joints_play_one_interval_behind_the_arrivals() {
    let mut l = LivePose::default();
    for k in 0..10u32 {
        let t = 100 + 2 * k;
        l.push(st(f64::from(t)), flags(0), pose([k as f32, 0.0, 0.0], [0.0; 3], 0.1 * k as f32), f64::from(t) + 3.0);
    }
    assert_eq!(l.delay_ticks(), 5.0, "the arrival delay 3 plus the interval 2");
    let p = |k: u32| pose([k as f32, 0.0, 0.0], [0.0; 3], 0.1 * k as f32);
    assert_eq!(l.view(121.0).unwrap().pose, GhostPose::blend(&p(7), &p(8), 1.0));
    assert_eq!(l.view(122.0).unwrap().pose, GhostPose::blend(&p(8), &p(9), 0.5));
}

#[test]
fn teleports_respawns_and_a_rewinds_start_snap() {
    for f in [
        SampleFlags { teleport: true, ..flags(0) },
        SampleFlags { respawn: true, ..flags(0) },
        SampleFlags { rewind: true, ..flags(0) },
    ] {
        let mut l = LivePose::default();
        l.push(st(10.0), flags(0), pose([0.0; 3], [0.5, 0.0, 0.0], 0.0), 10.0);
        assert_eq!(l.push(st(12.0), f, pose([800.0, 0.0, 0.0], [0.5, 0.0, 0.0], 0.0), 12.0), Push::Snapped, "{f:?}");
        let root = l.view(14.0).unwrap().root.x;
        let want = if f.rewind { 800.0 } else { 802.0 };
        assert_eq!(root, want, "{f:?}");
    }
    let mut l = LivePose::default();
    l.push(st(10.0), SampleFlags { rewind: true, ..flags(0) }, pose([5.0, 0.0, 0.0], [0.0; 3], 0.0), 10.0);
    let next = l.push(st(12.0), SampleFlags { rewind: true, ..flags(0) }, pose([4.0, 0.0, 0.0], [0.0; 3], 0.0), 12.0);
    assert_eq!(next, Push::Accepted);
}

#[test]
fn the_buffer_is_bounded() {
    let mut l = LivePose::default();
    for k in 0..(MAX_SAMPLES as u32 * 3) {
        l.push(st(f64::from(2 * k)), flags(0), pose([0.0; 3], [0.0; 3], 0.0), f64::from(2 * k));
    }
    assert_eq!(l.samples.len(), MAX_SAMPLES);
    assert!(l.delays.len() <= DELAY_WINDOW);
}

fn race_with(live: &[(u16, f32)]) -> crate::session::bots::BotRace {
    let mut race = crate::session::bots::BotRace::default();
    for &(id, x) in live {
        race.add_live(id);
        let r = race.live_mut(id).unwrap();
        r.pose.push(st(10.0), flags(0), pose([x, 0.0, 0.0], [0.0; 3], 0.0), 10.0);
    }
    race
}

#[test]
fn the_free_slots_hold_the_nearest_contiguously_and_are_chosen_once_a_second() {
    let mut race = race_with(&[(1, 400.0), (2, 10.0), (3, 50.0), (4, 900.0), (5, 20.0), (6, 300.0), (7, 30.0)]);
    let changed = race.update_live(10.0, Vec3::ZERO);
    let occupants: Vec<(usize, u16)> = race.live_slots.occupants().collect();
    assert_eq!(occupants, [(0, 2), (1, 5), (2, 7), (3, 3), (4, 6), (5, 1)], "the six nearest, nearest first");
    assert_eq!(changed, [0, 1, 2, 3, 4, 5]);
    race.live_mut(4).unwrap().pose.push(st(12.0), flags(0), pose([1.0, 0.0, 0.0], [0.0; 3], 0.0), 12.0);
    assert!(race.update_live(20.0, Vec3::ZERO).is_empty());
    let changed = race.update_live(40.0, Vec3::ZERO);
    let occupants: Vec<(usize, u16)> = race.live_slots.occupants().collect();
    assert_eq!(occupants, [(0, 2), (1, 5), (2, 7), (3, 3), (4, 6), (5, 4)], "the others keep their slots");
    assert_eq!(changed, [5]);
    race.remove_live(5);
    let changed = race.update_live(41.0, Vec3::ZERO);
    let occupants: Vec<(usize, u16)> = race.live_slots.occupants().collect();
    assert_eq!(occupants, [(0, 2), (1, 1), (2, 7), (3, 3), (4, 6), (5, 4)]);
    assert_eq!(changed, [1]);
}

#[test]
fn racers_without_samples_wait() {
    let mut race = race_with(&[(1, 5.0)]);
    race.add_live(2);
    race.update_live(10.0, Vec3::ZERO);
    let occupants: Vec<(usize, u16)> = race.live_slots.occupants().collect();
    assert_eq!(occupants, [(0, 1)], "rider 2 has sent nothing yet");
}

#[test]
fn a_rounds_slots_follow_the_standings_order() {
    let mut race = race_with(&[(1, 400.0), (2, 10.0), (3, 50.0), (4, 900.0), (5, 20.0), (6, 300.0), (7, 30.0)]);
    race.add_live(8);
    race.set_live_order(Some(vec![4, 8, 6, 1, 3, 7, 5, 2]));
    race.update_live(10.0, Vec3::ZERO);
    let occupants: Vec<u16> = race.live_slots.occupants().map(|(_, id)| id).collect();
    assert_eq!(occupants, [4, 6, 1, 3, 7, 5], "the order's first six with samples (rider 8 has none)");
    race.set_live_order(None);
    race.update_live(50.0, Vec3::ZERO);
    let mut occupants: Vec<u16> = race.live_slots.occupants().map(|(_, id)| id).collect();
    occupants.sort_unstable();
    assert_eq!(occupants, [1, 2, 3, 5, 6, 7], "the nearest by distance");
}

#[test]
fn a_stall_of_the_servers_time_does_not_pause_the_racer() {
    let mut l = LivePose::default();
    let stamp = |x: u32| f64::from(1000 + x) - if x >= 60 { 52.0 } else { 0.0 };
    let mut drawn = Vec::new();
    for x in (0..120u32).step_by(2) {
        let at = Stamp { tick: stamp(x), execution: x };
        let now = stamp(x) + 3.0;
        let pushed = l.push(at, flags(0), pose([x as f32, 0.0, 0.0], [0.5, 0.0, 0.0], 0.0), now);
        assert_ne!(pushed, Push::Stale, "execution {x}");
        for k in 0..2 {
            let truth = (x + 3 + k) as f32;
            drawn.push((x + k, l.view(now + f64::from(k)).map(|v| v.root.x).unwrap_or(f32::NAN), truth));
        }
    }
    let ticks: Vec<f64> = l.samples.iter().map(|s| s.tick).collect();
    assert!(ticks.windows(2).all(|w| w[1] - w[0] >= 2.0 - 1e-9), "{ticks:?}");
    for &(x, root, truth) in drawn.iter().filter(|(x, ..)| *x >= 64) {
        assert!((root - truth).abs() < 0.5, "execution {x}: drawn {root}, the owner {truth}");
    }
}

#[test]
fn equal_counts_are_ordered_by_their_stamps() {
    let mut l = LivePose::default();
    let at = |tick: f64, execution: u32| Stamp { tick, execution };
    assert_eq!(l.push(at(10.0, 0), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 12.0), Push::Snapped);
    assert_eq!(l.push(at(12.0, 0), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 14.0), Push::Accepted);
    assert_eq!(l.push(at(11.0, 0), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 14.0), Push::Stale);
    assert_eq!(l.push(at(9.0, 3), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 15.0), Push::Accepted, "a later count");
    assert_eq!(l.push(at(30.0, 2), flags(0), pose([0.0; 3], [0.0; 3], 0.0), 15.0), Push::Stale, "an earlier count");
    let ticks: Vec<f64> = l.samples.iter().map(|s| s.tick).collect();
    assert_eq!(ticks, [4.0, 6.0, 9.0]);
}
