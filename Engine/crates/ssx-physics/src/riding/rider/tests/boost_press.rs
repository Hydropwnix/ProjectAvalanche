use super::*;

#[test]
fn boost_accelerates_and_drains_the_meter() {
    let theta = 5f32.to_radians();
    let w = world(plane(2500.0, theta));
    let at = Vec3::new(0.0, 2000.0 * theta.tan() + 1.0, -2000.0);
    let (mut free, mut boosted) = (moving(&w, at, Vec3::Z, 0.1), moving(&w, at, Vec3::Z, 0.1));
    let mut empty = moving(&w, at, Vec3::Z, 0.1);
    (free.score.boost, boosted.score.boost) = (150.0, 150.0);
    let r2 = TrickInput { tweak: true, ..TrickInput::default() };
    for _ in 0..60 {
        step(&mut free, &w, &Input::default(), DT_TICKS);
        step_with(&mut boosted, &w, &Input::default(), &r2, DT_TICKS);
        step_with(&mut empty, &w, &Input::default(), &r2, DT_TICKS);
        assert!(boosted.boost.is_boosting && boosted.on_ground && !empty.boost.is_boosting);
    }
    assert!((boosted.score.boost - (150.0 - 60.0 * 2.0 * 0.3)).abs() < 1e-3, "{}", boosted.score.boost);
    assert_eq!((free.score.boost, empty.score.boost), (150.0, 0.0));
    assert!(boosted.speed() > free.speed() + 0.3, "{} vs {} km/h", kmh(boosted.speed()), kmh(free.speed()));
    assert!(boosted.speed() < w.props.max_boost_speed, "the thrust fades out at MaxBoostSpeed");
    assert_eq!(empty.speed(), free.speed());
    assert!(boosted.boost.blend > 0.99 && free.boost.blend == 0.0);
    assert!(boosted.carving.riding_height > -0.6 && free.carving.riding_height < -0.7, "{boosted:?}");
}

#[test]
fn braking_sprays_snow_and_the_air_does_not() {
    let theta = 15f32.to_radians();
    let w = world(plane(3000.0, theta));
    let at = Vec3::new(0.0, 2000.0 * theta.tan() + 1.0, -2000.0);
    let mut s = moving(&w, at, Vec3::Z, 0.3);
    ride(&w, &mut s, &Input { brake: 1.0, ..Input::default() }, 6, |s| assert!(s.on_ground && !s.stopped));
    let sp = s.spray;
    assert!(sp.carving.length() > 1e-3 && sp.carving.dot(s.velocity) > 0.0, "{sp:?}");
    let mut straight = moving(&w, at, Vec3::Z, 0.1);
    ride(&w, &mut straight, &Input::default(), 6, |_| {});
    assert_eq!(straight.spray.carving, Vec3::ZERO, "accelerating along the board");
    let v_ground = s.velocity - s.ground_normal * s.velocity.dot(s.ground_normal);
    assert!(sp.velocity.distance(-v_ground) < 1e-6 && sp.impact.dot(s.ground_normal) >= 0.0);
    let w = ledge(3.0);
    let mut s = moving(&w, Vec3::new(0.0, 0.5, -20.0), Vec3::Z, 0.25);
    ride(&w, &mut s, &Input::default(), 60, |_| {});
    let mut airborne = 0;
    ride(&w, &mut s, &Input::default(), 30, |s| {
        if s.state != RideState::Riding {
            airborne += 1;
            assert_eq!(s.spray, Spray::default());
        }
    });
    assert!(airborne > 0);
}

#[test]
fn a_held_x_is_a_nose_press_on_the_ground() {
    let w = world(plane(800.0, 0.0));
    let mut free = moving(&w, Vec3::new(0.0, 1.0, -700.0), Vec3::Z, kmh(60.0));
    let mut s = free.clone();
    let x = TrickInput { x: true, ..TrickInput::default() };
    for i in 0..60 {
        step_with(&mut s, &w, &Input::default(), &x, DT_TICKS);
        step(&mut free, &w, &Input::default(), DT_TICKS);
        assert!(s.press.pressing && s.press.inputs.nose, "update {i}");
        if i >= 1 {
            assert!(s.press.is_board_press, "update {i}: blend {}", s.press.blend);
        }
    }
    assert!(s.press.blend > 0.99, "{}", s.press.blend);
    assert!(s.grind.carving.grind_or_press_blend > 0.99);
    assert!(s.grind.carving.press.value.abs() > 0.9, "the rail press value follows: {}", s.grind.carving.press.value);
    assert!(s.speed() < free.speed() - 1e-3, "pressing brakes: {} vs {}", s.speed(), free.speed());
    for _ in 0..30 {
        step_with(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS);
    }
    assert!(!s.press.pressing && !s.press.is_board_press && s.press.blend < 0.01, "{}", s.press.blend);
}

#[test]
fn a_finished_track_force_stops_the_rider() {
    let w = world(plane(800.0, 0.05));
    let mut s = moving(&w, Vec3::new(0.0, 40.0, -700.0), Vec3::Z, kmh(80.0));
    let flow = GameFlowFlags { track_finished: true, ..GameFlowFlags::default() };
    let mut stopped_at = None;
    for i in 0..300 {
        let x = StepContext { flow, ..StepContext::default() };
        step_full(&mut s, &w, &Input::default(), &TrickInput::default(), DT_TICKS, x);
        assert!(s.force_stop);
        if s.speed() == 0.0 && stopped_at.is_none() {
            stopped_at = Some(i);
        }
        if stopped_at.is_some() {
            assert!(s.allow_race_end || s.force_race_end_timer >= 0.0);
        }
    }
    let at = stopped_at.expect("stops");
    assert!(at < 150, "stopped after {} s", at as f32 / 30.0);
    assert!(s.press.force_stop_mag > 0.9 && s.allow_race_end);
}

#[test]
fn the_ground_joint_hangs_along_the_boards_up() {
    let w = world(plane(100.0, 0.0));
    let mut s = moving(&w, Vec3::new(0.0, 1.0, 0.0), Vec3::Z, 0.3);
    let rh = s.carving.riding_height;
    assert!(rh < -0.3 && s.ground_normal.y > 0.999);
    assert!((s.board_point() - (s.traj + Vec3::Y * rh)).length() < 1e-4, "upright: straight below P");
    s.alignment = Quat::from_axis_angle(s.alignment * Vec3::X, 30f32.to_radians()) * s.alignment;
    let j = s.board_point();
    let down = (j - s.traj).normalize();
    assert!((down.dot(-Vec3::Y) - 30f32.to_radians().cos()).abs() < 1e-4, "{down:?}");
    assert!(((j - s.traj).length() - rh.abs()).abs() < 1e-5);
    let side = (j - s.traj) - Vec3::Y * (j - s.traj).y;
    assert!((side.length() - rh.abs() * 0.5).abs() < 1e-4, "{side:?}");
}
