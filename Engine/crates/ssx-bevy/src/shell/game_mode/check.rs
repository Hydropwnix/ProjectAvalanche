use std::path::{Path, PathBuf};

use anyhow::Context;
use bevy::prelude::{Commands, On, Res, ResMut, Resource, Vec2, error, info};
use bevy::render::MainWorld;
use bevy::render::render_resource::PipelineCache;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};

use crate::ui;

pub fn ui_cover(frame: &ui::UiFrame) -> Option<usize> {
    let vp = frame.viewport.as_vec2();
    if vp.x <= 0.0 || vp.y <= 0.0 {
        return None;
    }
    let mut masks = 0i32;
    let mut in_mask = false;
    for (i, op) in frame.ops.iter().enumerate() {
        let (draw, first_index, index_count, base_vertex) = match *op {
            ui::UiOp::MaskBegin | ui::UiOp::MaskPopBegin => {
                in_mask = true;
                continue;
            }
            ui::UiOp::MaskEnd => {
                (in_mask, masks) = (false, masks + 1);
                continue;
            }
            ui::UiOp::MaskPopEnd => {
                (in_mask, masks) = (false, masks - 1);
                continue;
            }
            ui::UiOp::Draw {
                draw,
                first_index,
                index_count,
                base_vertex,
                texture: ui::UiTexture::None,
                blend: ui::UiBlend::Normal,
            } => (draw, first_index, index_count, base_vertex),
            ui::UiOp::Draw { .. } => continue,
        };
        let Some(d) = frame.draws.get(draw as usize) else { continue };
        let alpha = (d[27] * d[11] + d[15]).clamp(0.0, 1.0);
        if in_mask || masks > 0 || d[22] != 0.0 || alpha < 0.99 {
            continue;
        }
        let point = |k: u32| -> Option<Vec2> {
            let index = *frame.indices.get(k as usize)? as i64 + i64::from(base_vertex);
            let [x, y] = *frame.vertices.get(usize::try_from(index).ok()?)?;
            Some(Vec2::new(d[0] * x + d[2] * y + d[4], d[1] * x + d[3] * y + d[5]))
        };
        let (mut lo, mut hi, mut area) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN), 0.0);
        for t in (first_index..first_index + index_count).step_by(3) {
            let (Some(a), Some(b), Some(c)) = (point(t), point(t + 1), point(t + 2)) else { continue };
            area += (b - a).perp_dot(c - a).abs() * 0.5;
            lo = lo.min(a).min(b).min(c);
            hi = hi.max(a).max(b).max(c);
        }
        let covers = lo.x <= vp.x * 0.02 && lo.y <= vp.y * 0.02 && hi.x >= vp.x * 0.98 && hi.y >= vp.y * 0.98;
        if covers && area >= 0.9 * vp.x * vp.y {
            return Some(i);
        }
    }
    None
}

#[derive(Resource, Default)]
pub struct UiPipelinesReady(pub bool);

pub fn report_pipelines(mut main_world: ResMut<MainWorld>, pipelines: Res<PipelineCache>) {
    if let Some(mut ready) = main_world.get_resource_mut::<UiPipelinesReady>() {
        ready.0 = pipelines.waiting_pipelines().next().is_none();
    }
}

pub fn screenshot(commands: &mut Commands, path: PathBuf, check_world: bool) {
    info!("screenshot {}", path.display());
    commands.spawn(Screenshot::primary_window()).observe(
        move |c: On<ScreenshotCaptured>, mut check: ResMut<WorldCheck>| {
            let status = c.image.clone().try_into_dynamic().context("converting the captured frame").and_then(|img| {
                let rgb = img.to_rgb8();
                if check_world {
                    check.check(rgb.width(), rgb.height(), rgb.as_raw(), &path);
                }
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
                }
                rgb.save(&path).with_context(|| format!("writing {}", path.display()))
            });
            if let Err(e) = status {
                error!("saving the screenshot failed: {e:#}");
            }
        },
    );
}

#[derive(Resource, Default)]
pub struct WorldCheck {
    pub shots: u32,
    pub covered: Vec<String>,
}

