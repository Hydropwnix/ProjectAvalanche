use std::collections::BTreeMap;

use anyhow::{Context, Result};
use bevy::math::Affine3A;
use bevy::prelude::{IVec2, Quat, Vec2, Vec3};
use ssx_assets::light_script::LightBehaviour;
use ssx_assets::track_lights::text::Strings;
use ssx_assets::track_lights::{LightNode, LightShape, load_track_lights};

use super::{CELL, Kind, Placed, Projected, TEXT_LAYER, light_words, sphere_cos};
use crate::render::lighting::game::GameLightUniform;

#[derive(Default)]
pub(super) struct Words(pub(super) Vec<[u32; 4]>);

impl Words {
    fn f(&mut self, v: [f32; 4]) {
        self.0.push(v.map(f32::to_bits));
    }
    fn u(&mut self, v: [u32; 4]) {
        self.0.push(v);
    }
    pub(super) fn bytes(&self) -> Vec<u8> {
        self.0.iter().flatten().flat_map(|w| w.to_le_bytes()).collect()
    }
}

const HEADER: u32 = 4;

const STACKS: u32 = 10;
const SLICES: u32 = 10;
const RECORD: u32 = 10;

pub(super) fn collect(
    big_root: &std::path::Path,
    track: Option<&str>,
    map: &[(u32, ssx_assets::track_objects::Placement)],
) -> Result<(Vec<Placed>, String)> {
    let mut lights = match track {
        Some(t) => load_track_lights(big_root, t).with_context(|| format!("{t}: light nodes"))?,
        None => Default::default(),
    };
    if !map.is_empty() {
        lights.extend(ssx_assets::track_lights::map_lights(big_root, map).context("the map's placed objects' lights")?);
    }
    let mut placed = Vec::new();
    let (mut entity, mut off, mut text) = (0u32, 0usize, 0usize);
    let mut strings: Option<Strings> = None;
    let text_off = std::env::var("SSX_TEXT_PROJECTIONS").is_ok_and(|v| v == "off");
    for (set, (object, placements)) in lights.instances.iter().enumerate() {
        let object = &lights.objects[*object];
        // TODO(guess): "controller ⇒ dynamic light" is inferred (0x8298FA18; 0x829241C8 registers static lights only for descriptor type 0 / 5, byte +0x18 0); settle: the writer of the descriptor's +0x14 / +0x18 (the instance-set build).
        let behaviour = object.controller_script.as_ref().map(|_| LightBehaviour::Off);
        let object_key = object.key;
        let nodes = object.lods.first().map(Vec::as_slice).unwrap_or_default();
        for (pi, placement) in placements.iter().enumerate() {
            let instance = crate::render::objects::placement_transform(placement).compute_affine();
            for (ni, node) in nodes.iter().enumerate() {
                let Some(light) = &node.light else { continue };
                if !light.enable {
                    off += 1;
                    continue;
                }
                let (kind, texture) = match &light.shape {
                    LightShape::Point { .. } => (Kind::Point, None),
                    LightShape::Spot { .. } => (Kind::Spot, None),
                    LightShape::Box { unk_ebeca5b9: true, .. } if text_off => {
                        off += 1;
                        continue;
                    }
                    LightShape::Box { unk_ebeca5b9: true, unk_fe912176: key, .. } => {
                        if strings.is_none() {
                            strings = Some(Strings::load(big_root)?);
                        }
                        let string = strings.as_ref().map_or_else(|| key.clone(), |s| s.text(key));
                        let colour = [light.color[0], light.color[1], light.color[2]].map(f32::to_bits);
                        text += 1;
                        (Kind::Textured, Some(Projected::Text { string, colour }))
                    }
                    LightShape::Box { projected_texture: Some(t), .. } => {
                        (Kind::Textured, Some(Projected::File(t.clone())))
                    }
                    LightShape::Box { .. } => (Kind::Box, None),
                };
                let bin = kind as u32 * 2 + light.enable_specular as u32;
                // TODO(guess): dynamic lights within a bin follow the track's order, the rider's after them; settle: the DynamicLightsEntity registration order (0x8297FDF0 callers).
                let order = match behaviour {
                    None => (bin, 0, set, ni, pi),
                    Some(_) => (bin, 1, set, pi, ni),
                };
                placed.push(Placed {
                    world: instance * node_affine(node),
                    light: light.clone(),
                    factor: None,
                    ctrl: behaviour.map(|_| (object_key, placement.position.map(f32::to_bits))),
                    node: node_affine(node),
                    scale: Vec3::from_array(placement.scale),
                    order,
                    behaviour,
                    entity,
                    texture,
                    texture_layer: 0,
                });
            }
            if behaviour.is_some() {
                entity += 1;
            }
        }
    }
    placed.sort_by_key(|p| p.order);
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &placed {
        *kinds.entry(p.light.shape.kind()).or_default() += 1;
    }
    let summary = format!(
        "local lights: {} placed ({kinds:?}), {} on entities' controllers; {off} off (Enable), {text} text projections",
        placed.len(),
        placed.iter().filter(|p| p.behaviour.is_some()).count()
    );
    Ok((placed, summary))
}

