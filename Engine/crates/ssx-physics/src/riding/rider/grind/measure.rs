use glam::Vec3;

use crate::collision::CollisionWorld;

const HALF_SPAN: f32 = 0.75;
const SPACING: f32 = 0.02;
const ABOVE: f32 = 1.0;
const REACH: f32 = 2.5;
const CROWN_BAND: f32 = 0.005;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RailProbe {
    pub gap: Option<f32>,
    pub crown_lateral: f32,
    pub crown_gap: f32,
    pub width: f32,
    pub class: u8,
}

pub fn rail_under(c: &CollisionWorld, p: Vec3, n: Vec3, forward: Vec3, class_mask: u32) -> Option<RailProbe> {
    let side = n.cross(forward).normalize_or_zero();
    if side == Vec3::ZERO {
        return None;
    }
    let count = (HALF_SPAN / SPACING).round() as i32;
    let mut hits: Vec<(f32, f32, u8)> = Vec::new();
    let mut gap = None;
    for k in -count..=count {
        let x = k as f32 * SPACING;
        let origin = p + side * x + n * ABOVE;
        if let Some(h) = c.ray_cast_masked(origin, -n, REACH, class_mask) {
            let height = ABOVE - h.t;
            hits.push((x, height, c.clusters[h.cluster as usize].class));
            if k == 0 {
                gap = Some(-height);
            }
        }
    }
    let top = hits.iter().map(|h| h.1).fold(f32::NEG_INFINITY, f32::max);
    let crown: Vec<&(f32, f32, u8)> = hits.iter().filter(|h| h.1 >= top - CROWN_BAND).collect();
    let first = crown.first()?;
    let lateral = crown.iter().map(|h| h.0).sum::<f32>() / crown.len() as f32;
    let (lo, hi) = hits.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(h.0), hi.max(h.0)));
    Some(RailProbe { gap, crown_lateral: lateral, crown_gap: -top, width: hi - lo + SPACING, class: first.2 })
}

pub fn crown_point(c: &CollisionWorld, p: Vec3, n: Vec3, forward: Vec3, class_mask: u32) -> Option<Vec3> {
    let r = rail_under(c, p, n, forward, class_mask)?;
    let side = n.cross(forward).normalize_or_zero();
    let coarse = p + side * r.crown_lateral;
    let mut best: Option<(f32, Vec3)> = None;
    for k in -15..=15 {
        let origin = coarse + side * (k as f32 * 0.002) + n * ABOVE;
        if let Some(h) = c.ray_cast_masked(origin, -n, REACH, class_mask)
            && best.is_none_or(|(t, _)| h.t < t)
        {
            best = Some((h.t, origin - n * h.t));
        }
    }
    best.map(|b| b.1).or(Some(coarse - n * r.crown_gap))
}

pub fn rail_tangent(c: &CollisionWorld, p: Vec3, forward: Vec3, class_mask: u32) -> Option<Vec3> {
    const BASELINE: f32 = 0.75;
    const MAX_STEP: f32 = 0.25;
    let f = forward.normalize_or_zero();
    let n = path_up(f);
    let a = crown_point(c, p - f * BASELINE, n, f, class_mask)?;
    let b = crown_point(c, p + f * BASELINE, n, f, class_mask)?;
    let d = b - a;
    let off = d - f * d.dot(f);
    (off.length() <= 2.0 * MAX_STEP).then(|| d.normalize_or_zero()).filter(|t| *t != Vec3::ZERO)
}

pub fn off_tangent(v: Vec3, t: Vec3) -> (f32, f32) {
    let (v, up) = (v.normalize_or_zero(), path_up(t));
    let side = up.cross(t).normalize_or_zero();
    (v.dot(up).clamp(-1.0, 1.0).asin().to_degrees(), v.dot(side).clamp(-1.0, 1.0).asin().to_degrees())
}

pub fn path_up(v: Vec3) -> Vec3 {
    let up = v.cross(Vec3::Y).cross(v).normalize_or_zero();
    if up == Vec3::ZERO { Vec3::Y } else { up }
}

pub fn jitter(xs: &[f32]) -> (f32, f32) {
    let d2: Vec<f32> = xs.windows(3).map(|w| w[2] - 2.0 * w[1] + w[0]).collect();
    if d2.is_empty() {
        return (0.0, 0.0);
    }
    let rms = (d2.iter().map(|d| d * d).sum::<f32>() / d2.len() as f32).sqrt();
    (rms, d2.iter().fold(0.0f32, |m, d| m.max(d.abs())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jitter_is_zero_on_a_straight_line_and_sees_a_kink() {
        let line: Vec<f32> = (0..10).map(|k| 0.3 * k as f32 + 1.0).collect();
        assert!(jitter(&line).1 < 1e-5);
        let mut kink = line.clone();
        kink[5] += 0.05;
        let (rms, max) = jitter(&kink);
        assert!((max - 0.1).abs() < 1e-5 && rms > 0.02, "{rms} {max}");
        assert_eq!(jitter(&[1.0, 2.0]), (0.0, 0.0));
    }

    #[test]
    fn a_velocity_off_the_tangent_reads_in_degrees() {
        let t = Vec3::new(1.0, 0.0, 0.0);
        let (up, side) = off_tangent(Vec3::new(1.0, 0.1, 0.0), t);
        assert!((up - 0.1f32.atan().to_degrees()).abs() < 1e-3 && side.abs() < 1e-4, "{up} {side}");
        let (up, side) = off_tangent(Vec3::new(2.0, 0.0, -0.2), t);
        assert!(up.abs() < 1e-4 && (side.abs() - 0.1f32.atan().to_degrees()).abs() < 1e-3, "{up} {side}");
        let t = Vec3::new(0.8, -0.6, 0.0);
        let (up, side) = off_tangent(t * 0.4, t);
        assert!(up.abs() < 1e-3 && side.abs() < 1e-3);
    }

    #[test]
    fn the_path_up_is_perpendicular_to_the_travel() {
        let v = Vec3::new(1.0, -0.5, 0.2);
        let up = path_up(v);
        assert!(up.dot(v).abs() < 1e-6 && up.y > 0.8 && (up.length() - 1.0).abs() < 1e-6);
        assert_eq!(path_up(Vec3::ZERO), Vec3::Y);
        assert_eq!(path_up(Vec3::new(0.0, -2.0, 0.0)), Vec3::Y);
    }
}
