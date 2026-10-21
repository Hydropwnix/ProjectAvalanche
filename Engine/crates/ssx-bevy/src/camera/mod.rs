pub mod chase;
pub mod graph;

use std::f32::consts::FRAC_PI_2;

use bevy::camera::{CameraProjection, Hdr, SubCameraView};
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::light::cluster::ClusterConfig;
use bevy::math::Vec3A;
use bevy::prelude::*;
use bevy::render::render_resource::TextureUsages;
use bevy::render::view::NoIndirectDrawing;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::render::post::{GamePostCamera, PostSettings};
use crate::render::terrain::{Bounds, LoadedTrack};

const LOOK_SENSITIVITY: f32 = 0.003;
const MAX_PITCH: f32 = FRAC_PI_2 - 0.01;
const FAST_MULTIPLIER: f32 = 5.0;
const WHEEL_STEP: f32 = 1.2;
const PIXELS_PER_NOTCH: f32 = 50.0;
const SPEED_RANGE: (f32, f32) = (1.0, 20_000.0);

pub const GAME_NEAR: f32 = 0.5;
pub const GAME_FAR: f32 = 8000.0;
pub const GAME_REST_HFOV_DEG: f32 = 89.52;
pub const GAME_ASPECT: f32 = 16.0 / 9.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GameProjection {
    pub hfov_deg: f32,
    pub aspect: f32,
    pub near: f32,
    pub far: f32,
}

impl Default for GameProjection {
    fn default() -> Self {
        Self { hfov_deg: GAME_REST_HFOV_DEG, aspect: GAME_ASPECT, near: GAME_NEAR, far: GAME_FAR }
    }
}

impl GameProjection {
    pub fn tan_half(&self) -> (f32, f32) {
        let h = (self.hfov_deg.to_radians() * 0.5).tan();
        (h, h / self.aspect)
    }

    fn off_centre(&self, left: f32, right: f32, bottom: f32, top: f32) -> Mat4 {
        let (n, f) = (self.near, self.far);
        let r = n / (f - n);
        Mat4::from_cols(
            Vec4::new(2.0 * n / (right - left), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 * n / (top - bottom), 0.0, 0.0),
            Vec4::new((right + left) / (right - left), (top + bottom) / (top - bottom), r, -1.0),
            Vec4::new(0.0, 0.0, r * f, 0.0),
        )
    }
}

impl CameraProjection for GameProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        let (th, tv) = self.tan_half();
        let (x, y) = (th * self.near, tv * self.near);
        self.off_centre(-x, x, -y, y)
    }

    fn get_clip_from_view_for_sub(&self, sub: &SubCameraView) -> Mat4 {
        let (th, tv) = self.tan_half();
        let (x, y) = (th * self.near, tv * self.near);
        let full = sub.full_size.as_vec2();
        let size = sub.size.as_vec2();
        let oy = full.y - (sub.offset.y + size.y);
        let l = -x + 2.0 * x * sub.offset.x / full.x;
        let r = -x + 2.0 * x * (sub.offset.x + size.x) / full.x;
        let b = -y + 2.0 * y * oy / full.y;
        let t = -y + 2.0 * y * (oy + size.y) / full.y;
        self.off_centre(l, r, b, t)
    }

    fn update(&mut self, width: f32, height: f32) {
        if width > 0.0 && height > 0.0 {
            self.aspect = width / height;
        }
    }

    fn far(&self) -> f32 {
        self.far
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        let (th, tv) = self.tan_half();
        let corner = |z: f32, sx: f32, sy: f32| Vec3A::new(sx * z.abs() * th, sy * z.abs() * tv, z);
        [
            corner(z_near, 1.0, -1.0),
            corner(z_near, 1.0, 1.0),
            corner(z_near, -1.0, 1.0),
            corner(z_near, -1.0, -1.0),
            corner(z_far, 1.0, -1.0),
            corner(z_far, 1.0, 1.0),
            corner(z_far, -1.0, 1.0),
            corner(z_far, -1.0, -1.0),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub position: Vec3,
    pub target: Vec3,
}

pub fn initial_pose(track: &LoadedTrack, position: Option<Vec3>, target: Option<Vec3>, aspect: f32) -> Pose {
    let target = target.unwrap_or_else(|| track.bounds.center());
    let position = position.unwrap_or_else(|| {
        let back = DEFAULT_VIEW_BACK.normalize();
        target + back * framing_distance(track.vertices(), target, back, aspect)
    });
    Pose { position, target }
}

pub const DEFAULT_VIEW_BACK: Vec3 = Vec3::new(-0.6, 0.55, 0.6);

fn framing_distance(points: impl Iterator<Item = Vec3>, target: Vec3, back: Vec3, aspect: f32) -> f32 {
    let forward = -back;
    let right = forward.cross(Vec3::Y).normalize();
    let up = right.cross(forward);
    let (tan_h, tan_v) = GameProjection { aspect, ..default() }.tan_half();
    let distance = points
        .map(|p| {
            let offset = p - target;
            let depth = offset.dot(forward);
            (offset.dot(right).abs() / tan_h).max(offset.dot(up).abs() / tan_v) - depth
        })
        .fold(0.0, f32::max);
    distance * 1.15
}

#[derive(Component, Debug)]
pub struct FlyCam {
    yaw: f32,
    pitch: f32,
    pub speed: f32,
}

impl FlyCam {
    pub fn sync_to(&mut self, rotation: Quat) {
        let (yaw, pitch, _) = rotation.to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH);
    }
}

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraMode {
    Fly,
    Chase,
}

