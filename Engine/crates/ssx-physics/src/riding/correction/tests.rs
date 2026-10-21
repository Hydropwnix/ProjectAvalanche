use super::*;
use crate::collision::CollisionWorld;
use crate::testutil::{add_floor, add_quad, world};

fn walled() -> World {
    let mut c = CollisionWorld::default();
    let v = |x: f32, y: f32| Vec3::new(x, y, 5.0);
    add_quad(&mut c, [v(-20.0, -1.0), v(-20.0, 10.0), v(20.0, 10.0), v(20.0, -1.0)], 1);
    add_floor(&mut c, [-20.0, 20.0], [-20.0, 20.0], 0.0, 0);
    c.finish();
    world(c)
}

fn inputs(from: Vec3, to: Vec3, v: Vec3) -> CorrectionInputs {
    CorrectionInputs {
        from,
        to,
        velocity: v,
        orientation: Quat::IDENTITY,
        ground_valid: true,
        distance_from_ground: 0.83,
        prev_colliding: false,
        grinding: false,
        prev_grinding: false,
        dt: 2.0,
    }
}

#[test]
fn open_space_changes_nothing() {
    let w = walled();
    let i = inputs(Vec3::new(0.0, 0.83, 0.0), Vec3::new(0.0, 0.83, 0.6), Vec3::Z * 0.3);
    let out = w.collision_correction(&i);
    assert_eq!(out.position, i.to);
    assert!(out.bounce.is_none() && out.soft.is_none() && out.impact == ImpactState::default());
}

#[test]
fn a_wall_stops_the_point_and_bounces_the_velocity() {
    let w = walled();
    let v = Vec3::Z * 0.3;
    let i = inputs(Vec3::new(0.0, 0.83, 4.7), Vec3::new(0.0, 0.83, 5.3), v);
    let out = w.collision_correction(&i);
    assert!(out.position.z < 5.0, "{:?}", out.position);
    let b = out.bounce.expect("moving into the wall bounces");
    assert!(b.z < 0.0 && b.z > -0.3 * 0.5, "{b:?}");
    assert!(out.impact.colliding && out.impact.in_soft_collision && out.impact.impact_this_frame);
    assert!(out.impact.impact_speed > 0.0);
    let again = w.collision_correction(&CorrectionInputs { prev_colliding: true, ..i });
    assert!(again.position.z < 5.0 && !again.impact.impact_this_frame);
}

#[test]
fn a_hit_near_the_up_axis_is_no_collision() {
    let w = walled();
    let i = CorrectionInputs {
        ground_valid: false,
        distance_from_ground: 0.0,
        ..inputs(Vec3::new(0.0, 0.3, 0.0), Vec3::new(0.0, -0.3, 0.0), Vec3::new(0.0, -0.3, 0.0))
    };
    let out = w.collision_correction(&i);
    assert!(out.position.y > -0.05, "{:?}", out.position);
    assert!(!out.impact.colliding);
}

#[test]
fn a_bot_pushes_by_a_tenth_of_its_odd_velocity_difference_at_most_5_cm_per_tick() {
    let w = walled();
    let v = Vec3::new(0.0, 0.0, 0.3);
    let g = Vec3::new(0.2, 0.0, 0.3);
    let b = w.ghost_bounce(v, g);
    assert!(b.distance(v + (g - Vec3::splat(0.09)) * 0.1) < 1e-6, "{b:?}");
    let far = w.ghost_bounce(v, Vec3::new(3.0, 0.0, 0.0));
    assert!(((far - v).length() - 0.05).abs() < 1e-6);
    let st = w.ghost_impact_state(ImpactState::default(), Quat::IDENTITY, v, false);
    assert!(st.colliding && st.impact_this_frame && !st.trigger_impact && st.impact_velocity == Vec3::ZERO);
    let again = w.ghost_impact_state(ImpactState::default(), Quat::IDENTITY, v, true);
    assert!(again.colliding && !again.impact_this_frame);
}
