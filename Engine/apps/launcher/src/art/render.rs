use super::image::Picture;
use super::noise::{fbm, hash, smoothstep, zigzag};
use super::procedural::{Rgb, Scene};

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

struct Layer {
    crest: Vec<f32>,
    apex: Vec<(f32, f32)>,
    seed: u32,
}

fn layer(scene: &Scene, k: usize, width: usize, height: usize) -> Layer {
    let (r, h) = (scene.ranges[k], height as f32);
    let seed = scene.seed.wrapping_add(k as u32 * 7919 + 1);
    let rnd = |i: i32, n: i32| hash(i, n, seed);
    let mut points = vec![(-(0.2 + 0.6 * rnd(-1, 0)) * r.spacing * h, r.base * h)];
    let mut i = 0;
    while points[points.len() - 1].0 < width as f32 {
        let (x, span) = (points[points.len() - 1].0, r.spacing * h * (0.7 + 0.6 * rnd(i, 1)));
        points.push((x + span * (0.36 + 0.28 * rnd(i, 2)), (r.base - r.rise * (0.5 + 0.5 * rnd(i, 3))) * h));
        points.push((x + span, (r.base - r.rise * 0.22 * rnd(i, 4)) * h));
        i += 1;
    }
    let (mut crest, mut apex, mut at) = (Vec::with_capacity(width), Vec::with_capacity(width), 0);
    for px in 0..width {
        let x = px as f32;
        while at + 2 < points.len() && points[at + 1].0 < x {
            at += 1;
        }
        let ((x0, y0), (x1, y1)) = (points[at], points[at + 1]);
        let line = y0 + (y1 - y0) * ((x - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let top = if at % 2 == 1 { points[at] } else { points[at + 1] };
        let away = ((x - top.0).abs() / (0.05 * h)).min(1.0);
        let crag = zigzag(x / (0.045 * h), seed ^ 0x11) * 0.030 + zigzag(x / (0.012 * h), seed ^ 0x22) * 0.008;
        crest.push(line + crag * h * away * (r.rise / 0.25));
        apex.push(top);
    }
    Layer { crest, apex, seed }
}

fn sky(scene: &Scene, px: usize, py: usize, height: usize, aspect: f32) -> Rgb {
    let (x, y) = (px as f32 / height as f32, py as f32 / height as f32);
    let t = (y / scene.horizon).clamp(0.0, 1.0);
    let base =
        if t < 0.5 { mix(scene.sky[0], scene.sky[1], t * 2.0) } else { mix(scene.sky[1], scene.sky[2], t * 2.0 - 1.0) };
    let band = smoothstep(0.56, 0.80, fbm(x * 0.9, y * 13.0, 3, scene.seed ^ 0x51));
    let clouded = mix(base, scene.sky[2], band * 0.26 * smoothstep(0.15, 0.7, t));
    let (sx, sy, radius) = scene.sun;
    let d = ((x - sx * aspect).powi(2) + (y - sy).powi(2)).sqrt() / radius;
    let lit = mix(clouded, scene.glow, (-d * d * 3.2).exp() * 0.70 + (-d * d * 70.0).exp());
    let star = if hash(px as i32, py as i32, scene.seed ^ 0x57A2) > 0.9988 { (1.0 - t).powi(2) * 0.8 } else { 0.0 };
    mix(lit, scene.glow, star)
}

fn face(scene: &Scene, k: usize, l: &Layer, px: usize, py: usize, width: usize, height: usize) -> Rgb {
    let (r, h, x, y) = (scene.ranges[k], height as f32, px as f32, py as f32);
    let side = if scene.sun.0 > 0.5 { 1.0 } else { -1.0 };
    let below_crest = (y - l.crest[px]).max(0.0);
    let lean = side * below_crest * 0.16 + zigzag(below_crest / (0.06 * h), l.seed ^ 0x77) * below_crest * 0.5;
    let (ax, ay) = l.apex[((x + lean).round().max(0.0) as usize).min(width - 1)];
    let below_apex = (y - ay).max(0.0);
    let peak = l.seed ^ (ax as i32 as u32).wrapping_mul(2_654_435_761);
    let edge = ax - side * below_apex * 0.16 + zigzag(below_apex / (0.055 * h), peak) * below_apex * 0.55;
    let lit = smoothstep(-0.8, 0.8, (x - edge) * side);
    let t = (y - l.crest[px]) / (r.rise * h);
    let mut tone = mix(scene.shade, scene.snow, lit);
    let line = 0.46 + zigzag(x / (0.040 * h), l.seed ^ 0xA5) * 0.42;
    let rocky = smoothstep(line, line + 0.03, t) * (0.50 + 0.40 * (1.0 - lit)) * (0.30 + 0.70 * r.depth);
    tone = mix(tone, mix(scene.rock, scene.shade, 0.30 * lit), rocky);
    let far = (1.0 - r.depth) * 0.74;
    let mist = smoothstep(0.25, 1.7, t) * 0.60 * (1.0 - 0.55 * r.depth);
    mix(mix(tone, scene.haze, far), scene.haze, mist)
}

fn pixel(scene: &Scene, layers: &[Layer], px: usize, py: usize, width: usize, height: usize) -> Rgb {
    let aspect = width as f32 / height as f32;
    let mut colour: Option<(Rgb, f32)> = None;
    for (k, l) in layers.iter().enumerate().rev() {
        let below = py as f32 - l.crest[px] + 0.5;
        if below <= 0.0 {
            continue;
        }
        let here = face(scene, k, l, px, py, width, height);
        colour = Some(match colour {
            Some((front, cover)) => (mix(here, front, cover), 1.0),
            None => (here, below.min(1.0)),
        });
        if colour.is_some_and(|(_, cover)| cover >= 1.0) {
            break;
        }
    }
    match colour {
        Some((c, cover)) if cover >= 1.0 => c,
        Some((c, cover)) => mix(sky(scene, px, py, height, aspect), c, cover),
        None => sky(scene, px, py, height, aspect),
    }
}

pub fn render(scene: &Scene, width: usize, height: usize) -> Picture {
    let layers: Vec<Layer> = (0..scene.ranges.len()).map(|k| layer(scene, k, width, height)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(height.max(1));
    let rows = height.div_ceil(threads);
    let mut rgba = vec![255u8; width * height * 4];
    std::thread::scope(|scope| {
        for (band, out) in rgba.chunks_mut(rows * width * 4).enumerate() {
            let layers = &layers;
            scope.spawn(move || {
                for (i, p) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let (px, py) = (i % width, band * rows + i / width);
                    let c = pixel(scene, layers, px, py, width, height);
                    p[..3].copy_from_slice(&c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
                }
            });
        }
    });
    Picture { width, height, rgba }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::procedural::{BACKDROP, TILES};

    #[test]
    fn scenes_render_the_same_every_time() {
        let a = render(&BACKDROP, 96, 54);
        assert_eq!(a, render(&BACKDROP, 96, 54));
        assert_eq!(a.rgba.len(), 96 * 54 * 4);
        assert!(a.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        assert_ne!(render(&TILES[0], 48, 32), render(&TILES[2], 48, 32));
    }
}
