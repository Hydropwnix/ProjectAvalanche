use bevy::camera::CameraProjection;
use bevy::prelude::*;

use super::hud_projection;
use ssx_ui::geom::RectF;
use ssx_ui::render::{ScaleMode, stage_transform};

#[test]
fn a_tag_sits_on_its_point_at_every_ratio() {
    let stage = RectF { x0: 0.0, y0: 0.0, x1: 1280.0 * 20.0, y1: 720.0 * 20.0 };
    let world_from_view = Mat4::from_rotation_translation(Quat::from_rotation_y(0.4), Vec3::new(3.0, 12.0, -40.0));
    for (w, h) in [(1024, 768), (1280, 800), (1920, 1080), (2560, 1080), (3440, 1440), (5120, 1440)] {
        let projection = crate::camera::GameProjection { aspect: w as f32 / h as f32, ..Default::default() };
        let clip_from_view = projection.get_clip_from_view();
        let cam = hud_projection(clip_from_view, world_from_view, Some(Vec2::new(w as f32, h as f32)));
        let place = stage_transform(stage, (w, h), ScaleMode::NoBorder);
        for local in [Vec3::new(0.0, 0.0, -30.0), Vec3::new(14.0, -3.0, -25.0), Vec3::new(-30.0, 5.0, -20.0)] {
            let p = world_from_view.transform_point3(local);
            let ndc = (clip_from_view * world_from_view.inverse()).project_point3(p);
            let want = ((ndc.x + 1.0) * 0.5 * w as f32, (1.0 - ndc.y) * 0.5 * h as f32);
            let m = &cam.view_proj;
            let c: [f32; 4] = std::array::from_fn(|k| p.x * m[0][k] + p.y * m[1][k] + p.z * m[2][k] + m[3][k]);
            let s = ((c[0] / c[3] + 1.0) * 640.0, (1.0 - c[1] / c[3]) * 360.0);
            let got = place.apply(s.0 * 20.0, s.1 * 20.0);
            let off = ((got.0 - want.0).abs()).max((got.1 - want.1).abs());
            assert!(off < 0.05, "{w} x {h}, {local}: HUD {got:?}, view {want:?}");
        }
    }
}
