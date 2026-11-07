use std::collections::BTreeMap;

use ssx_content::ContentId;

use crate::value::ElementId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkerHitShape {
    None,
    Sphere,
    Cylinder,
    Ring,
    Gate,
}

impl MarkerHitShape {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "sphere" => Self::Sphere,
            "cylinder" => Self::Cylinder,
            "ring" => Self::Ring,
            "gate" => Self::Gate,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerView {
    pub element: ElementId,
    pub package: String,
    pub id: ContentId,
    pub position: [f32; 3],
    pub heading: f32,
    pub size: f32,
    pub colour: u32,
    pub visible: bool,
    pub hit: MarkerHitShape,
    pub hit_scale: f32,
    pub sound: Option<ContentId>,
    pub(crate) vertices: u64,
    pub(crate) triangles: u64,
    pub(crate) inside: bool,
    pub instance: Option<ElementId>,
    pub(crate) elsewhere: bool,
}

impl MarkerView {
    pub(crate) fn of_level(element: ElementId, m: &super::LevelMarker) -> MarkerView {
        MarkerView {
            element,
            package: String::new(),
            id: m.id.clone(),
            position: m.position,
            heading: m.heading,
            size: m.size,
            colour: m.colour,
            visible: true,
            hit: m.hit,
            hit_scale: 1.0,
            sound: m.sound.clone(),
            vertices: 0,
            triangles: 0,
            inside: false,
            instance: None,
            elsewhere: false,
        }
    }

    pub fn is_level(&self) -> bool {
        self.package.is_empty()
    }

    pub fn placed(element: ElementId, package: &str, id: ContentId, position: [f32; 3]) -> MarkerView {
        MarkerView {
            element,
            package: package.to_string(),
            id,
            position,
            heading: 0.0,
            size: 1.0,
            colour: 0xFFFF_FFFF,
            visible: true,
            hit: MarkerHitShape::Sphere,
            hit_scale: 1.0,
            sound: None,
            vertices: 0,
            triangles: 0,
            inside: false,
            instance: None,
            elsewhere: false,
        }
    }
}

#[derive(Default)]
pub struct Markers {
    pub(crate) by_element: BTreeMap<ElementId, MarkerView>,
    pub(crate) level: Option<u64>,
    pub(crate) generation: u64,
    pub(crate) vertices: u64,
    pub(crate) triangles: u64,
}

impl Markers {
    pub(crate) fn count_of(&self, package: &str) -> usize {
        self.by_element.values().filter(|m| m.package == package).count()
    }

    pub(crate) fn scripts_count(&self) -> usize {
        self.by_element.values().filter(|m| !m.is_level()).count()
    }

    pub(crate) fn insert(&mut self, m: MarkerView) {
        self.vertices += m.vertices;
        self.triangles += m.triangles;
        self.by_element.insert(m.element, m);
        self.generation += 1;
    }

    pub(crate) fn forget(&mut self, gone: &[ElementId]) {
        for e in gone {
            if let Some(m) = self.by_element.remove(e) {
                self.vertices = self.vertices.saturating_sub(m.vertices);
                self.triangles = self.triangles.saturating_sub(m.triangles);
                self.generation += 1;
            }
        }
    }

    pub(crate) fn touch(&mut self) {
        self.generation += 1;
    }

