use std::collections::BTreeMap;

use ssx_assets::career::globe::{GlobeData, Shot};

use crate::game::fe::globe::scene::{Camera, M4, Quat, V3, inverse_rigid, mmul, qmat, qrot, transform};

const STAGE: (f32, f32) = crate::game::fe::globe::STAGE;

#[cfg(test)]
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn xyz(p: [f32; 4]) -> V3 {
    [p[0], p[1], p[2]]
}

pub fn quat_roll_pitch_yaw(r: V3) -> Quat {
    let (sp, cp) = (r[0] * 0.5).sin_cos();
    let (sy, cy) = (r[1] * 0.5).sin_cos();
    let (sr, cr) = (r[2] * 0.5).sin_cos();
    [sp * cy * cr + cp * sy * sr, cp * sy * cr - sp * cy * sr, cp * cy * sr - sp * sy * cr, cp * cy * cr + sp * sy * sr]
}

pub fn scale_quat_pos(scale: V3, q: Quat, pos: V3) -> M4 {
    let r = qmat(q);
    let mut m = r;
    for (i, row) in m.iter_mut().take(3).enumerate() {
        for v in row.iter_mut().take(3) {
            *v *= scale[i];
        }
    }
    m[3] = [pos[0], pos[1], pos[2], 1.0];
    m
}

