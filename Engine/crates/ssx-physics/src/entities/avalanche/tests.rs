use super::emit::*;
use super::*;

fn slough() -> ParticleTraits {
    ParticleTraits {
        gravity: 60.0,
        h_terminal: 180.0,
        v_terminal: 240.0,
        restitution: 0.02,
        stop_friction: 0.2,
        roll_friction: 0.01,
        energy_preservation: 0.6,
        stopping_speed: 0.15,
        surface_noise: 0.1,
        roll_factor: 0.4,
        max_lifetime: 600.0,
        max_lifetime_frozen: 15.0,
        aging_ground: 1.0,
        aging_air: 1.0,
        spawn_offset_depth: 1.0,
        spawn_offset_duration: 0.0,
    }
}

#[test]
fn the_generator_matches_the_seed() {
    let mut r = Rng::default();
    let a = r.next4();
    assert_eq!(r.0, [0xCEC6_E739, 0xA1C8_3804, 0xCE3C_5E87, 0xF230_09E2]);
    for (x, y) in a.iter().zip([0.553_931_36, 0.564_209_46, 0.471_634_75, 0.375_301_6]) {
        assert!((x - y).abs() < 1e-6, "{a:?}");
    }
}

#[test]
fn the_drag_model_comes_from_the_terminal_velocities() {
    let (kh, kv, g) = slough().drag();
    let g0 = 60.0 / 3600.0;
    assert!((g - g0).abs() < 1e-9);
    assert!((kh - g0 / (180.0f32 / 216.0).powi(2)).abs() < 1e-7);
    assert!((kv - g0 / (240.0f32 / 216.0).powi(2)).abs() < 1e-7);
}

#[test]
fn contact_snaps_bounces_and_freezes() {
    let tr = slough();
    let mut rng = Rng::default();
    let n = Vec3::Y;
    let mut p = Particle {
        pos: Vec3::new(0.0, 0.05, 0.0),
        vel: Vec3::new(0.3, -0.2, 0.0),
        contact: Some((Vec3::ZERO, n)),
        flags: FLAG_SIMULATED,
        id: 1,
        scale: 1.0,
        frame: Quat::IDENTITY,
        ..Particle::default()
    };
    integrate(&mut p, &tr, 0.01, &mut rng, 2.0);
    assert!(p.flags & FLAG_CONTACT != 0 && p.impact_cos > 0.5, "{p:?}");
    assert!((p.pos.y - 0.01).abs() < 1e-6, "snapped to ε: {:?}", p.pos);
    assert!(p.vel.x > 0.0 && p.vel.length() < 0.36, "slowed: {:?}", p.vel);
    let mut slow = Particle { vel: Vec3::new(0.001, -0.01, 0.0), age: 10.0, ..p };
    slow.pos = Vec3::new(0.0, 0.005, 0.0);
    slow.flags = FLAG_SIMULATED;
    integrate(&mut slow, &tr, 0.01, &mut rng, 2.0);
    assert!(slow.flags & FLAG_FROZEN != 0 && slow.vel == Vec3::ZERO, "{slow:?}");
    assert_eq!(slow.age, tr.max_lifetime - tr.max_lifetime_frozen);
    let before = slow;
    integrate(&mut slow, &tr, 0.01, &mut rng, 2.0);
    assert_eq!(slow.pos, before.pos, "frozen records stay");
    let mut air =
        Particle { pos: Vec3::Y * 10.0, vel: Vec3::ZERO, flags: FLAG_SIMULATED, id: 2, ..Particle::default() };
    integrate(&mut air, &tr, 0.01, &mut rng, 2.0);
    assert!((air.vel.y + 2.0 * 60.0 / 3600.0).abs() < 1e-6, "{:?}", air.vel);
}

