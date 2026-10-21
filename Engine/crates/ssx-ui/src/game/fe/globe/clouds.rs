use ssx_formats::lynx::{Action, Effect, Value};
use ssx_formats::lynx_schema::{self, ParamDefault};

pub type V3 = [f32; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Rng([u32; 4]);

impl Default for Rng {
    fn default() -> Self {
        Rng([0x00F1_FBDC, 0x43BB_7EB2, 0x5CAD_6A98, 0x53DF_C8C1])
    }
}

impl Rng {
    pub fn next4(&mut self) -> [f32; 4] {
        let mut out = [0.0; 4];
        for (lane, o) in self.0.iter_mut().zip(&mut out) {
            let mut x = *lane;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *lane = x;
            *o = f32::from_bits(0x3F80_0000 | (x & 0x007F_FFFF)) - 1.0;
        }
        out
    }

    pub fn spread4(&mut self) -> [f32; 4] {
        self.next4().map(|v| v - 0.5)
    }
}

fn param(a: &Action, name: &str, index: u32) -> Option<Value> {
    if let Some(p) = a.params.iter().find(|p| p.name == name && p.index == index) {
        return Some(p.value.clone());
    }
    let d = lynx_schema::class(&a.class_name)?.param(name)?.default;
    Some(match d {
        ParamDefault::Bool(b) => Value::Bool(b),
        ParamDefault::Int(i) => Value::Int(i),
        ParamDefault::Float(f) => Value::Float(f),
        ParamDefault::Vector(v) => Value::Vector(v.to_vec()),
        ParamDefault::None => return None,
    })
}

fn f(a: &Action, name: &str) -> f32 {
    param(a, name, 0).and_then(|v| v.as_f32()).unwrap_or(0.0)
}

fn i(a: &Action, name: &str) -> i32 {
    match param(a, name, 0) {
        Some(Value::Int(x)) => x,
        Some(Value::Float(x)) => x as i32,
        Some(Value::Bool(b)) => i32::from(b),
        _ => 0,
    }
}

fn b(a: &Action, name: &str) -> bool {
    match param(a, name, 0) {
        Some(Value::Bool(x)) => x,
        Some(Value::Int(x)) => x != 0,
        _ => false,
    }
}

fn v4(a: &Action, name: &str, index: u32) -> [f32; 4] {
    match param(a, name, index).or_else(|| param(a, name, 0)) {
        Some(Value::Vector(v)) => std::array::from_fn(|k| v.get(k).copied().unwrap_or(if k == 3 { 1.0 } else { 0.0 })),
        _ => [0.0; 4],
    }
}

fn v3(a: &Action, name: &str) -> V3 {
    let v = v4(a, name, 0);
    [v[0], v[1], v[2]]
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Particle {
    pub pos: V3,
    pub size: f32,
    pub ori: V3,
    pub birth: f32,
    pub vel: V3,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CloudRender {
    pub age_max: i32,
    pub color_mult: [f32; 4],
    pub color_mult_spread: [f32; 4],
    pub color_ramp: [[f32; 4]; 4],
    pub color_ramp_times: [f32; 4],
    pub size_ramp: [f32; 4],
    pub size_ramp_times: [f32; 4],
    pub use_color_ramp: bool,
    pub use_color_mult: bool,
    pub use_size_ramp: bool,
    pub use_anim_texture: bool,
    pub anim_mode: i32,
    pub anim_count: i32,
    pub anim_rows: i32,
    pub anim_speed: f32,
    pub anim_random_start: i32,
    pub near_fade: [f32; 2],
    pub z_feather: [f32; 2],
    pub z_bias: f32,
    pub pivot_shift: [f32; 4],
    pub texture: String,
    pub technique: String,
}

#[derive(Clone, Debug)]
pub struct CloudEffect {
    pub name: String,
    emit: Action,
    kill: Action,
    pub render: CloudRender,
    capacity: usize,
    time_started_ms: i32,
    pub particles: Vec<Particle>,
}

impl CloudEffect {
    pub fn new(e: &Effect) -> Option<Self> {
        let g = e.groups.first()?;
        let find = |c: &str| g.actions.iter().find(|a| a.class_name == c).cloned();
        let (emit, kill, render) =
            (find("ParticleActionEmitBox")?, find("ParticleActionKillAge")?, find("ParticleActionRender")?);
        let capacity =
            g.params.iter().find(|p| p.name == "iNumParticlesMax").and_then(|p| p.value.as_i32()).unwrap_or(256).max(0)
                as usize;
        let s = |n: &str| match param(&render, n, 0) {
            Some(Value::String(s)) => s,
            _ => String::new(),
        };
        let r = CloudRender {
            age_max: i(&kill, "iAgeMs"),
            color_mult: v4(&render, "vColorMult", 0),
            color_mult_spread: v4(&render, "vColorMultSpread", 0),
            color_ramp: std::array::from_fn(|k| v4(&render, "vColorRamp", k as u32)),
            color_ramp_times: v4(&render, "vColorRampTimes", 0),
            size_ramp: v4(&render, "vSizeRamp", 0),
            size_ramp_times: v4(&render, "vSizeRampTimes", 0),
            use_color_ramp: b(&render, "bUseColorRamp"),
            use_color_mult: b(&render, "bUseColorMult"),
            use_size_ramp: b(&render, "bUseSizeRamp"),
            use_anim_texture: b(&render, "bUseAnimTexture"),
            anim_mode: i(&render, "iAnimTextureMode"),
            anim_count: i(&render, "iAnimTextureCount"),
            anim_rows: i(&render, "iAnimTextureRows"),
            anim_speed: f(&render, "fAnimTextureSpeed"),
            anim_random_start: i(&render, "iAnimTextureRandomStart"),
            near_fade: [f(&render, "fNearFadeStart"), f(&render, "fNearFadeEnd")],
            z_feather: [f(&render, "fZFeatherFalloff"), f(&render, "fZFeatherOffset")],
            z_bias: f(&render, "fZBias"),
            pivot_shift: v4(&render, "vPivotShift", 0),
            texture: s("sTexture"),
            technique: s("sTechnique"),
        };
        Some(CloudEffect {
            name: e.name.clone(),
            emit,
            kill,
            render: r,
            capacity,
            time_started_ms: 0,
            particles: Vec::new(),
        })
    }

    fn emit_count(&mut self, ctx: &Clock, rng: &mut Rng) -> usize {
        let e = &self.emit;
        let free = self.capacity.saturating_sub(self.particles.len());
        if self.time_started_ms == 0 {
            self.time_started_ms = ctx.time_ms;
        }
        let (rate, dur, cycles) = (i(e, "iEmitRate") as i64, i(e, "iEmitDurationMs") as i64, i(e, "iEmitCycles"));
        if cycles == 1 && dur > 0 && dur < 16 {
            return 0;
        }
        if rate == 0 || free == 0 || self.time_started_ms < 0 || ctx.dt == 0.0 {
            return 0;
        }
        let r = rng.next4()[0];
        let elapsed = ctx.time_ms - self.time_started_ms;
        let delay = (i(e, "iEmitDelayVarianceMs") as f32 * r + 0.5) as i32 + i(e, "iEmitDelayMs");
        let dur = dur as i32;
        if dur > 0 {
            if cycles != 0 {
                let period = dur + delay;
                if period == 0 {
                    return 0;
                }
                let (cyc, rem) = (elapsed / period, elapsed % period);
                if (cycles > 0 && cyc > cycles) || rem < delay {
                    return 0;
                }
            } else if elapsed > dur || delay > elapsed {
                return 0;
            }
        } else if delay > elapsed {
            return 0;
        }
        let per_frame = b(e, "bEmitRatePerFrame") || b(e, "bInject");
        let n_f = if per_frame { rate as f32 } else { rate as f32 * ctx.dt_smoothed };
        let n = n_f as i64;
        let frac = n_f - n as f32;
        let iv = if frac == 0.0 { 1000 } else { ((1.0 / frac + 0.5) as i64).clamp(1, 1000) };
        let extra = (ctx.frame as i64 % iv == 0 && !per_frame) as i64;
        (n + extra).clamp(0, free as i64) as usize
    }

    // TODO(guess): the particle nodes' locator (`WorldLocation` 0x8295B1A0, type 0) is the identity at rest (the box emits in camera space as authored); settle: the transform 0x8295B1A0 hands the globe entity's effect instance.
    fn emit_box(&mut self, n: usize, ctx: &Clock, rng: &mut Rng) {
        let e = &self.emit;
        let _start = rng.next4()[0];
        let (size_mean, dep) = (f(e, "fSizeMean"), f(e, "fMassDependenceOnSize"));
        let dt_ms = ctx.dt * 1000.0;
        let (vm, vs) = (v3(e, "vVelocityMean"), v3(e, "vVelocitySpread"));
        let (cm, dims) = (v3(e, "vCenter"), v3(e, "vDimensions"));
        let (om, os) = (v3(e, "vOrientationMean"), v3(e, "vOrientationSpread"));
        for k in 0..n {
            let u = k as f32 * (1.0 / n as f32);
            let (s1, s2, s3, _s4) = (rng.spread4(), rng.spread4(), rng.spread4(), rng.spread4());
            let (s5, _s6) = (rng.spread4()[0], rng.spread4()[0]);
            let vel: V3 = std::array::from_fn(|j| vm[j] + vs[j] * s1[j]);
            let pos: V3 = std::array::from_fn(|j| cm[j] + dims[j] * s2[j]);
            let ori: V3 = std::array::from_fn(|j| om[j] + os[j] * s3[j]);
            let size = size_mean + f(e, "fSizeSpread") * s5;
            let _ = dep;
            let r7 = rng.next4()[0];
            let birth = ctx.time_ms as f32 - i(e, "iAgeSpreadMs") as f32 * r7 - dt_ms * (n - 1 - k) as f32 / n as f32;
            let _ = (rng.spread4(), rng.spread4());
            let p: V3 = std::array::from_fn(|j| pos[j] - vel[j] * (u * ctx.dt));
            self.particles.push(Particle { pos: p, size, ori, birth, vel });
        }
    }

    fn kill_age(&mut self, time_ms: i32) {
        let (t, max) = (time_ms as f32, i(&self.kill, "iAgeMs") as f32);
        let ps = &mut self.particles;
        match i(&self.kill, "iOrdering") {
            1 => {
                let n = ps.iter().take_while(|q| t - q.birth > max || t - q.birth < 0.0).count();
                ps.drain(..n);
            }
            2 => {
                let n = ps.iter().filter(|q| t - q.birth > max || t - q.birth < 0.0).count();
                ps.drain(..n.min(ps.len()));
            }
            _ => {
                for k in (0..ps.len()).rev() {
                    let q = ps[k];
                    if !((t - q.birth) - max < 0.0 && (q.birth - t) - 0.1 < 0.0) {
                        ps.swap_remove(k);
                    }
                }
            }
        }
    }

    fn update(&mut self, ctx: &Clock, rng: &mut Rng) {
        let n = self.emit_count(ctx, rng);
        self.emit_box(n, ctx, rng);
        self.kill_age(ctx.time_ms);
        for p in &mut self.particles {
            for j in 0..3 {
                p.pos[j] += p.vel[j] * ctx.dt;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub clock: f64,
    pub time_ms: i32,
    pub dt: f32,
    pub frame: u32,
    pub dt_smoothed: f32,
}

impl Default for Clock {
    fn default() -> Self {
        Clock { clock: 0.0, time_ms: 0, dt: 0.0, frame: 0, dt_smoothed: 1.0 / 60.0 }
    }
}

#[derive(Clone, Debug, Default)]
pub struct CloudTransition {
    pub effects: Vec<CloudEffect>,
    pub rng: Rng,
    pub clock: Clock,
}

impl CloudTransition {
    pub fn new(effects: Vec<CloudEffect>) -> Self {
        CloudTransition { effects, ..Default::default() }
    }

    pub fn update(&mut self, dt: f32) {
        let c = &mut self.clock;
        let prev = c.clock;
        c.clock += f64::from(dt);
        c.dt = ((c.clock - prev) as f32).clamp(-0.1, 0.1);
        c.time_ms = (c.clock * 1000.0) as i32;
        c.frame = c.frame.wrapping_add(1);
        c.dt_smoothed = 0.5 * c.dt_smoothed + 0.5 * c.dt;
        let ctx = *c;
        for e in &mut self.effects {
            e.update(&ctx, &mut self.rng);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CloudDraw {
    pub render: CloudRender,
    pub particles: Vec<Particle>,
    pub time_ms: i32,
    pub transparency: f32,
}

impl CloudTransition {
    pub fn draws(&self, transparency: [f32; 2]) -> Vec<CloudDraw> {
        let mut out: Vec<CloudDraw> = self
            .effects
            .iter()
            .filter(|e| !e.particles.is_empty())
            .map(|e| {
                let t = if e.render.technique.ends_with("TransOut") { transparency[1] } else { transparency[0] };
                let mut particles = e.particles.clone();
                particles.sort_by(|a, b| a.pos[2].total_cmp(&b.pos[2]));
                CloudDraw { render: e.render.clone(), particles, time_ms: self.clock.time_ms, transparency: t }
            })
            .collect();
        let far = |d: &CloudDraw| d.particles.first().map_or(0.0, |p| p.pos[2]);
        out.sort_by(|a, b| far(a).total_cmp(&far(b)));
        out
    }
}