impl WorldCheck {
    pub const COVERED: f32 = 0.9;

    pub fn measure(width: u32, height: u32, rgb: &[u8]) -> (f32, [u8; 3]) {
        let (w, h) = (width as usize, height as usize);
        let px = |x: usize, y: usize| -> [u8; 3] {
            let i = (y * w + x) * 3;
            rgb.get(i..i + 3).map_or([0; 3], |p| [p[0], p[1], p[2]])
        };
        let (x0, y0) = (w * 3 / 8, h * 3 / 8);
        let (x1, y1) = ((w * 5 / 8).max(x0 + 1).min(w), (h * 5 / 8).max(y0 + 1).min(h));
        let (mut black, mut n) = (0usize, 0usize);
        for y in y0..y1 {
            for x in x0..x1 {
                n += 1;
                black += usize::from(px(x, y).iter().all(|&c| c <= 2));
            }
        }
        (black as f32 / n.max(1) as f32, px(w / 2, h / 2))
    }

    pub fn covered_by_ui(&mut self, path: &Path, op: usize) {
        error!(
            "world check: {}: the UI covers the level's world (an opaque full-screen fill, UI op {op})",
            path.display()
        );
        self.covered.push(format!("{} (the UI)", path.display()));
    }

    fn check(&mut self, width: u32, height: u32, rgb: &[u8], path: &Path) {
        let (share, centre) = Self::measure(width, height, rgb);
        self.shots += 1;
        if share >= Self::COVERED {
            error!(
                "world check: {}: {:.1} % of the centre region is black (centre pixel {centre:?}): the level's world is \
                 covered or not drawn",
                path.display(),
                share * 100.0
            );
            self.covered.push(path.display().to_string());
        } else {
            info!(
                "world check: {}: {:.1} % of the centre region black, centre pixel {centre:?}",
                path.display(),
                share * 100.0
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bevy::prelude::{UVec2, default};

    use super::*;

    fn frame(rect: [f32; 4], a: f32, masked: bool) -> ui::UiFrame {
        let mut d = [0.0f32; ui::DRAW_FLOATS];
        d[0] = 1.0;
        d[3] = 1.0;
        d[8..12].copy_from_slice(&[1.0; 4]);
        d[27] = a;
        let [x0, y0, x1, y1] = rect;
        let draw = ui::UiOp::Draw {
            draw: 0,
            first_index: 0,
            index_count: 6,
            base_vertex: 0,
            texture: ui::UiTexture::None,
            blend: ui::UiBlend::Normal,
        };
        let ops = if masked {
            vec![ui::UiOp::MaskBegin, ui::UiOp::MaskEnd, draw, ui::UiOp::MaskPopBegin, ui::UiOp::MaskPopEnd]
        } else {
            vec![draw]
        };
        ui::UiFrame {
            viewport: UVec2::new(1280, 720),
            vertices: Arc::new(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]),
            indices: Arc::new(vec![0, 1, 2, 0, 2, 3]),
            draws: Arc::new(vec![d]),
            ops: Arc::new(ops),
            ..default()
        }
    }

    #[test]
    fn ui_cover_finds_an_opaque_full_screen_fill_only() {
        let full = [0.0, 0.0, 1280.0, 720.0];
        assert_eq!(ui_cover(&frame(full, 1.0, false)), Some(0));
        assert_eq!(ui_cover(&frame(full, 0.5, false)), None);
        assert_eq!(ui_cover(&frame(full, 1.0, true)), None);
        assert_eq!(ui_cover(&frame([0.0, 0.0, 640.0, 720.0], 1.0, false)), None);
    }

    #[test]
    fn world_check_measures_the_centre_region() {
        let (w, h) = (64u32, 36u32);
        let black = vec![0u8; (w * h * 3) as usize];
        assert_eq!(WorldCheck::measure(w, h, &black), (1.0, [0, 0, 0]));
        let night = [4u8, 9, 14].repeat((w * h) as usize);
        assert_eq!(WorldCheck::measure(w, h, &night), (0.0, [4, 9, 14]));
    }
}