fn spray_tuning() -> ParticleTuning {
    let spray = ParticleTraits {
        gravity: 9.8,
        h_terminal: 20.0,
        v_terminal: 20.0,
        restitution: 0.1,
        stop_friction: 0.4,
        roll_friction: 0.01,
        energy_preservation: 0.75,
        stopping_speed: 0.0,
        surface_noise: 0.02,
        roll_factor: 10.0,
        max_lifetime: 90.0,
        max_lifetime_frozen: 90.0,
        aging_ground: 1.0,
        aging_air: 1.0,
        spawn_offset_depth: 1.0,
        spawn_offset_duration: 0.0,
    };
    ParticleTuning {
        spray,
        spray_vector_min: 0.005,
        spray_vector_max: 0.023,
        spray_velocity_min: 0.08,
        spray_velocity_max: 0.5,
        spray_velocity_exponent: 3.0,
        spray_density: 20.0,
        spray_width: 0.6,
        spray_height: 0.15,
        spray_scale_min: 0.5,
        spray_scale_max: 2.0,
        ..ParticleTuning::default()
    }
}

#[test]
fn the_spray_throws_simulated_and_attached_chunks() {
    let pt = spray_tuning();
    let inputs = SimInputs {
        vel: Vec3::new(0.0, 0.0, 0.4),
        spray: Vec3::new(0.023, 0.0, 0.0),
        powder_weight: 1.0,
        ..SimInputs::default()
    };
    let mut sim = AvalancheSim { inputs, ..AvalancheSim::default() };
    (sim.tip_prev, sim.tip) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 1.0));
    sim.emit_spray(&pt, 2.0);
    assert_eq!(sim.spawned_spray, 20, "1 m of tip travel × 20 per metre");
    assert!((sim.spray_vel - Vec3::new(0.5, 0.0, 0.0)).length() < 1e-6, "at VecMax: VelMax along the spray");
    let alive: Vec<(usize, &Particle)> = sim.spray.iter().enumerate().filter(|(_, p)| p.alive()).collect();
    assert_eq!(alive.len(), 20);
    let parents: Vec<usize> = alive.iter().filter(|(_, p)| p.flags & FLAG_SIMULATED != 0).map(|(k, _)| *k).collect();
    assert_eq!(parents, vec![0, 1, 2], "records 0, 9 and 18 are simulated");
    for (k, p) in &alive {
        assert_eq!(p.row, 0, "powder");
        assert!((0.5..=2.0).contains(&p.scale) && p.flags & FLAG_ROLLING == 0);
        if *k >= SPRAY_RECORDS {
            let (at, id) = p.parent.expect("attached");
            assert!(at < 3 && sim.spray[at as usize].id == id && p.flags & FLAG_SIMULATED == 0);
        }
    }
    sim.inputs.spray = Vec3::new(0.004, 0.0, 0.0);
    sim.tip_prev = sim.tip;
    sim.tip += Vec3::Z;
    sim.emit_spray(&pt, 2.0);
    assert_eq!((sim.spawned_spray, sim.spray_vel), (20, Vec3::ZERO));
    sim.inputs.spray = Vec3::new(0.014, 0.0, 0.0);
    sim.inputs.ice_weight = 1.0;
    sim.emit_spray(&pt, 2.0);
    let x = (0.014f32 - 0.005) / (0.023 - 0.005);
    assert!((sim.spray_vel.x - (0.08 + 0.42 * x * x * x)).abs() < 1e-5, "{}", sim.spray_vel);
    assert_eq!(sim.spray.iter().filter(|p| p.alive() && p.row == 2).count(), 20);
}

#[test]
fn spray_chunks_vanish_at_their_first_contact() {
    let mut t = crate::testutil::avalanche();
    t.small = spray_tuning();
    let world = crate::testutil::plane(50.0, 0.0);
    let mut sim = AvalancheSim::default();
    let at = Vec3::new(0.0, 0.3, 0.0);
    let v = Vec3::new(0.05, 0.0, 0.0);
    sim.spray[0] = Particle {
        kind: Kind::Spray,
        pos: at,
        vel: v,
        scale: 1.0,
        flags: FLAG_SIMULATED,
        id: 7,
        ..Particle::default()
    };
    let child = Particle { pos: at + Vec3::X * 0.1, id: 8, flags: 0, parent: Some((0, 7)), ..sim.spray[0] };
    sim.spray[SPRAY_RECORDS] = child;
    let inputs = SimInputs { dt: 2.0, in_air: true, ..SimInputs::default() };
    let (mut steps, mut landed_at) = (0, None);
    while sim.spray_particles().count() > 0 && steps < 60 {
        sim.dispatch(&t, inputs, &world);
        sim.gather_spray_records(&t);
        sim.gather(&t, Vec3::ZERO, Vec3::ZERO, Vec3::Z, None);
        steps += 1;
        if landed_at.is_none() && sim.spray_landed > 0 {
            landed_at = Some(steps);
        }
    }
    assert_eq!((sim.spray_landed, sim.spray_expired), (2, 0), "both land within {steps} updates");
    assert!(landed_at.is_some_and(|s| s > 3 && s < 30), "{landed_at:?}");
    assert_eq!(sim.spawned_spray, 0, "nothing thrown in the air");
}