    pub fn list(&self) -> Vec<MarkerView> {
        self.by_element.values().map(|m| MarkerView { visible: m.visible && !m.elsewhere, ..m.clone() }).collect()
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn nearest(p0: [f32; 3], p1: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let d = sub(p1, p0);
    let len2 = dot(d, d);
    let t = if len2 > 0.0 { (dot(sub(c, p0), d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    [p0[0] + d[0] * t, p0[1] + d[1] * t, p0[2] + d[2] * t]
}

pub(crate) fn hit(m: &mut MarkerView, p0: [f32; 3], p1: [f32; 3]) -> Option<bool> {
    let half = m.size * m.hit_scale * 0.5;
    let c = m.position;
    match m.hit {
        MarkerHitShape::None => None,
        MarkerHitShape::Sphere => {
            let q = nearest(p0, p1, c);
            let d = sub(q, c);
            let now = dot(sub(p1, c), sub(p1, c)) <= half * half;
            let touched = dot(d, d) <= half * half;
            let entered = touched && !m.inside;
            m.inside = now;
            entered.then_some(true)
        }
        MarkerHitShape::Cylinder => {
            let (lo, hi) = (c[1], c[1] + 2.0 * half);
            let (dy, mut t0, mut t1) = (p1[1] - p0[1], 0.0f32, 1.0f32);
            if dy.abs() < f32::EPSILON {
                if p0[1] < lo || p0[1] > hi {
                    m.inside = false;
                    return None;
                }
            } else {
                let (a, b) = ((lo - p0[1]) / dy, (hi - p0[1]) / dy);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
            }
            let at = |t: f32| [p0[0] + (p1[0] - p0[0]) * t, 0.0, p0[2] + (p1[2] - p0[2]) * t];
            let flat = [c[0], 0.0, c[2]];
            let touched = t0 <= t1 && {
                let q = nearest(at(t0), at(t1), flat);
                dot(sub(q, flat), sub(q, flat)) <= half * half
            };
            let end = sub([p1[0], 0.0, p1[2]], flat);
            let now = p1[1] >= lo && p1[1] <= hi && dot(end, end) <= half * half;
            let entered = touched && !m.inside;
            m.inside = now;
            entered.then_some(true)
        }
        MarkerHitShape::Ring | MarkerHitShape::Gate => {
            let (s, co) = m.heading.to_radians().sin_cos();
            let normal = [s, 0.0, co];
            let (a, b) = (dot(sub(p0, c), normal), dot(sub(p1, c), normal));
            if !(a < 0.0 && b >= 0.0 || a > 0.0 && b <= 0.0) {
                return None;
            }
            let t = a / (a - b);
            let x = [p0[0] + (p1[0] - p0[0]) * t, p0[1] + (p1[1] - p0[1]) * t, p0[2] + (p1[2] - p0[2]) * t];
            let off = sub(x, c);
            let inside = match m.hit {
                MarkerHitShape::Ring => dot(off, off) <= half * half,
                _ => {
                    let right = [co, 0.0, -s];
                    dot(off, right).abs() <= half && off[1] >= -0.5 && off[1] <= half * 2.0
                }
            };
            inside.then_some(a < 0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker(hit: MarkerHitShape, heading: f32) -> MarkerView {
        MarkerView {
            element: ElementId(9),
            package: "p".into(),
            id: "ava:marker/ring".parse().unwrap(),
            position: [0.0, 10.0, 0.0],
            heading,
            size: 4.0,
            colour: 0xFFFF_FFFF,
            visible: true,
            hit,
            hit_scale: 1.0,
            sound: None,
            vertices: 0,
            triangles: 0,
            inside: false,
            instance: None,
            elsewhere: false,
        }
    }

    #[test]
    fn a_gate_is_hit_crossing_its_plane_inside_its_opening_once_per_crossing() {
        let mut g = marker(MarkerHitShape::Gate, 0.0);
        assert_eq!(hit(&mut g, [1.0, 11.0, -3.0], [1.0, 11.0, 2.0]), Some(true));
        assert_eq!(hit(&mut g, [1.0, 11.0, 2.0], [1.0, 11.0, 6.0]), None);
        assert_eq!(hit(&mut g, [1.0, 11.0, 6.0], [1.0, 11.0, -1.0]), Some(false));
        assert_eq!(hit(&mut g, [2.5, 11.0, -3.0], [2.5, 11.0, 2.0]), None);
        assert_eq!(hit(&mut g, [0.0, 14.5, -3.0], [0.0, 14.5, 2.0]), None);
        let mut r = marker(MarkerHitShape::Gate, 90.0);
        assert_eq!(hit(&mut r, [-3.0, 11.0, 0.5], [3.0, 11.0, 0.5]), Some(true));
        assert_eq!(hit(&mut r, [0.5, 11.0, -3.0], [0.5, 11.0, 3.0]), None, "along its plane");
    }

    #[test]
    fn a_ring_takes_its_disc_and_a_sphere_fires_once_per_entry() {
        let mut r = marker(MarkerHitShape::Ring, 0.0);
        assert_eq!(hit(&mut r, [0.0, 11.5, -1.0], [0.0, 11.5, 1.0]), Some(true));
        assert_eq!(hit(&mut r, [0.0, 12.5, -1.0], [0.0, 12.5, 1.0]), None, "outside its radius of 2 m");
        let mut s = marker(MarkerHitShape::Sphere, 0.0);
        assert_eq!(hit(&mut s, [-30.0, 10.0, 0.0], [30.0, 10.0, 0.0]), Some(true));
        assert_eq!(hit(&mut s, [-30.0, 10.0, 1.0], [0.0, 10.0, 1.0]), Some(true), "left it, then entered again");
        assert_eq!(hit(&mut s, [0.0, 10.0, 1.0], [0.5, 10.0, 1.0]), None, "still inside");
        let mut n = marker(MarkerHitShape::None, 0.0);
        assert_eq!(hit(&mut n, [0.0, 10.0, -3.0], [0.0, 10.0, 3.0]), None);
    }

    #[test]
    fn a_cylinder_takes_its_height() {
        let mut c = marker(MarkerHitShape::Cylinder, 0.0);
        assert_eq!(hit(&mut c, [-5.0, 12.0, 0.0], [5.0, 12.0, 0.0]), Some(true));
        let mut c = marker(MarkerHitShape::Cylinder, 0.0);
        assert_eq!(hit(&mut c, [-5.0, 15.0, 0.0], [5.0, 15.0, 0.0]), None, "above its 4 m");
        let mut c = marker(MarkerHitShape::Cylinder, 0.0);
        assert_eq!(hit(&mut c, [0.0, 20.0, 0.0], [0.0, 0.0, 0.0]), Some(true), "falling through it");
    }
}
