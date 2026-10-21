use std::sync::Arc;

use bevy::math::{Mat4, Vec3, Vec4};

use super::{Context, EffectInstance, SetArena, View};

pub type VertexRecord = [Vec4; 3];

#[derive(Clone)]
pub struct Draw {
    pub effect: usize,
    pub world: Mat4,
    pub layer: i32,
    pub half: bool,
    pub tonemap: bool,
    pub shader: String,
    pub technique: String,
    pub texture: String,
    pub time_now: i32,
    pub age_max: i32,
    pub age_warp_inv: f32,
    pub stretch: Vec3,
    pub stretch_per_particle: bool,
    pub color_mult: Vec4,
    pub color_ramp: [Vec4; 4],
    pub z_feather: [f32; 4],
    pub use_global_lighting: bool,
    pub light: (Vec3, Vec4, f32, f32),
    pub modulo_position: bool,
    pub dimensions: Vec3,
    pub stretch_per_camera: bool,
    pub mesh_type: i32,
    pub extras: Vec<(String, Vec4)>,
    pub registry: Option<Arc<Vec<(String, Vec4)>>>,
    pub vertices: Vec<VertexRecord>,
    pub aabb: (Vec3, Vec3),
}

impl EffectInstance {
    fn reference(&self, path: &str) -> Option<Vec4> {
        let mut it = path.split('/');
        let (g, a, p) = (it.next()?, it.next()?, it.next()?);
        let gd = self.def.groups.iter().find(|x| x.name == g)?;
        let ad = gd.actions.iter().find(|x| x.class == a || x.instance_name.as_deref() == Some(a))?;
        Some(View::new(&ad.params, &self.weights).v(p, 0))
    }

    fn group_space(&self, gi: usize) -> i32 {
        let gp = View::new(&self.def.groups[gi].params, &self.weights);
        match (gp.i("iInstancing"), gp.i("iSpace")) {
            (1, _) | (_, 0) => 0,
            (2, _) | (_, 1) => 1,
            _ => View::new(&self.def.params, &self.weights).i("iSpace"),
        }
    }

    pub fn world_matrix(&self) -> Mat4 {
        let (r, t) = self.cur;
        Mat4::from_cols(r.x_axis.extend(0.0), r.y_axis.extend(0.0), r.z_axis.extend(0.0), t.extend(1.0))
    }

    pub fn draws(&self, index: usize, ctx: &Context, view_axis: Vec3, arena: &SetArena, out: &mut Vec<Draw>) {
        for &(gi, ai, count) in &self.rendered {
            let particles = &arena.sets[self.sets[gi]].particles;
            let particles = &particles[..count.min(particles.len())];
            let ad = &self.def.groups[gi].actions[ai];
            {
                let p = View::new(&ad.params, &self.weights);
                let m = if self.group_space(gi) == 0 { self.world_matrix() } else { Mat4::IDENTITY };
                let world = m * Mat4::from_scale(Vec3::splat(p.f("fScale")));
                let age_max = p.text("rKillAge").and_then(|r| self.reference(r)).map_or(0, |v| v.x as i32);
                let (sv, ss) = (p.v3("vStretchVelocity"), p.f("fStretchSpeedScale"));
                let stretch = if ss >= 1.192_092_9e-7 && sv.length() > p.f("fStretchSpeedMin") {
                    sv.normalize() * (1.0 + sv.length() / ss)
                } else {
                    Vec3::ZERO
                };
                let per_particle = p.b("bStretchPerParticle");
                let mut order: Vec<usize> = (0..particles.len()).collect();
                if p.b("bDepthSort") {
                    let key =
                        |i: &usize| !view_axis.dot(world.transform_point3(particles[*i].pos) - ctx.camera).to_bits();
                    order.sort_by_key(key);
                }
                let mut aabb = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                let vertices = order
                    .iter()
                    .map(|&i| {
                        let q = &particles[i];
                        let wp = world.transform_point3(q.pos);
                        aabb = (aabb.0.min(wp), aabb.1.max(wp));
                        let t0 = if per_particle { q.vel * ss } else { q.ori };
                        [q.pos.extend(q.size), t0.extend(q.birth), Vec4::new(q.stamp, 0.0, 0.0, 0.0)]
                    })
                    .collect();
                let extras = p.names().filter(|n| !p.built_in(n)).map(|n| (n.to_owned(), p.v(n, 0))).collect();
                let draw = Draw {
                    effect: index,
                    world,
                    layer: p.i("iRenderLayer"),
                    half: p.b("bHalfSizeRender"),
                    tonemap: p.b("bToneMapRender"),
                    shader: p.text("sShader").unwrap_or_default().to_owned(),
                    technique: p.text("sTechnique").unwrap_or_default().to_owned(),
                    texture: p.text("sTexture").unwrap_or_default().to_owned(),
                    time_now: ctx.time_ms,
                    age_max,
                    age_warp_inv: 1.0 / p.f("fAgeWarp"),
                    stretch,
                    stretch_per_particle: per_particle,
                    color_mult: p.v("vColorMult", 0),
                    color_ramp: [
                        p.v("vColorRamp", 0),
                        p.v("vColorRamp", 1),
                        p.v("vColorRamp", 2),
                        p.v("vColorRamp", 3),
                    ],
                    z_feather: [
                        p.f("fZFeatherHeight"),
                        p.f("fZFeatherDist"),
                        p.f("fZFeatherFalloff"),
                        p.f("fZFeatherOffset"),
                    ],
                    use_global_lighting: p.b("bUseGlobalLighting"),
                    light: (p.v3("vLightPosition"), p.v("vLightColor", 0), p.f("fLightRadius"), p.f("fLightFalloff")),
                    modulo_position: p.b("bModuloPosition"),
                    dimensions: p.v3("vDimensions"),
                    stretch_per_camera: p.b("bStretchPerCamera"),
                    mesh_type: p.i("iType"),
                    extras,
                    registry: None,
                    vertices,
                    aabb,
                };
                if p.b("bFollowCamera") {
                    let n = p.i("iPeriodicBoxNum").max(1);
                    let half = (n - 1) as f32 * 0.5;
                    for (i, j, k) in (0..n).flat_map(|i| (0..n).flat_map(move |j| (0..n).map(move |k| (i, j, k)))) {
                        let cell = Vec3::new(i as f32, j as f32, k as f32) - Vec3::splat(half);
                        let t = ctx.camera + draw.dimensions * cell;
                        let world = draw.world * Mat4::from_translation(t);
                        let aabb = (draw.aabb.0 + t, draw.aabb.1 + t);
                        let mut copy = draw.clone();
                        (copy.world, copy.aabb) = (world, aabb);
                        out.push(copy);
                    }
                } else {
                    out.push(draw);
                }
            }
        }
    }
}