pub fn inverse_scale_quat_pos(scale: V3, q: Quat, pos: V3) -> M4 {
    let inv_r = inverse_rigid(&qmat(q));
    let inv_s = [
        [1.0 / scale[0], 0.0, 0.0, 0.0],
        [0.0, 1.0 / scale[1], 0.0, 0.0],
        [0.0, 0.0, 1.0 / scale[2], 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let t = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [-pos[0], -pos[1], -pos[2], 1.0]];
    mmul(&mmul(&t, &inv_r), &inv_s)
}

#[derive(Clone, Debug, Default)]
pub struct RangeNode {
    pub key: u32,
    pub shot: Shot,
    pub turn: f32,
    pub scale: V3,
    pub rotation: Quat,
    pub position: V3,
    pub world: M4,
    pub old_inverse: M4,
    pub drop_scale: f32,
}

#[derive(Clone, Debug, Default)]
pub struct PeakNode {
    pub key: u32,
    pub range: u32,
    pub position: V3,
    pub shot_drop: Shot,
    pub shot_peak: Shot,
    pub has_drops: bool,
    pub dd: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DropNode {
    pub key: u32,
    pub peak: u32,
    pub position: V3,
    pub screen_x: f32,
    pub title: String,
    pub selectable: bool,
    pub drop_type: i64,
    pub custom_status: i64,
    pub event_type: i64,
}

#[derive(Clone, Debug, Default)]
pub struct NodeMap {
    pub ranges: BTreeMap<u32, RangeNode>,
    pub peaks: BTreeMap<u32, PeakNode>,
    pub drops: BTreeMap<u32, DropNode>,
}

const PEAK_STAND_IN: (V3, V3) = ([-39.751, 1211.109, -17.263], [20.0, 0.0, 0.0]);
const DROP_STAND_IN: (V3, V3) = ([-51.466, 1210.353, -16.867], [1.5, -0.8, 0.0]);

fn is_zero(p: V3) -> bool {
    p.iter().all(|c| c.abs() <= 1.192_092_9e-7)
}

impl NodeMap {
    pub fn build(data: &GlobeData, widescreen: bool) -> Self {
        let mut map = NodeMap::default();
        for r in &data.ranges {
            map.ranges.insert(r.key, range_node(r));
        }
        for p in &data.peaks {
            let Some(range) = map.ranges.get(&p.range) else { continue };
            let carry = |s: &Shot| Shot {
                offset: xyz(transform(xyz(transform(s.offset, &range.old_inverse)), &range.world)),
                look: xyz(transform(xyz(transform(s.look, &range.old_inverse)), &range.world)),
                ..*s
            };
            let shot_peak = if widescreen { &p.shot_peak_wide } else { &p.shot_peak };
            let position = if is_zero(p.position) { [0.0; 3] } else { xyz(transform(p.position, &range.world)) };
            map.peaks.insert(
                p.key,
                PeakNode {
                    key: p.key,
                    range: p.range,
                    position,
                    shot_drop: carry(&p.shot_drop),
                    shot_peak: carry(shot_peak),
                    has_drops: false,
                    dd: p.dd,
                },
            );
        }
        for d in &data.drops {
            let Some(peak) = map.peaks.get_mut(&d.peak) else { continue };
            let Some(range) = map.ranges.get(&peak.range) else { continue };
            peak.has_drops = true;
            let p = d.position.map(|c| c * range.drop_scale);
            let position = if is_zero(p) { [0.0; 3] } else { xyz(transform(p, &range.world)) };
            map.drops.insert(
                d.key,
                DropNode {
                    key: d.key,
                    peak: d.peak,
                    position,
                    screen_x: 0.0,
                    title: d.title.clone(),
                    selectable: d.selectable,
                    drop_type: d.drop_type,
                    custom_status: d.custom_status,
                    event_type: d.event_type,
                },
            );
        }
        map.stand_ins();
        let aspect = STAGE.0 / STAGE.1;
        let cams: BTreeMap<u32, M4> = map
            .peaks
            .values()
            .map(|p| {
                let c = Camera::from_shot(&p.shot_drop);
                (p.key, mmul(&c.view(), &c.projection(aspect)))
            })
            .collect();
        for d in map.drops.values_mut() {
            if let Some(vp) = cams.get(&d.peak) {
                d.screen_x = stage_point(d.position, vp).0;
            }
        }
        map
    }

    fn stand_ins(&mut self) {
        let ranges: Vec<u32> = self.ranges.keys().copied().collect();
        for range in ranges {
            let keys: Vec<u32> = self.peaks.values().filter(|p| p.range == range).map(|p| p.key).collect();
            let mut prev: Option<V3> = None;
            for k in keys {
                let Some(p) = self.peaks.get_mut(&k) else { continue };
                if is_zero(p.position) {
                    p.position = prev.map_or(PEAK_STAND_IN.0, |q| add(q, PEAK_STAND_IN.1));
                }
                prev = Some(p.position);
                let drops: Vec<u32> = self.drops.values().filter(|d| d.peak == k).map(|d| d.key).collect();
                let mut prev_drop: Option<V3> = None;
                for dk in drops {
                    let Some(d) = self.drops.get_mut(&dk) else { continue };
                    if is_zero(d.position) {
                        d.position = prev_drop.map_or(DROP_STAND_IN.0, |q| add(q, DROP_STAND_IN.1));
                    }
                    prev_drop = Some(d.position);
                }
            }
        }
    }

    pub fn peaks_of(&self, range: u32) -> impl Iterator<Item = &PeakNode> {
        self.peaks.values().filter(move |p| p.range == range)
    }

    pub fn drops_of(&self, peak: u32) -> impl Iterator<Item = &DropNode> {
        self.drops.values().filter(move |d| d.peak == peak)
    }

    pub fn peak_of(&self, drop: u32) -> Option<u32> {
        self.drops.get(&drop).map(|d| d.peak)
    }

    pub fn range_of(&self, peak: u32) -> Option<u32> {
        self.peaks.get(&peak).map(|p| p.range)
    }
}

pub fn stage_point(p: V3, vp: &M4) -> (f32, f32) {
    let c = transform(p, vp);
    ((c[0] / c[3] + 1.0) * STAGE.0 * 0.5, (1.0 - c[1] / c[3]) * STAGE.1 * 0.5)
}

fn range_node(r: &ssx_assets::career::globe::RangeView) -> RangeNode {
    let cam = Camera::from_shot(&r.shot);
    let f = cam.forward;
    let mut a = (-f[2]).clamp(-1.0, 1.0).acos();
    if f[0] < 0.0 {
        a = std::f32::consts::TAU - a;
    }
    let q_turn = quat_roll_pitch_yaw([0.0, a, 0.0]);
    let look = add(r.shot.offset, qrot(q_turn, f));
    let shot = Shot { look, ..r.shot };
    let rot = r.midground_rotation.map(f32::to_radians);
    let (scale, pos_old) = (r.midground_scale, r.midground_position);
    let q_old = quat_roll_pitch_yaw(rot);
    let old_inverse = inverse_scale_quat_pos(scale, q_old, pos_old);
    let rotation = quat_roll_pitch_yaw([rot[0], rot[1] + a, rot[2]]);
    let new_cam = Camera::from_shot(&shot);
    let position = xyz(transform(xyz(transform(pos_old, &cam.view())), &inverse_rigid(&new_cam.view())));
    RangeNode {
        key: r.key,
        shot,
        turn: a,
        scale,
        rotation,
        position,
        world: scale_quat_pos(scale, rotation, position),
        old_inverse,
        drop_scale: r.drop_scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_turn_points_the_camera_down_minus_z() {
        let r = ssx_assets::career::globe::RangeView {
            shot: Shot { offset: [20.0, 1200.0, -77.0], look: [21.0, 1200.0, -76.0], target: [0.0; 3], fov: 1.0 },
            midground_position: [0.0, 1200.0, 0.0],
            midground_scale: [0.001; 3],
            drop_scale: 1.0,
            ..Default::default()
        };
        let n = range_node(&r);
        let d = sub(n.shot.look, n.shot.offset);
        assert!(d[0].abs() < 1e-5 && (d[2] + 1.0).abs() < 1e-5, "{d:?}");
        let before = sub(r.midground_position, r.shot.offset);
        let after = sub(n.position, n.shot.offset);
        let len = |v: V3| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((len(before) - len(after)).abs() < 1e-2);
        let p = [12.0, -3.0, 40.0];
        let m = scale_quat_pos([0.5, 2.0, 1.0], quat_roll_pitch_yaw([0.3, 1.1, -0.2]), [5.0, 6.0, 7.0]);
        let i = inverse_scale_quat_pos([0.5, 2.0, 1.0], quat_roll_pitch_yaw([0.3, 1.1, -0.2]), [5.0, 6.0, 7.0]);
        let q = xyz(transform(xyz(transform(p, &m)), &i));
        assert!(sub(p, q).iter().all(|c| c.abs() < 1e-3), "{q:?}");
    }
}
