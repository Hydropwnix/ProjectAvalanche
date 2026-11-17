use super::*;
use crate::character::CarvingTuning;
use ssx_formats::vmx;

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

pub(super) fn trajectory_delta_rotation(s: &RiderState, c: &CarvingTuning, j: Quat, n: Vec3, uber: bool) -> Quat {
    if uber {
        return uber_slide_delta(j, n, c.uber_crash_traj_angle_max_delta, s.carving.ride_frames);
    }
    let st = &s.steering;
    let f0 = st.slip_angle * s.kin.fwd;
    let k = s.kin.slip;
    let slip_vis = if f0 >= 0.0 {
        if k >= 0.0 { (f0 - k).max(0.0) } else { f0 }
    } else if k >= 0.0 {
        f0
    } else {
        (f0 - k).min(0.0)
    };
    let l = &s.landing;
    let stance = -(c.stance_angle_sideways_value * st.stance_override_sign);
    ground_delta(j, s.alignment, [l.pitch, l.spin, stance], crate::steering::vmx_wrap_pi(slip_vis), n)
}

pub(super) fn ground_delta(j: Quat, a: Quat, angles: [f32; 3], slip: f32, n: Vec3) -> Quat {
    use crate::steering::{vmx_axis_angle as axis_angle, vmx_conjugate as conj, vmx_quat_mul as mul, vmx_rotate};
    let m1 = mul(conj(j), a);
    let qz = axis_angle([0.0, 0.0, 1.0], angles[0]);
    let qy = axis_angle([0.0, 1.0, 0.0], angles[1]);
    let qs = axis_angle([0.0, 1.0, 0.0], angles[2]);
    let p = mul(mul(mul(mul(j, m1), qz), qy), qs);
    let n_p = vmx_rotate(conj(p), n);
    mul(mul(mul(mul(m1, qz), qy), qs), axis_angle(n_p.to_array(), slip))
}

pub(super) fn uber_slide_delta(j: Quat, n: Vec3, max: f32, ride_frames: i32) -> Quat {
    use crate::steering::{vmx_conjugate, vmx_cross, vmx_rotate};
    const X: [f32; 3] = [1.0, 0.0, 0.0];
    let nl = vmx_rotate(vmx_conjugate(j), n).to_array();
    let x = vmx::dot3(X, nl);
    let axis = vmx_cross(X, nl);
    if ride_frames <= 1 || !vmx::gt(vmx::abs(vmx::sub(vmx::abs(x), 1.0)), TINY) {
        return Quat::IDENTITY;
    }
    let y = vmx::rsqrt(vmx::dot3(axis, axis));
    let angle = vmx::min(max, vmx::max(-max, vmx::acos(x)));
    let (sin, cos) = vmx::sin_cos(vmx::mul(angle, 0.5));
    let [ax, ay, az] = axis.map(|v| vmx::mul(vmx::mul(v, y), sin));
    Quat::from_xyzw(ax, ay, az, cos)
}

pub(super) fn carving_trajectory(s: &RiderState) -> Quat {
    world_motion(s.traj_orientation, s.carving.traj_delta.to_array(), [0.0; 3]).1
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
        None if s.state == RideState::Riding => carving_trajectory(s),
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