pub fn fly_active(mode: Option<Res<CameraMode>>) -> bool {
    mode.is_none_or(|m| *m == CameraMode::Fly)
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct CameraStart {
    pub pose: Pose,
    pub bounds: Bounds,
}

pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            crate::shell::level::LevelStartup,
            (|commands: Commands, start: Res<CameraStart>, post: Option<Res<PostSettings>>| {
                spawn_camera(commands, start.pose, start.bounds, post.is_some())
            })
            .run_if(resource_exists::<CameraStart>),
        )
        .add_systems(Update, (look, fly, adjust_speed).run_if(fly_active).run_if(resource_exists::<CameraStart>));
    }
}

fn spawn_camera(mut commands: Commands, pose: Pose, bounds: Bounds, game_post: bool) {
    let transform = Transform::from_translation(pose.position).looking_at(pose.target, Vec3::Y);
    let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
    let extent = bounds.size().length();
    let farthest = (0..8)
        .map(|i| {
            let pick = |bit: usize, lo: f32, hi: f32| if i & bit == 0 { lo } else { hi };
            let (lo, hi) = (bounds.min, bounds.max);
            Vec3::new(pick(1, lo.x, hi.x), pick(2, lo.y, hi.y), pick(4, lo.z, hi.z)).distance(pose.position)
        })
        .fold(0.0, f32::max);
    let projection = GameProjection { far: GAME_FAR.max(farthest * 1.05), ..default() };
    let mut camera = commands.spawn((
        Camera3d {
            depth_texture_usages: (TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING).into(),
            ..default()
        },
        Projection::custom(projection),
        transform,
        FlyCam { yaw, pitch, speed: (extent / 20.0).clamp(SPEED_RANGE.0, SPEED_RANGE.1) },
        NoIndirectDrawing,
        ClusterConfig::None,
    ));
    if game_post {
        camera.insert((Hdr, Tonemapping::None, DebandDither::Disabled, GamePostCamera));
    }
}

fn look(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut cameras: Query<(&mut Transform, &mut FlyCam)>,
) {
    if buttons.just_pressed(MouseButton::Right) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if buttons.just_released(MouseButton::Right) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    if !buttons.pressed(MouseButton::Right) || motion.delta == Vec2::ZERO {
        return;
    }
    for (mut transform, mut cam) in &mut cameras {
        cam.yaw -= motion.delta.x * LOOK_SENSITIVITY;
        cam.pitch = (cam.pitch - motion.delta.y * LOOK_SENSITIVITY).clamp(-MAX_PITCH, MAX_PITCH);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    }
}

fn fly(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut cameras: Query<(&mut Transform, &FlyCam)>) {
    let axis = |pos, neg| f32::from(u8::from(keys.pressed(pos))) - f32::from(u8::from(keys.pressed(neg)));
    let fast = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    for (mut transform, cam) in &mut cameras {
        let direction = transform.forward() * axis(KeyCode::KeyW, KeyCode::KeyS)
            + transform.right() * axis(KeyCode::KeyD, KeyCode::KeyA)
            + Vec3::Y * axis(KeyCode::KeyE, KeyCode::KeyQ);
        if direction == Vec3::ZERO {
            continue;
        }
        let speed = if fast { cam.speed * FAST_MULTIPLIER } else { cam.speed };
        transform.translation += direction.normalize() * speed * time.delta_secs();
    }
}

fn adjust_speed(scroll: Res<AccumulatedMouseScroll>, mut cameras: Query<&mut FlyCam>) {
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
    };
    if notches == 0.0 {
        return;
    }
    for mut cam in &mut cameras {
        cam.speed = (cam.speed * WHEEL_STEP.powf(notches)).clamp(SPEED_RANGE.0, SPEED_RANGE.1);
    }
}
