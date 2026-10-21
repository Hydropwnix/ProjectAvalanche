struct Midground {
    world: array<vec4<f32>, 4>,
    view: array<vec4<f32>, 4>,
    proj: array<vec4<f32>, 4>,
    hl_xform: array<vec4<f32>, 4>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
    light_colour: vec4<f32>,
    light_view_offset: vec4<f32>,
    bounce_view: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    ramp: vec4<f32>,
    misc: vec4<f32>,
    beta_r: vec4<f32>,
    beta_dash_r: vec4<f32>,
    beta_m: vec4<f32>,
    beta_dash_m: vec4<f32>,
    heights: vec4<f32>,
    multipliers: vec4<f32>,
    tiles: vec4<f32>,
    ao_tile: vec4<f32>,
    highlight: vec4<f32>,
    peak_colour: vec4<f32>,
    peak: vec4<f32>,
    peak2: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: Midground;
@group(0) @binding(1) var normal_tex: texture_2d<f32>;
@group(0) @binding(2) var ao_tex: texture_2d<f32>;
@group(0) @binding(3) var mask_tex: texture_2d<f32>;
@group(0) @binding(4) var red_tex: texture_2d<f32>;
@group(0) @binding(5) var green_tex: texture_2d<f32>;
@group(0) @binding(6) var base_tex: texture_2d<f32>;
@group(0) @binding(7) var highlight_tex: texture_2d<f32>;
@group(0) @binding(8) var wrap_trilinear: sampler;
@group(0) @binding(9) var border_trilinear: sampler;

fn fetch(t: texture_2d<f32>, s: sampler, uv: vec2<f32>) -> vec4<f32> {
    if g.ao_tile.z > 0.5 {
        return textureSampleLevel(t, s, uv, 0.0);
    }
    return textureSample(t, s, uv);
}

fn xnormalize(v: vec3<f32>) -> vec3<f32> {
    let r = inverseSqrt(abs(dot(v, v)));
    return select(v * r, vec3<f32>(0.0), v == vec3<f32>(0.0));
}

fn row_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec4<f32> {
    return p.x * m[0] + p.y * m[1] + p.z * m[2] + m[3];
}

fn rot_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec3<f32> {
    return p.x * m[0].xyz + p.y * m[1].xyz + p.z * m[2].xyz;
}

struct MgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) o0: vec4<f32>,
    @location(1) o1: vec4<f32>,
    @location(2) o2: vec3<f32>,
    @location(3) o3: vec3<f32>,
    @location(4) o4: vec3<f32>,
    @location(5) o5: vec3<f32>,
}

@vertex
fn mg_vs(
    @location(0) p: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) t: vec3<f32>,
    @location(3) uv: vec2<f32>,
) -> MgOut {
    var o: MgOut;
    let w = row_mul(p, g.world).xyz;
    let v = row_mul(w, g.view).xyz;
    o.pos = row_mul(v, g.proj);
    o.o0 = vec4<f32>(rot_mul(n, g.world), uv.x);
    o.o1 = vec4<f32>(rot_mul(t, g.world), uv.y);
    o.o2 = v;
    o.o5 = w;
    let d = w - g.camera_pos.xyz;
    let s = sqrt(abs(dot(d, d)));
    let vw = xnormalize(d);
    let cw = saturate(dot(vw, g.light_dir.xyz));
    let rp = g.beta_dash_r.w;
    let hy = w.y - 1200.0 + rp;
    let h = sqrt(abs(w.x * w.x + w.z * w.z + hy * hy)) - rp;
    let rho_r = exp2((g.heights.y - h) * (g.heights.x * 1.442695));
    let rho_m = exp2((g.heights.w - h) * (g.heights.z * 1.442695));
    let sigma = rho_r * g.beta_r.xyz + rho_m * g.beta_m.xyz;
    let tr = exp2(-(s * 1.442695) * sigma);
    o.o3 = tr * g.multipliers.x;
    let gg = g.beta_r.w;
    let mie = exp2(log2(abs(1.0 + gg * gg - 2.0 * gg * cw)) * 1.5);
    let bd = (1.0 + cw * cw) * g.beta_dash_r.xyz + g.beta_dash_m.xyz * (1.0 - gg * gg) / mie;
    let fr = bd / (g.beta_r.xyz + g.beta_m.xyz);
    o.o4 = fr * (1.0 - tr) * g.multipliers.y * g.light_colour.xyz;
    return o;
}