fn node_affine(n: &LightNode) -> Affine3A {
    let [x, y, z, w] = n.orientation;
    Affine3A::from_scale_rotation_translation(
        Vec3::new(n.scale[0], n.scale[1], n.scale[2]),
        Quat::from_xyzw(x, y, z, w),
        Vec3::new(n.position[0], n.position[1], n.position[2]),
    )
}

pub(super) fn assign_layers(placed: &mut [Placed]) -> Vec<Projected> {
    let mut paths: Vec<Projected> = Vec::new();
    for p in placed.iter_mut() {
        if let Some(t) = &p.texture {
            p.texture_layer = match paths.iter().position(|q| q == t) {
                Some(i) => i as u32,
                None => {
                    paths.push(t.clone());
                    paths.len() as u32 - 1
                }
            };
        }
    }
    paths
}

fn constants(p: &Placed) -> ([f32; 3], [f32; 3], Vec3, f32, [f32; 2]) {
    let m = p.world.matrix3;
    let scale = m.x_axis.length().max(m.y_axis.length()).max(m.z_axis.length());
    let origin = Vec3::from(p.world.translation);
    match p.light.shape {
        LightShape::Point { outer_radius, inner_radius } => {
            let a1 = [inner_radius, outer_radius, 1.0 / p.light.fall_off];
            let s = outer_radius / sphere_cos();
            (a1, [0.0; 3], origin, outer_radius * scale, [s, s])
        }
        LightShape::Spot { cone_angle, penumbra_angle, outer_length, inner_length, drop_off } => {
            let half = cone_angle * 0.5;
            let outer = penumbra_angle.max(0.0) + half;
            let inner = penumbra_angle.min(0.0) + half;
            let a1 = [inner_length, outer_length, 1.0 / p.light.fall_off];
            let a2 = [(inner as f64).cos() as f32, (outer as f64).cos() as f32, drop_off];
            let r = ((outer as f64).tan() as f32) * outer_length;
            (a1, a2, origin, outer_length * scale, [r, outer_length])
        }
        LightShape::Box { width, height, penumbra, outer_length, inner_length, .. } => {
            let a1 = [inner_length, outer_length, 1.0 / p.light.fall_off];
            let a2 = [width * 0.5, height * 0.5, penumbra];
            let centre = p.world.transform_point3(Vec3::new(0.0, 0.0, -0.5 * outer_length));
            let radius = 0.5 * Vec3::new(width, height, outer_length).length() * scale;
            (a1, a2, centre, radius, [0.0, 0.0])
        }
    }
}

fn flags(p: &Placed, kind: u32) -> u32 {
    let mode = match (p.factor, p.behaviour) {
        (Some(_), _) | (None, Some(LightBehaviour::Off)) => 3,
        (None, None | Some(LightBehaviour::On)) => 0,
        (None, Some(LightBehaviour::Flicker { .. })) => 1,
        (None, Some(LightBehaviour::Blink { .. })) => 2,
    };
    let text = if matches!(p.texture, Some(Projected::Text { .. })) { TEXT_LAYER } else { 0 };
    kind | (p.light.enable_specular as u32) << 2 | text | mode << 4 | p.texture_layer << 8
}

