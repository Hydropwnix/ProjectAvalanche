struct Globe {
    w_inv: array<vec4<f32>, 4>,
    world_view: array<vec4<f32>, 4>,
    proj: array<vec4<f32>, 4>,
    view_inv: array<vec4<f32>, 4>,
    camera_pos: vec4<f32>,
    project_cam: vec4<f32>,
    globe_r: vec4<f32>,
    atmos_r: vec4<f32>,
    light_dir: vec4<f32>,
    light_colour: vec4<f32>,
    light_view: vec4<f32>,
    bounce_view: vec4<f32>,
    highlight_view: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    spec_colour: vec4<f32>,
    ramp: vec4<f32>,
    misc: vec4<f32>,
    surface: vec4<f32>,
    cloud: vec4<f32>,
    atmos: vec4<f32>,
    scatter_inv_wl: vec4<f32>,
    scatter_keys: vec4<f32>,
    scatter_scales: vec4<f32>,
    samples: vec4<f32>,
    highlight_colour: vec4<f32>,
    highlight: vec4<f32>,
    highlight2: vec4<f32>,
    halo: vec4<f32>,
    debug: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: Globe;
@group(0) @binding(1) var diffuse_tex: texture_2d<f32>;
@group(0) @binding(2) var relief_tex: texture_2d<f32>;
@group(0) @binding(3) var frame_tex: texture_2d<f32>;
@group(0) @binding(4) var wrap_linear: sampler;
@group(0) @binding(5) var clamp_point: sampler;

fn row_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec4<f32> {
    return p.x * m[0] + p.y * m[1] + p.z * m[2] + m[3];
}

fn rot_mul(p: vec3<f32>, m: array<vec4<f32>, 4>) -> vec3<f32> {
    return p.x * m[0].xyz + p.y * m[1].xyz + p.z * m[2].xyz;
}

// TODO(guess): the game projects `view / globeRadiusParameters.x` (same screen position, depth R× nearer) and the FE camera's planes are not traced; the port has no depth test here, so clip z is the middle of the range; settle: the FE render camera's projection (GameCameraManager 0x82EE7710 for the front end).
fn project(look: vec3<f32>) -> vec4<f32> {
    let local = row_mul(look, g.w_inv).xyz;
    let view = row_mul(local, g.world_view).xyz;
    let c = row_mul(view / g.globe_r.x, g.proj);
    return vec4<f32>(c.x, c.y, 0.5 * c.w, c.w);
}

struct EarthOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) plane: vec3<f32>,
}

@vertex
fn earth_vs(@location(0) p: vec3<f32>) -> EarthOut {
    let plane = p * 0.001 * g.atmos_r.y;
    var o: EarthOut;
    o.pos = project(plane);
    o.plane = plane;
    return o;
}

fn relief(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(relief_tex, wrap_linear, uv, 0.0);
}

fn scale_fn(cos_a: f32) -> f32 {
    let x = 1.0 - cos_a;
    let p = x * (x * (x * (-cos_a * 5.25 + -1.5500002) + 3.83) + 0.459);
    return exp2(p * 1.442695 + -0.0041405344) * g.scatter_scales.y;
}