@fragment
fn mg_fs(in: MgOut) -> @location(0) vec4<f32> {
    let u = in.o0.w;
    let v = in.o1.w;
    let pw = in.o5;
    let bt = g.tiles.x;
    let dh = pw - g.highlight.xyz;
    let cov = g.peak.x;
    let ux = cov * (dh.x * g.hl_xform[0].x + dh.y * g.hl_xform[1].x + dh.z * g.hl_xform[2].x) + g.hl_xform[3].x;
    let uz = cov * (dh.x * g.hl_xform[0].z + dh.y * g.hl_xform[1].z + dh.z * g.hl_xform[2].z) + g.hl_xform[3].z;
    let hl_uv = vec2<f32>(ux, uz) * 0.5 + 0.5;
    let time = g.misc.w;
    let line_phase = fract(g.peak.y * (dh.y * 0.15915494) + 0.5) * 6.2831855 - 3.1415927;
    let pulse_phase = fract(g.peak2.y * (time * 0.15915494) + 0.5) * 6.2831855 - 3.1415927;
    let nn = xnormalize(in.o0.xyz);
    let wts = max(abs(nn) * 7.0 - 1.4, vec3<f32>(0.0));
    let wn = wts / dot(wts, vec3<f32>(1.0));
    let red = fetch(red_tex, wrap_trilinear, vec2<f32>(u, v) * g.tiles.y).rgb;
    let green = fetch(green_tex, wrap_trilinear, vec2<f32>(u, v) * g.tiles.z).rgb;
    let ao = fetch(ao_tex, wrap_trilinear, vec2<f32>(u * g.ao_tile.x, v * g.ao_tile.y)).g;
    let b1 = fetch(base_tex, wrap_trilinear, vec2<f32>(pw.z * bt, -pw.y * bt)).rgb;
    let b2 = fetch(base_tex, wrap_trilinear, vec2<f32>(pw.x * bt, pw.z * bt)).rgb;
    let b3 = fetch(base_tex, wrap_trilinear, vec2<f32>(pw.x * bt, -pw.y * bt)).rgb;
    let nt_raw = fetch(normal_tex, wrap_trilinear, vec2<f32>(u, v)).xy;
    let hl = fetch(highlight_tex, border_trilinear, hl_uv).rgb;
    let m = fetch(mask_tex, wrap_trilinear, vec2<f32>(u, v)).rgb;
    let nt = normalize(vec3<f32>(nt_raw * 2.0 - 1.0, 1.0));
    let tt = xnormalize(in.o1.xyz);
    let bb_ = cross(nn, tt);
    let np = nt.x * tt + nt.y * bb_ + nt.z * nn;
    let npn = xnormalize(np);
    let nv0 = rot_mul(npn, g.view);
    let nv = xnormalize(nv0);
    let vdir = xnormalize(-in.o2);
    let ndl = saturate(dot(nv, g.light_view_offset.xyz));
    let rim = saturate(exp2(log2(abs(1.0 - abs(dot(nv, vdir)))) * g.ramp.y));
    let dd = ndl + (rim - ndl) * (ndl * (g.ramp.z - g.ramp.w) + g.ramp.w);
    let bnc = saturate((dot(nv, g.bounce_view.xyz) - g.misc.y) / (1.0 - g.misc.y));
    let diffuse = mix(g.ramp1.xyz, g.ramp0.xyz, dd) * g.ramp.x + g.misc.x * bnc * bnc * (3.0 - 2.0 * bnc) * g.ramp2.xyz;
    let ao_tint = (ao * ao) * (1.0 - g.ramp1.xyz) + g.ramp1.xyz;
    let base = wn.x * b1 + wn.y * b2 + wn.z * b3;
    var alb = mix(base * base, green * green, m.g);
    alb = mix(alb, vec3<f32>(0.5), saturate(g.tiles.w + m.b));
    alb = mix(alb, red * red, m.r);
    let c = diffuse * ao_tint * alb;
    let edge1 = saturate(1.0 - g.peak.z);
    let edge0 = saturate(edge1 - g.peak.w);
    let s_line = sin(line_phase) * 0.5 + 0.5;
    let lt = saturate((s_line - edge0) / (edge1 - edge0));
    let lines = g.peak2.x * lt * lt * (3.0 - 2.0 * lt);
    let pulse = 0.5 * g.peak2.z * sin(pulse_phase) + 1.0 - 0.5 * g.peak2.z;
    let hc = hl * g.peak_colour.xyz * g.peak_colour.w * (1.0 - g.peak2.x + lines) * pulse * (1.0 - g.highlight.w);
    let luma = dot(hc, vec3<f32>(0.299, 0.587, 0.114));
    let lit = c * (1.0 - luma) + hc * luma;
    let col = in.o3 * g.misc.z * lit + in.o4;
    return vec4<f32>(sqrt(abs(col)), 1.0);
}
