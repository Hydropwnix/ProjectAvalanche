use super::{MAX_COORDINATE, Placement};
use crate::model::Transform;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VolumeShape {
    Box { centre: [f32; 3], rotation: [f32; 4], half_extents: [f32; 3] },
    Sphere { centre: [f32; 3], radius: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Volume {
    pub node: String,
    pub name: String,
    pub order: u32,
    pub shape: VolumeShape,
}

const SHEAR_TOLERANCE: f64 = 1e-4;

pub fn placement(xf: &Transform) -> Result<Placement, String> {
    let m = &xf.0;
    let mut cols = [[m[0][0], m[0][1], m[0][2]], [m[1][0], m[1][1], m[1][2]], [m[2][0], m[2][1], m[2][2]]];
    let len = |c: &[f64; 3]| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
    let dot = |a: &[f64; 3], b: &[f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut s = [len(&cols[0]), len(&cols[1]), len(&cols[2])];
    if s.iter().any(|x| !x.is_finite() || *x < 1e-9) {
        return Err("a scale of zero (the node is flattened)".into());
    }
    for (a, b) in [(0, 1), (1, 2), (0, 2)] {
        if dot(&cols[a], &cols[b]).abs() > SHEAR_TOLERANCE * s[a] * s[b] {
            return Err("a sheared transform (a parent's non-uniform scale under a rotation); a placement is scale, \
                        rotation and position"
                .into());
        }
    }
    for (c, k) in cols.iter_mut().zip(s) {
        c.iter_mut().for_each(|x| *x /= k);
    }
    let cross = [
        cols[0][1] * cols[1][2] - cols[0][2] * cols[1][1],
        cols[0][2] * cols[1][0] - cols[0][0] * cols[1][2],
        cols[0][0] * cols[1][1] - cols[0][1] * cols[1][0],
    ];
    if dot(&cross, &cols[2]) < 0.0 {
        cols[0] = cols[0].map(|x| -x);
        s[0] = -s[0];
    }
    let rotation = quaternion(&cols);
    let position = [m[3][0], m[3][1], m[3][2]];
    if position.iter().any(|x| !x.is_finite() || x.abs() > f64::from(MAX_COORDINATE)) {
        return Err(format!("a position beyond ±{MAX_COORDINATE} m"));
    }
    if s.iter().any(|x| x.abs() > f64::from(MAX_COORDINATE)) {
        return Err(format!("a scale beyond {MAX_COORDINATE}"));
    }
    Ok(Placement {
        scale: s.map(|x| x as f32),
        rotation: rotation.map(|x| x as f32),
        position: position.map(|x| x as f32),
    })
}

fn quaternion(c: &[[f64; 3]; 3]) -> [f64; 4] {
    let r = |row: usize, col: usize| c[col][row];
    let trace = r(0, 0) + r(1, 1) + r(2, 2);
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(r(2, 1) - r(1, 2)) / s, (r(0, 2) - r(2, 0)) / s, (r(1, 0) - r(0, 1)) / s, 0.25 * s]
    } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
        let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
        [0.25 * s, (r(0, 1) + r(1, 0)) / s, (r(0, 2) + r(2, 0)) / s, (r(2, 1) - r(1, 2)) / s]
    } else if r(1, 1) > r(2, 2) {
        let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
        [(r(0, 1) + r(1, 0)) / s, 0.25 * s, (r(1, 2) + r(2, 1)) / s, (r(0, 2) - r(2, 0)) / s]
    } else {
        let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
        [(r(0, 2) + r(2, 0)) / s, (r(1, 2) + r(2, 1)) / s, 0.25 * s, (r(1, 0) - r(0, 1)) / s]
    };
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
    q.map(|x| sign * x / l)
}

pub fn mesh_box(xf: &Transform, lo: [f32; 3], hi: [f32; 3]) -> Result<VolumeShape, String> {
    let p = placement(xf)?;
    let local_centre = std::array::from_fn(|k| (f64::from(lo[k]) + f64::from(hi[k])) * 0.5);
    let centre = xf.point_f64(local_centre).map(|x| x as f32);
    let half_extents = std::array::from_fn(|k| ((hi[k] - lo[k]) * 0.5 * p.scale[k]).abs());
    if centre.iter().chain(&half_extents).any(|x| !x.is_finite() || x.abs() > MAX_COORDINATE) {
        return Err(format!("a volume beyond ±{MAX_COORDINATE} m"));
    }
    Ok(VolumeShape::Box { centre, rotation: p.rotation, half_extents })
}

pub fn sphere(xf: &Transform, radius: f32) -> Result<VolumeShape, String> {
    let p = placement(xf)?;
    let r = radius * p.scale.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if !r.is_finite() || r <= 0.0 || r > MAX_COORDINATE {
        return Err(format!("a radius of {r} m once scaled (above 0, at most {MAX_COORDINATE})"));
    }
    Ok(VolumeShape::Sphere { centre: p.position, radius: r })
}