@fragment
fn earth_fs(in: EarthOut) -> @location(0) vec4<f32> {
    let r0 = in.plane;
    let rho2 = r0.x * r0.x + r0.y * r0.y;
    if !(g.atmos_r.w > rho2) {
        discard;
    }
    let outside = rho2 > g.globe_r.w;
    let rx2 = select(g.globe_r.z, g.atmos_r.z, outside);
    let rx = select(g.globe_r.x, g.atmos_r.x, outside);
    let d = r0 - g.project_cam.xyz;
    let d2 = r0.y * r0.y + g.project_cam.w + r0.x * r0.x;
    let inv_d = inverseSqrt(abs(d2));
    let l_over_d = g.project_cam.z * inv_d;
    let b2 = (l_over_d * rho2) * l_over_d;
    let tc = sqrt(abs(d2)) - sqrt(abs(rho2 - b2));
    let half_chord = sqrt(abs(rx2 - b2));
    let dir = normalize_xenos(d);
    let t = tc - half_chord;
    let hit = dir * t + g.project_cam.xyz;
    let n_l = rot_mul(hit, g.w_inv);
    let sin_lon_part = rx2 - n_l.y * n_l.y;
    let sgn_z = select(0.0, 1.0, n_l.z > -n_l.z) - select(0.0, 1.0, -n_l.z > n_l.z);
    let inv_len = inverseSqrt(abs(dot(n_l, n_l)));
    let view_pos = row_mul(n_l, g.world_view).xyz;
    let nx = n_l.x * inv_len;
    let ny = n_l.y * inv_len;
    let nz = n_l.z * inv_len;
    let ny_r = n_l.y * (1.0 / rx);
    let cos_lon = n_l.x * inverseSqrt(abs(sin_lon_part));
    let tv = nz * g.world_view[0].xyz - nx * g.world_view[2].xyz;
    let nv = nx * g.world_view[0].xyz + ny * g.world_view[1].xyz + nz * g.world_view[2].xyz;
    let bv = cross(nv, tv);
    let ax = vec2<f32>(abs(ny_r), abs(cos_lon));
    var poly = ax * -0.0187293 + 0.074261;
    poly = ax * poly + -0.2121144;
    poly = ax * poly + 1.5707288;
    let root = sqrt(abs(1.0 - ax));
    let neg = vec2<f32>(select(0.0, 1.0, -ny_r > ny_r), select(0.0, 1.0, -cos_lon > cos_lon));
    let acos_abs = poly * root;
    let fix = (root * -2.0 * poly + 3.1415927) * neg;
    let ang = acos_abs + fix;
    let lat = 1.5707964 - ang.x;
    let lon = sgn_z * ang.y;
    var uv = vec2<f32>(-lon * 0.15915494 + 0.75, -lat * 0.31830987 + 0.5);
    let e_t = vec3<f32>(dot(nv, view_pos), dot(tv, view_pos), dot(bv, view_pos));
    let e_n = normalize_xenos(e_t);
    let inv_n = 1.0 / abs(e_n.x);
    let step_t = e_n.y * g.surface.z * inv_n;
    let step_b = e_n.z * g.surface.z * inv_n;
    let ray_ratio = sqrt(abs(step_t * step_t + step_b * step_b));
    var depth = 1.0;
    var ruv = uv;
    let cone_steps = i32(g.samples.z);
    for (var i = 0; i < cone_steps; i++) {
        let h = relief(ruv);
        let cone = h.r;
        let s = cone / (ray_ratio + cone) * saturate(depth - h.g);
        depth = depth - s;
        ruv = ruv + vec2<f32>(s * step_t, -step_b * s);
    }
    var half_step = 0.5 - depth * 0.5;
    var rstep = vec2<f32>(step_t, -step_b) * half_step;
    var rdepth = 1.0 - half_step;
    var buv = uv + rstep;
    var h_last = relief(buv).g;
    let bin_steps = i32(g.samples.w);
    for (var i = 0; i < bin_steps; i++) {
        let above = rdepth >= h_last;
        half_step = half_step * 0.5;
        rstep = rstep * 0.5;
        if above {
            rdepth = rdepth - half_step;
            buv = buv + rstep;
        } else {
            rdepth = rdepth + half_step;
            buv = buv - rstep;
        }
        h_last = relief(buv).g;
    }
    uv = buv;
    let world_pos = row_mul(view_pos, g.view_inv).xyz;
    let to_pos = world_pos - g.camera_pos.xyz;
    let to_pos2 = dot(to_pos, to_pos);
    let vw = normalize_xenos(to_pos);
    let cl = normalize_xenos(g.light_view.xyz * g.cloud.z + view_pos);
    let shadow_dir = cl.x * tv + cl.y * bv + cl.z * nv;
    let cam_ray = dot(vw, g.camera_pos.xyz);
    let miss2 = -cam_ray * cam_ray + g.project_cam.w;
    let disc = max(g.atmos_r.z - miss2, 0.0);
    let shadow_off = clamp(vec2<f32>(shadow_dir.x, -shadow_dir.y), vec2<f32>(-0.3), vec2<f32>(0.3)) * g.cloud.w;
    let cloud_uv = vec2<f32>(fract(-g.misc.w * g.cloud.x + uv.x), uv.y);
    let h_up_r = relief(vec2<f32>(uv.x + 0.00048828125, uv.y + 0.0009765625)).g;
    let h_dn_r = relief(vec2<f32>(uv.x + 0.00048828125, uv.y - 0.0009765625)).g;
    let h_dn_l = relief(vec2<f32>(uv.x - 0.00048828125, uv.y - 0.0009765625)).g;
    let h_up_l = relief(vec2<f32>(uv.x - 0.00048828125, uv.y + 0.0009765625)).g;
    let albedo_tex = textureSampleLevel(diffuse_tex, wrap_linear, uv, 0.0).rgb;
    let cloud = relief(cloud_uv).b;
    let cloud_shadow = relief(cloud_uv + shadow_off).b;
    let sea = saturate(h_last * g.atmos.y + cloud);
    let cloud_cover = saturate(-cloud * g.cloud.y + 1.0);
    let albedo = albedo_tex * albedo_tex + cloud * g.cloud.y;
    let lv = normalize_xenos(g.light_view.xyz);
    let n_dot_l_geo = dot(lv, nv);
    let eye = normalize_xenos(-view_pos);
    let n_dot_e = dot(eye, nv);
    let hs = vec4<f32>(h_up_r, h_dn_r, h_dn_l, h_up_l) * g.surface.w;
    let dy = hs.x - hs.z;
    let dz = hs.y - hs.w;
    let gx = dy * 0.0009765625 - dz * 0.0009765625;
    let gy = dz * -0.001953125 - dy * 0.001953125;
    let g2 = dot(vec2<f32>(gy, gx), vec2<f32>(gy, gx)) + 1.4551915e-11;
    let ig = inverseSqrt(abs(g2));
    let shadow = g.atmos.x * cloud_shadow * cloud_cover;
    let bump = (3.8146973e-6 * ig) * nv + (gy * ig) * tv + (gx * ig) * bv;
    let refl = lv - 2.0 * n_dot_l_geo * nv;
    let bn = normalize_xenos(bump);
    let spec_base = saturate(dot(-eye, refl));
    let spec = exp2(log2(spec_base) * g.surface.x);
    let rim_shadow_span = g.ramp.z - g.ramp.w;
    let n_dot_l = saturate(dot(nv, lv));
    let bn_dot_l = saturate(dot(bn, lv));
    let spread_scale = 1.0 / (1.0 - g.misc.y);
    let bounce_b = saturate((dot(bn, g.bounce_view.xyz) - g.misc.y) * spread_scale);
    let bounce_n = saturate((dot(nv, g.bounce_view.xyz) - g.misc.y) * spread_scale);
    let rim_b = saturate(exp2(g.ramp.y * log2(abs(1.0 - abs(dot(bn, eye))))));
    let rim_n = saturate(exp2(g.ramp.y * log2(abs(1.0 - abs(n_dot_e)))));
    let fresnel = (1.0 - g.surface.y) * (1.0 - abs(-n_dot_e)) + g.surface.y;
    let k_b = bn_dot_l + (rim_b - bn_dot_l) * (bn_dot_l * rim_shadow_span + g.ramp.w);
    let k_n = n_dot_l + (rim_n - n_dot_l) * (n_dot_l * rim_shadow_span + g.ramp.w);
    let bounce_col_b = (bounce_b * bounce_b * g.misc.x) * (3.0 - 2.0 * bounce_b) * g.ramp2.rgb;
    let bounce_col_n = (bounce_n * bounce_n * g.misc.x) * (3.0 - 2.0 * bounce_n) * g.ramp2.rgb;
    let light_b = mix(g.ramp1.rgb, g.ramp0.rgb, k_b) * g.ramp.x + bounce_col_b;
    let light_n = mix(g.ramp1.rgb, g.ramp0.rgb, k_n) * g.ramp.x + bounce_col_n;
    let lit = light_b * light_n * saturate(albedo - shadow);
    let spec_col = fresnel * (g.light_dir.w * g.light_colour.w) * spec * g.spec_colour.rgb;
    let surface = spec_col * (1.0 - sea) + lit;
    let pos_dist = sqrt(abs(to_pos2));
    let chord = sqrt(abs(disc));
    let enter = cam_ray - chord;
    let ray_end = select(pos_dist, chord * 2.0 + pos_dist, outside);
    let ray_start = select(enter, pos_dist, outside);
    let sample_len = (ray_end - ray_start) / g.samples.x;
    let step = vw * sample_len;
    let start = vw * ray_start + g.camera_pos.xyz;
    let scaled_len = g.scatter_scales.x / g.samples.x * (ray_end - ray_start);
    let kr4 = g.scatter_inv_wl.xyz * g.scatter_keys.z + g.scatter_keys.w;
    let n_samples = i32(g.samples.y);
    var colour: vec3<f32>;
    if outside {
        let start_angle = dot(vw, start) * (1.0 / g.atmos_r.x);
        let start_offset = scale_fn(start_angle) * exp2(-1.442695 * (1.0 / g.scatter_scales.y));
        var p = start + step * 0.5;
        var acc = vec3<f32>(0.0);
        for (var i = 0; i < n_samples; i++) {
            let h = sqrt(abs(dot(p, p)));
            let ih = inverseSqrt(abs(dot(p, p)));
            let light_angle = dot(p, g.light_dir.xyz) * ih;
            let camera_angle = dot(vw, p) * ih;
            let fdepth = exp2((g.globe_r.x - h) * (1.442695 * g.scatter_scales.z));
            let scatter = (scale_fn(light_angle) - scale_fn(camera_angle)) * fdepth + start_offset;
            let atten = exp2(-scatter * kr4 * 1.442695);
            p = p + step;
            acc = acc + atten * (fdepth * scaled_len);
        }
        let ray_cos = dot(vw, g.light_dir.xyz) * (1.0 / ray_start);
        let phase = ray_cos * ray_cos * 0.75 + 0.75;
        let rayleigh = (g.scatter_inv_wl.xyz * g.scatter_keys.x + g.scatter_keys.y) * acc;
        let mie = acc * g.scatter_keys.y;
        colour = (phase * rayleigh + mie * 0.25) * g.atmos.z;
    } else {
        let ip = 1.0 / g.globe_r.x;
        let camera_angle = dot(-vw, world_pos) * ip;
        let light_angle = dot(world_pos, g.light_dir.xyz) * ip;
        let fdepth0 = exp2((1.442695 * (1.0 / g.scatter_scales.y)) * (g.globe_r.x - g.atmos_r.x));
        let cam_scale = scale_fn(camera_angle);
        let ftemp = scale_fn(light_angle) + cam_scale;
        let cam_offset = cam_scale * fdepth0;
        var p = start + step * 0.5;
        var acc = vec3<f32>(0.0);
        var atten = vec3<f32>(0.0);
        for (var i = 0; i < n_samples; i++) {
            let h = sqrt(abs(dot(p, p)));
            let fdepth = exp2((g.scatter_scales.z * 1.442695) * (g.globe_r.x - h));
            let scatter = fdepth * ftemp - cam_offset;
            atten = exp2(-scatter * kr4 * 1.442695);
            acc = acc + atten * (scaled_len * fdepth);
            p = p + step;
        }
        let inscatter = (g.scatter_inv_wl.xyz * g.scatter_keys.x + g.scatter_keys.y) * acc;
        colour = (inscatter + (atten * 0.25 + g.ramp2.rgb)) * (surface * g.atmos.w);
    }
    let clip = view_pos.x * g.proj[0] + view_pos.y * g.proj[1] + view_pos.z * g.proj[2] + g.proj[3];
    let iw = 1.0 / clip.w;
    let screen = vec2<f32>(0.5 * clip.x * iw + 0.5, -0.5 * clip.y * iw + 0.5);
    let fb = textureSampleLevel(frame_tex, clamp_point, screen, 0.0).rgb;
    let behind = select(vec3<f32>(0.0), fb * fb, outside);
    let hl_dot = saturate(dot(nv, g.highlight_view.xyz));
    let phase_arg = fract((g.highlight.x * hl_dot + g.highlight.w * g.misc.w) * 0.15915494 + 0.5) * 6.2831855 - 3.1415927;
    let ripple = saturate(sin(phase_arg) * 0.5 + 0.5);
    let core = saturate(1.0 - g.highlight2.x);
    let e0 = saturate(core - g.highlight2.y);
    let line = saturate(1.0 - g.highlight.y);
    let e1 = saturate(line - g.highlight.z);
    let s0 = saturate((hl_dot - e0) / (core - e0));
    let s1 = saturate((ripple - e1) / (line - e1));
    let mask = s1 * s1 * (3.0 - 2.0 * s1) * (s0 * s0) * (3.0 - 2.0 * s0);
    let hl = g.highlight_colour.rgb * g.highlight_colour.w * mask;
    let hl_lum = dot(hl, vec3<f32>(0.299, 0.587, 0.114));
    var c = colour;
    if !outside {
        c = colour * (1.0 - hl_lum) + hl * hl_lum;
    }
    let back = 1.0 - behind / g.misc.z;
    let out_c = (1.0 - (1.0 - c) * back) * g.misc.z;
    return vec4<f32>(sqrt(abs(out_c)), 1.0);
}