pub(super) fn bin_of(p: &Placed) -> u32 {
    let kind = match (&p.light.shape, &p.texture) {
        (LightShape::Point { .. }, _) => Kind::Point,
        (LightShape::Spot { .. }, _) => Kind::Spot,
        (LightShape::Box { .. }, Some(_)) => Kind::Textured,
        (LightShape::Box { .. }, None) => Kind::Box,
    };
    kind as u32 * 2 + p.light.enable_specular as u32
}

pub(super) fn record(w: &mut Words, p: &Placed) {
    let (a1, a2, centre, radius, vol) = constants(p);
    let inv = p.world.inverse();
    let m = inv.matrix3;
    let t = Vec3::from(inv.translation);
    for k in 0..3 {
        let row = m.row(k);
        w.f([row.x, row.y, row.z, t[k]]);
    }
    let kind = bin_of(p) / 2;
    let o = Vec3::from(p.world.translation);
    w.u([o.x.to_bits(), o.y.to_bits(), o.z.to_bits(), flags(p, kind)]);
    let c = p.light.color;
    w.f([c[0], c[1], c[2], p.light.diffuse_intensity]);
    w.f([a1[0], a1[1], a1[2], p.light.specular_intensity]);
    w.f([a2[0], a2[1], a2[2], radius]);
    let axis = Vec3::from(p.world.matrix3.z_axis).normalize_or_zero();
    w.u([axis.x.to_bits(), axis.y.to_bits(), axis.z.to_bits(), p.entity]);
    let (strength, frequency, gap) = match (p.factor, p.behaviour) {
        (Some(f), _) => (f, 0.0, 0.0),
        (None, Some(LightBehaviour::Flicker { strength })) => (strength, 0.0, 0.0),
        (None, Some(LightBehaviour::Blink { strength, frequency, gap })) => (strength, frequency, gap),
        _ => (0.0, 0.0, 0.0),
    };
    w.f([centre.x, centre.y, centre.z, strength]);
    w.f([frequency, gap, vol[0], vol[1]]);
}

pub(super) fn bounds(p: &Placed) -> [f32; 4] {
    let (_, _, centre, _, _) = constants(p);
    let m = p.world.matrix3;
    let gram = m.transpose() * m;
    let stretch =
        [gram.x_axis, gram.y_axis, gram.z_axis].map(|c| c.abs().element_sum()).into_iter().fold(0.0, f32::max);
    let stretch = stretch.sqrt();
    let (extent, ends) = match p.light.shape {
        LightShape::Point { outer_radius, inner_radius } => (outer_radius, inner_radius <= outer_radius),
        LightShape::Spot { outer_length, inner_length, .. } => (outer_length, inner_length <= outer_length),
        LightShape::Box { width, height, outer_length, inner_length, penumbra, .. } => {
            (0.5 * Vec3::new(width, height, outer_length).length(), inner_length <= outer_length && penumbra >= 0.0)
        }
    };
    let radius = extent.abs() * stretch * 1.0001 + 1e-4;
    [centre.x, centre.y, centre.z, if ends && radius.is_finite() { radius } else { f32::MAX }]
}

