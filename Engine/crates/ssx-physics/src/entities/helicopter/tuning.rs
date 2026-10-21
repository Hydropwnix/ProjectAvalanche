use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct HeliUpdateTuning {
    pub max_speed_ratio: f32,
    pub target_drop_scale: f32,
    pub target_distance: f32,
    pub prepare_target_distance: f32,
    pub desired_elevation_agl: f32,
    pub avoid_boarder_strength: f32,
    pub velocity_avoid_distance: f32,
    pub velocity_avoid_target_distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeliMoveTuning {
    pub name: String,
    pub position_error_to_speed: Curve,
    pub vertical_error_to_speed: Option<Curve>,
    pub time_speed_fixed_target: Curve,
    pub avoid_boarder_thrust: Curve,
    pub pid: [f32; 3],
    pub facing_stiffness: f32,
    pub facing_stiffness_fixed: f32,
    pub velocity_facing_speed: f32,
    pub velocity_facing_distance: f32,
    pub max_velocity: f32,
    pub frames_to_blend_fixed: f32,
    pub avoid_boarder_thrust_scalar: f32,
    pub nominal_thrust: f32,
    pub drag: f32,
    pub target_orientation: bool,
    pub fixed_orientation: bool,
    pub fixed_target: bool,
    pub avoid_boarder: bool,
}

pub(super) fn view_f32(v: &AssetView, f: &str) -> Result<f32> {
    v.f32(f).with_context(|| format!("{}.{f}", v.name()))
}

impl HeliUpdateTuning {
    pub fn load(b: &Bundle) -> Result<Self> {
        let v = b.view("HeliUpdateSceneOp", "HeliUpdateSceneOpAsset")?;
        Ok(Self {
            max_speed_ratio: view_f32(&v, "MaxSpeedRatio")?,
            target_drop_scale: view_f32(&v, "TargetDropScale")?,
            target_distance: view_f32(&v, "TargetDistance")?,
            prepare_target_distance: view_f32(&v, "PrepareForInteractTargetDistance")?,
            desired_elevation_agl: view_f32(&v, "DesiredElevationAGL")?,
            avoid_boarder_strength: view_f32(&v, "AvoidBoarderStrength")?,
            velocity_avoid_distance: view_f32(&v, "VelocityCollisionAvoidanceDistanceMultiplier")?,
            velocity_avoid_target_distance: view_f32(&v, "VelocityCollisionAvoidanceTargetDistanceMultiplier")?,
        })
    }
}

impl HeliMoveTuning {
    pub fn load(b: &Bundle, name: &str) -> Result<Self> {
        let v = b.view(name, "HeliMovementControllerAsset")?;
        let pid = v.asset("MotionControlPidAsset")?;
        let curve = |f: &str| -> Result<Option<Curve>> { v.asset_opt(f)?.map(|c| c.as_curve()).transpose() };
        Ok(Self {
            name: name.to_string(),
            position_error_to_speed: v.curve("PositionErrorToSpeedCurveAsset")?,
            vertical_error_to_speed: curve("VerticalPositionErrorToSpeedCurveAsset")?,
            time_speed_fixed_target: v.curve("TimeSpeedModifierForFixedTargetAsset")?,
            avoid_boarder_thrust: v.curve("AvoidBoarderDistanceToThrustCurveAsset")?,
            pid: [
                view_f32(&pid, "ProportionalGain")?,
                view_f32(&pid, "IntegralGain")?,
                view_f32(&pid, "DerivativeGain")?,
            ],
            facing_stiffness: view_f32(&v, "FacingActuatorStiffness")?,
            facing_stiffness_fixed: view_f32(&v, "FacingActuatorStiffnessForFixedTarget")?,
            velocity_facing_speed: view_f32(&v, "VelocityFacingSpeedThreshold")?,
            velocity_facing_distance: view_f32(&v, "VelocityFacingDistanceThreshold")?,
            max_velocity: view_f32(&v, "MaxVelocity")?,
            frames_to_blend_fixed: view_f32(&v, "FramesToBlendWhenSwitchingToFixedTarget")?,
            avoid_boarder_thrust_scalar: view_f32(&v, "AvoidBoarderThrustScalar")?,
            nominal_thrust: view_f32(&v, "NominalThrust")?,
            drag: view_f32(&v, "DragCoefficient")?,
            target_orientation: v.asset_opt("InputTargetOrientationAsset")?.is_some(),
            fixed_orientation: v.asset_opt("InputFixedTargetOrientationAsset")?.is_some(),
            fixed_target: v.asset_opt("InputFixedTargetAsset")?.is_some(),
            avoid_boarder: v.asset_opt("AvoidBoarderAsset")?.is_some_and(|a| a.name() != "BLN_False"),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeliPosingTuning {
    pub velocity_tilt: Curve,
    pub acceleration_tilt: Curve,
    pub strength: f32,
    pub brake_tolerance: f32,
    pub axis_convergence: f32,
    pub max_tilt: f32,
}

impl HeliPosingTuning {
    pub fn load(b: &Bundle) -> Result<Self> {
        let v = b.view("heli_posing", "HeliPosingControllerAsset")?;
        Ok(Self {
            velocity_tilt: v.curve("VelocityTiltCurveAsset")?,
            acceleration_tilt: v.curve("AccelerationTiltCurveAsset")?,
            strength: view_f32(&v, "TiltActuatorStrength")?,
            brake_tolerance: view_f32(&v, "TiltAxisBrakeTolerance")?,
            axis_convergence: view_f32(&v, "TiltAxisConvergence")?,
            max_tilt: view_f32(&v, "MaxTiltAngle")?,
        })
    }
}