fn normalize_xenos(v: vec3<f32>) -> vec3<f32> {
    let r = inverseSqrt(abs(dot(v, v)));
    return select(v * r, vec3<f32>(0.0), v == vec3<f32>(0.0));
}

struct HaloOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) clip: vec4<f32>,
}

@vertex
fn halo_vs(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> HaloOut {
    let plane = p * 0.001 * g.halo.w * g.atmos_r.y;
    let local = row_mul(plane, g.w_inv).xyz;
    let view = row_mul(local, g.world_view).xyz;
    let c = row_mul(view / g.globe_r.x, g.proj);
    var o: HaloOut;
    o.pos = vec4<f32>(c.x, c.y, 0.5 * c.w, c.w);
    o.uv = uv + g.halo.xy;
    o.clip = c;
    return o;
}

@fragment
fn halo_fs(in: HaloOut) -> @location(0) vec4<f32> {
    let iw = 1.0 / in.clip.w;
    let screen = vec2<f32>(in.clip.x * iw * 0.5 + 0.5, in.clip.y * iw * -0.5 + 0.5);
    let fb = textureSampleLevel(frame_tex, clamp_point, screen, 0.0).rgb;
    let tex = textureSampleLevel(diffuse_tex, wrap_linear, in.uv, 0.0).rgb;
    let back = 1.0 - (fb * fb) * (1.0 / g.misc.z);
    let front = 1.0 - (tex * tex) * g.halo.z;
    let c = (1.0 - front * back) * g.misc.z;
    return vec4<f32>(sqrt(abs(c)), 1.0);
}