pub(super) fn encode(placed: &[Placed], slots: u32, light: &GameLightUniform) -> (Words, usize) {
    let records: Vec<_> = placed.iter().map(constants).collect();
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for (_, _, c, r, _) in &records {
        lo = lo.min(Vec2::new(c.x - r, c.z - r));
        hi = hi.max(Vec2::new(c.x + r, c.z + r));
    }
    if records.is_empty() {
        (lo, hi) = (Vec2::ZERO, Vec2::ZERO);
    }
    let dims = ((hi - lo) / CELL).ceil().max(Vec2::ONE).as_uvec2();
    let mut cells: Vec<Vec<u32>> = vec![Vec::new(); (dims.x * dims.y) as usize];
    for (i, (_, _, c, r, _)) in records.iter().enumerate() {
        let a = ((Vec2::new(c.x - r, c.z - r) - lo) / CELL).floor().as_ivec2().max(IVec2::ZERO);
        let b = ((Vec2::new(c.x + r, c.z + r) - lo) / CELL).floor().as_ivec2().min(dims.as_ivec2() - 1);
        for z in a.y..=b.y {
            for x in a.x..=b.x {
                cells[(z as u32 * dims.x + x as u32) as usize].push(i as u32);
            }
        }
    }
    let planes = sphere_planes();
    let lights_off = HEADER + planes.len() as u32;
    let cells_off = lights_off + RECORD * placed.len() as u32;
    let list_off = cells_off + cells.len() as u32;
    let mut w = Words::default();
    w.u([lo.x.to_bits(), lo.y.to_bits(), CELL.to_bits(), placed.len() as u32]);
    w.u([dims.x, dims.y, cells_off, list_off]);
    w.u([lights_off, HEADER, sphere_cos().to_bits(), 0]);
    w.u([0; 4]);
    for plane in planes {
        w.f(plane);
    }
    for p in placed {
        record(&mut w, p);
    }
    let mut start = 0u32;
    for cell in &cells {
        w.u([start, cell.len() as u32, 0, 0]);
        start += cell.len() as u32;
    }
    let flat: Vec<u32> = cells.concat();
    for chunk in flat.chunks(4) {
        let mut v = [0u32; 4];
        v[..chunk.len()].copy_from_slice(chunk);
        w.u(v);
    }
    let dynamic_at = w.0.len();
    w.0.resize(dynamic_at + (RECORD * slots) as usize, [0; 4]);
    w.0[3] = [dynamic_at as u32, 0, 0, w.0.len() as u32];
    w.0.extend(light_words(light));
    for p in placed {
        w.f(bounds(p));
    }
    w.0.resize(w.0.len() + slots as usize, [0; 4]);
    (w, dynamic_at)
}

fn sphere_planes() -> Vec<[f32; 4]> {
    let v = |i: u32, j: u32| {
        let (t, f) =
            (std::f64::consts::PI * i as f64 / STACKS as f64, std::f64::consts::TAU * j as f64 / SLICES as f64);
        bevy::math::DVec3::new(f.cos() * t.sin(), f.sin() * t.sin(), t.cos())
    };
    let mut planes = Vec::new();
    for i in 0..STACKS {
        for j in 0..SLICES {
            let (a, b, c, d) = (v(i, j), v(i + 1, j), v(i, (j + 1) % SLICES), v(i + 1, (j + 1) % SLICES));
            let (p, q, r) = if i == 0 { (a, b, d) } else { (a, b, c) };
            let mut n = (q - p).cross(r - p).normalize();
            if n.dot(a + b + c + d) < 0.0 {
                n = -n;
            }
            planes.push([n.x as f32, n.y as f32, n.z as f32, n.dot(p) as f32]);
        }
    }
    planes
}

#[cfg(test)]
mod tests {
    use bevy::prelude::Vec3A;

    use super::super::{DYNAMIC_SLOTS, LIGHT_WORDS};
    use super::*;

    #[test]
    fn sphere_volume_planes() {
        let planes = sphere_planes();
        assert_eq!(planes.len(), (STACKS * SLICES) as usize);
        let min = planes.iter().map(|p| p[3]).fold(f32::MAX, f32::min);
        assert!((min - 0.940_447).abs() < 1e-5, "{min}");
        assert!((min / sphere_cos() - 0.988_844).abs() < 1e-5);
        for i in 0..=STACKS {
            for j in 0..SLICES {
                let (t, f) = (std::f32::consts::PI * i as f32 / 10.0, std::f32::consts::TAU * j as f32 / 10.0);
                let v = Vec3::new(f.cos() * t.sin(), f.sin() * t.sin(), t.cos());
                assert!(planes.iter().all(|p| Vec3::new(p[0], p[1], p[2]).dot(v) <= p[3] + 1e-5));
            }
        }
    }