fn flat_grid() -> SurfaceAnalysis {
    let c = 255.0 / 16.0;
    let rows = [
        glam::Vec4::new(c, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 31.875, 0.0),
        glam::Vec4::new(0.0, c, 0.0, 0.0),
        glam::Vec4::new(128.0, 128.0, 31.875, 1.0),
    ];
    let mut sa = SurfaceAnalysis::from_rows(rows);
    sa.heights.fill(32);
    sa
}

#[test]
fn a_landing_throws_impact_slabs_onto_the_surface() {
    let pt = ParticleTuning {
        impact_trigger_force_min: 0.08,
        impact_trigger_force_max: 0.55,
        impact_density_min: 8.0,
        impact_density_max: 16.0,
        impact_density_exponent: 3.0,
        impact_throw: 0.3,
        impact_lift: 0.1,
        impact_spread: 0.3,
        impact_size_ahead: 3.2,
        impact_size_behind: 0.8,
        impact_size_left_right: 7.5,
        slab_scale_min: 0.5,
        slab_scale_max: 0.8,
        ..ParticleTuning::default()
    };
    let sa = flat_grid();
    let sw = AvalancheSwitches { impact_sensitivity: 1.0, ..AvalancheSwitches::default() };
    let inputs = SimInputs {
        ground_valid: true,
        ground_normal: Vec3::Y,
        snow_coverage: 1.0,
        switches: sw,
        ..SimInputs::default()
    };
    let mut sim = AvalancheSim { inputs, ..AvalancheSim::default() };
    (sim.landing_dir, sim.landing_impact, sim.landing_speed) = (Vec3::new(0.0, -0.6, 0.8), 0.55, 0.6);
    sim.tip = Vec3::new(0.0, 0.0, 0.0);
    sim.impact_slabs(&pt, Some(&sa));
    assert_eq!(sim.spawned_slabs, 17, "the tip's slab and 16 at the full force (every cell has a height)");
    let alive: Vec<&Particle> = sim.slabs.iter().filter(|p| p.alive()).collect();
    assert_eq!(alive[0].pos, Vec3::ZERO);
    assert!((alive[0].vel - Vec3::new(0.0, 0.1, 0.18)).length() < 1e-6, "{}", alive[0].vel);
    for p in &alive[1..] {
        assert!((p.pos.y - 0.125 / 31.875).abs() < 1e-5 && p.pos.x.abs() <= 7.6, "{}", p.pos);
        assert!(p.pos.z >= -0.85 && p.pos.z <= 3.25, "ahead 3.2 m, behind 0.8 m: {}", p.pos);
        assert!((p.vel.y - 0.1).abs() < 1e-6 && p.vel.z > 0.17, "{}", p.vel);
        assert!(p.flags & FLAG_SPAWN_FRAME != 0 && (0.5..=0.8).contains(&p.scale));
    }
    sim.landing_impact = 0.08;
    sim.impact_slabs(&pt, Some(&sa));
    sim.landing_impact = 0.55;
    sim.impact_slabs(&pt, None);
    assert_eq!(sim.spawned_slabs, 17);
}

