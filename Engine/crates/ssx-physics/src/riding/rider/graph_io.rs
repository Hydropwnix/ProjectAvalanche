use super::*;

pub(super) fn trick_context(s: &RiderState, in_air: bool) -> TrickContext {
    TrickContext {
        in_air,
        enable_air_tricks: !(s.state == RideState::Airborne && s.jump.allow),
        goofy: s.goofy,
        awesome: s.score.is_awesome,
        super_awesome: s.score.is_super_awesome,
        ticks_to_land: s.ballistic.ticks,
        recovering: s.recovering_from_uber_bail(),
        board_particle: s.board_particle.enabled,
        air_jump: false,
        is_grinding: s.grind.is_grinding(),
        abort: s.heli.abort_aerial_trick,
    }
}

pub(super) fn air_tricks_ticks(s: &RiderState, dt: f32) -> Option<f32> {
    if s.heli_trick.is_some() {
        return None;
    }
    match s.state {
        RideState::Riding => None,
        RideState::Airborne => Some(dt),
        RideState::Jump if s.jump_airborne => Some(dt),
        RideState::Jump => {
            let due = (JUMP_END_WINDOW_TICKS - (s.jump_ticks - dt)).max(0.0);
            (due < dt).then_some(dt - due)
        }
        RideState::WingsuitEnter | RideState::WingsuitFly | RideState::WingsuitExit => None,
    }
}

pub(super) fn ground_trajectory(s: &RiderState) -> Quat {
    if s.landing.uber_crash {
        return s.alignment;
    }
    let st = &s.steering;
    let f0 = st.slip_angle * s.kin.fwd;
    let c = s.kin.slip;
    let slip_vis = if f0 >= 0.0 {
        if c >= 0.0 { (f0 - c).max(0.0) } else { f0 }
    } else if c >= 0.0 {
        f0
    } else {
        (f0 - c).min(0.0)
    };
    let l = &s.landing;
    let stance = Quat::from_rotation_y((1.0 - st.stance_override_sign) * FRAC_PI_2);
    (Quat::from_axis_angle(s.ground_normal, slip_vis)
        * s.alignment
        * Quat::from_rotation_z(l.pitch)
        * Quat::from_rotation_y(l.spin)
        * stance)
        .normalize()
}

pub(super) fn rig_ops(s: &mut RiderState, _w: &World, dp: Vec3, root: Option<Quat>, dt: f32) {
    let t_old = s.traj_orientation;
    s.traj += dp;
    s.traj_orientation = rig_op_rotation(s, root);
    if dt <= TINY {
        return;
    }
    let inv = 1.0 / dt;
    s.traj_velocity = dp * inv;
    let w_step = (s.traj_orientation * t_old.conjugate()).normalize();
    s.angular_velocity_q = nlerp(Quat::IDENTITY, w_step, inv);
    s.angular_velocity = s.angular_velocity_q.to_scaled_axis();
    s.filtered_velocity = s.traj_velocity + (s.filtered_velocity - s.traj_velocity) * (-0.1 * dt).exp();
    s.filtered_angular_velocity = nlerp(s.angular_velocity_q, s.filtered_angular_velocity, (-0.075 * dt).exp());
}

pub(super) fn rig_op_rotation(s: &RiderState, root: Option<Quat>) -> Quat {
    match root {
        Some(q) => q,
        None if s.state == RideState::Riding => ground_trajectory(s),
        None => s.alignment,
    }
}

pub(super) const CARVING: &[&str] = &["CarvingControllerAsset"];
pub(super) const AERIAL: &[&str] = &["AerialControllerAsset"];

pub(super) const AERIAL_NODES: [&str; 2] = ["state_airborne", "state_airborne_jump"];

pub(super) fn resolve_root_motion(
    s: &mut RiderState,
    motion: Delta,
    physics: (Vec3, Option<Quat>),
    dt: f32,
) -> (Vec3, Option<Quat>) {
    let (dp, root) = match motion {
        Delta::Physics => return physics,
        Delta::Value { q, t } => world_motion(s.traj_orientation, q, t),
        Delta::None => (Vec3::ZERO, s.traj_orientation),
        // TODO(guess): the physics' own step where the pose's delta set is an undecoded composition (only the 6-tick `BlendCurveWithJointPivot` of an uber-crash landing; its pose job 0x82DD33F8 → 0x82DC82D0, given the pivot joint chain and the delta set's index, read, its kernel not; the other blends take the valid side, or nlerp / lerp, `Pose_BlendDeltaTrajectory` 0x83184298);
        // settle: that kernel's delta handling.
        Delta::Untraced => {
            crate::guess_hit!();
            return physics;
        }
    };
    if s.janked.janked || s.get_going.is_some() {
        s.velocity = dp / dt;
    }
    (dp, Some(root))
}