    #[test]
    fn buffer_header_without_lights() {
        let (w, dynamic_at) = encode(&[], DYNAMIC_SLOTS, &GameLightUniform::default());
        assert_eq!(w.0[0][3], 0);
        assert_eq!(w.0[2][0], HEADER + STACKS * SLICES);
        assert_eq!(dynamic_at as u32, HEADER + STACKS * SLICES + 1);
        let light_at = HEADER + STACKS * SLICES + 1 + RECORD * DYNAMIC_SLOTS;
        assert_eq!(w.0[3], [dynamic_at as u32, 0, 0, light_at]);
        assert_eq!(w.0.len(), light_at as usize + LIGHT_WORDS + DYNAMIC_SLOTS as usize);
    }

    #[test]
    fn bounds_hold_the_attenuation() {
        let shapes = [
            LightShape::Point { outer_radius: 7.0, inner_radius: 1.0 },
            LightShape::Spot {
                cone_angle: 1.2,
                penumbra_angle: 0.1,
                outer_length: 9.0,
                inner_length: 2.0,
                drop_off: 3.0,
            },
            LightShape::Box {
                width: 8.0,
                height: 5.0,
                penumbra: 1.0,
                outer_length: 6.0,
                inner_length: 0.5,
                projected_texture: None,
                unk_fe912176: String::new(),
                unk_ebeca5b9: false,
            },
        ];
        let world = Affine3A::from_cols(
            Vec3A::new(2.0, 0.3, -0.1),
            Vec3A::new(0.9, 0.5, 0.2),
            Vec3A::new(-0.4, 0.2, 1.7),
            Vec3A::new(100.0, -20.0, 35.0),
        );
        let mut seed = 0x2545_f491_u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        for shape in shapes {
            let p = placed(world, shape.clone());
            let b = bounds(&p);
            let (centre, radius) = (Vec3::new(b[0], b[1], b[2]), b[3]);
            let inv = world.inverse();
            let (mut reached, mut beyond) = (0, 0);
            for _ in 0..200_000 {
                let q = Vec3::from(world.translation) + Vec3::new(next(), next(), next()) * 20.0;
                let pl = inv.transform_point3(q);
                let lit = match shape {
                    LightShape::Point { outer_radius: r, .. } | LightShape::Spot { outer_length: r, .. } => {
                        pl.length() < r
                    }
                    LightShape::Box { width, height, outer_length, .. } => {
                        pl.z < 0.0 && -pl.z < outer_length && pl.x.abs() < width * 0.5 && pl.y.abs() < height * 0.5
                    }
                };
                reached += lit as u32;
                beyond += (lit && q.distance(centre) > radius) as u32;
            }
            assert!(reached > 100, "{shape:?}: {reached}");
            assert_eq!(beyond, 0, "{shape:?}");
        }
        let point = |outer_radius, inner_radius| LightShape::Point { outer_radius, inner_radius };
        assert_eq!(bounds(&placed(Affine3A::IDENTITY, point(2.0, 3.0)))[3], f32::MAX);
        let turned = Affine3A::from_scale_rotation_translation(Vec3::splat(3.0), Quat::from_rotation_y(0.7), Vec3::ONE);
        assert!((bounds(&placed(turned, point(2.0, 2.0)))[3] - 6.0).abs() < 2e-3);
    }

    fn placed(world: Affine3A, shape: LightShape) -> Placed {
        let light = ssx_assets::track_lights::SceneLight {
            name: String::new(),
            debug_name: String::new(),
            color: [1.0; 4],
            diffuse_intensity: 1.0,
            specular_intensity: 0.0,
            fall_off: 1.0,
            enable_specular: false,
            enable: true,
            shape,
        };
        Placed {
            world,
            light,
            factor: None,
            ctrl: None,
            node: Affine3A::IDENTITY,
            scale: Vec3::ONE,
            order: (0, 0, 0, 0, 0),
            behaviour: None,
            entity: 0,
            texture: None,
            texture_layer: 0,
        }
    }
}