#[test]
fn the_tip_emitter_counts_along_the_segment() {
    let pt = ParticleTuning { slab: slough(), ..ParticleTuning::default() };
    let mut sim = AvalancheSim::default();
    let e = Emit {
        kind: Kind::Slab,
        flags: FLAG_SPAWN_FRAME,
        density: 2.5,
        tip: Vec3::new(0.0, 0.0, 2.0),
        tip_prev: Vec3::ZERO,
        vel: Vec3::Z,
        vel_prev: Vec3::Z,
        normal: Vec3::Y,
        side: Vec3::X,
        width: 0.5,
        height: 0.2,
        scale_min: 0.2,
        scale_max: 0.5,
    };
    let mut acc = 0.4;
    assert_eq!(sim.emit_along_tip(&pt, &e, &mut acc, 2.0), 5);
    assert!((acc - 0.4).abs() < 1e-6);
    let alive: Vec<&Particle> = sim.slabs.iter().filter(|p| p.alive()).collect();
    assert_eq!(alive.len(), 5);
    assert!((alive[0].pos - Vec3::new(0.0, 0.0, 2.0)).length() < 1e-6, "first: tip_prev + v·dt");
    assert!(alive.iter().all(|p| (0.2..=0.5).contains(&p.scale) && p.flags & FLAG_SPAWN_FRAME != 0));
    assert!(alive.iter().all(|p| p.pos.z >= 0.0 && p.pos.z <= 4.0 && p.age <= 2.0));
}

#[test]
fn the_probe_measures_mass_coherence_and_filters() {
    let one = Curve::new(vec![[0.0, 1.0]]);
    let mut sim = AvalancheSim::default();
    let q = Quat::from_rotation_y(0.7);
    for (k, s) in [(0, 0.5), (1, 0.25)] {
        sim.sloughs[k] = Particle { id: k as u16 + 1, scale: s, frame: q, age: 50.0, ..Particle::default() };
    }
    let m = sim.measure(Vec3::ZERO, &one, &one, 0.3, Measure::default(), 2.0);
    assert!((m.mass - 0.75).abs() < 1e-6 && (m.speed - 1.0).abs() < 1e-5, "{m:?}");
    assert!(m.orientation.dot(q).abs() > 0.9999);
    sim.sloughs[1].frame = q * Quat::from_rotation_y(std::f32::consts::PI);
    sim.sloughs[1].scale = 0.5;
    let raw = sim.measure(Vec3::ZERO, &one, &one, 0.3, Measure::default(), 2.0);
    assert!(raw.speed < 1e-5, "opposite flows: {raw:?}");
    let f = sim.measure(Vec3::ZERO, &one, &one, 0.3, m, 2.0);
    let e = (-0.6f32).exp();
    assert!((f.mass - (e * (0.75 - 1.0) + 1.0)).abs() < 1e-5, "{f:?}");
    assert_eq!(AvalancheSim::default().measure(Vec3::ZERO, &one, &one, 0.3, m, 2.0).mass, 0.0);
}

#[test]
fn the_avalanche_tumbles_and_lets_go() {
    let t = crate::testutil::avalanche().carving(2).clone();
    let mut a = RiderAvalanche::default();
    let mut bail = false;
    a.frame.value = 5.0;
    a.balance(&t, &mut bail, false, false);
    assert!(!bail, "a fresh controller's delay is 0, not below");
    a.end_of_update(bail, 2.0);
    let dmg = a.balance(&t, &mut bail, false, false);
    assert!(bail && a.bailed_because_of_avalanche && dmg > 0.0);
    let mut ticks = 0.0;
    while bail {
        a.end_of_update(bail, 2.0);
        ticks += 2.0;
        a.balance(&t, &mut bail, false, false);
    }
    assert!(ticks > 10.0 && ticks < 30.0, "{ticks}");
    assert_eq!(a.prev_tumble_value, 5.0);
    a.end_of_update(bail, 2.0);
    a.frame.value = 6.0;
    a.balance(&t, &mut bail, false, false);
    assert!(!bail, "within the re-entry delay and below 10× the last value");
    a.fresh_controller();
    assert_eq!((a.ride_in_ticks, a.delay_reentry_timer, a.bailed_because_of_avalanche), (0.0, 0.0, false));
}
