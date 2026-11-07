use super::*;

fn check(src: &str) -> Result<Accepted, Vec<Problem>> {
    check_hook(HookFamily::Object, src.as_bytes(), &Limits::default())
}

fn refused(src: &str, want: &str) {
    match check(src) {
        Ok(a) => panic!("accepted ({}), wanted a refusal with {want:?}:\n{src}", a.summary()),
        Err(p) => assert!(p.iter().any(|(_, _, m)| m.contains(want)), "{p:?} has no {want:?}\n{src}"),
    }
}

fn accepted(src: &str) -> Cost {
    match check(src) {
        Ok(a) => a.surface.expect("a surface hook"),
        Err(p) => panic!("refused: {p:?}\n{src}"),
    }
}

const HEAD: &str = "fn surface(input: SurfaceIn, base: Surface, diffuse: texture_2d<f32>, diffuse_sampler: sampler, \
                    normal_map: texture_2d<f32>, specular_map: texture_2d<f32>, map_sampler: sampler) -> Surface {";

fn surface(body: &str) -> String {
    format!("{HEAD}\n    var out = base;\n{body}\n    return out;\n}}\n")
}

fn with(helpers: &str, body: &str) -> String {
    format!("{helpers}\n{}", surface(body))
}

fn work(body: &str) -> u64 {
    accepted(&surface(body)).operations - accepted(&surface("")).operations
}

#[test]
fn reproducer_1_one_loop_per_counter() {
    refused(
        &surface(
            "    var i: i32 = 0;\n    for (; i < 2000000000; i += 10000000) { out.specular += 0.0; }\n    for (; i >= 0; i--) { out.specular += 0.000001; }",
        ),
        "counts 2 loops",
    );
    accepted(&surface(
        "    var i: i32 = 0;\n    for (; i < 8; i++) { out.specular += 0.1; }\n    var j: i32 = 8;\n    for (; j >= 0; j--) { out.specular += 0.1; }",
    ));
}

#[test]
fn reproducer_2_the_guard_is_evaluated_on_every_pass() {
    refused(
        &surface(
            "    var i: i32 = 0;\n    let c = i < 10;\n    loop { if c { } else { break; } out.specular += 0.000001; continuing { i = i + 1; } }",
        ),
        "condition on `i` is evaluated before the loop",
    );
    accepted(&surface(
        "    var i: i32 = 0;\n    loop { if i < 10 { } else { break; } out.specular += 0.000001; continuing { i = i + 1; } }",
    ));
    refused(
        &surface(
            "    var i: i32 = 0;\n    let v = i;\n    loop { if v < 10 { } else { break; } out.specular += 0.1; continuing { i = i + 1; } }",
        ),
        "condition on `i` is evaluated before the loop",
    );
}

#[test]
fn reproducer_3_the_step_is_evaluated_on_every_pass() {
    refused(
        &surface(
            "    var i: i32 = 0;\n    let s = i + 1;\n    loop { if i < 10 { } else { break; } out.specular += 0.000001; continuing { i = s; } }",
        ),
        "step of the loop counter `i` is evaluated before the loop",
    );
    accepted(&surface(
        "    var i: i32 = 0;\n    loop { if i < 10 { } else { break; } out.specular += 0.000001; continuing { let s = i + 1; i = s; } }",
    ));
}

#[test]
fn reproducer_4_memory_is_capped_and_moves_are_counted() {
    refused(
        &surface("    var a: array<vec4<f32>, 65536>;\n    out.specular += a[7].x;"),
        "over the cap of 2048 per value",
    );
    accepted(&surface("    var a: array<vec4<f32>, 16>;\n    a[3] = vec4(1.0);\n    out.specular += a[3].x;"));
    refused(&surface("    let z = array<vec4<f32>, 4096>();\n    out.specular += z[u32(input.time)].x;"), "per value");
    refused(
        &surface(
            "    var a: array<vec4<f32>, 32>;\n    var b: array<vec4<f32>, 32>;\n    for (var i = 0; i < 256; i++) { b = a; a = b; b = a; a = b; }\n    out.specular += a[0].x;",
        ),
        "scalar operations per invocation, over the cap",
    );
    let copy = work("    var a: array<vec4<f32>, 32>;\n    var b: array<vec4<f32>, 32>;\n    b = a;");
    assert!(copy >= 256, "{copy}");
    refused(
        &surface(
            "    var a: array<vec4<f32>, 64>;\n    var b: array<vec4<f32>, 64>;\n    b = a;\n    out.specular += b[0].x;",
        ),
        "over the cap of 2048 per hook",
    );
    refused(
        &with(
            "fn helper(k: i32) -> f32 { var t: array<vec4<f32>, 96>; t[k] = vec4(1.0); return t[0].x; }",
            "    var a: array<vec4<f32>, 64>;\n    out.specular += helper(1) + a[0].x;",
        ),
        "bytes of locals and arguments along its calls, over the cap of 2048",
    );
    accepted(&with(
        "fn helper(k: i32) -> f32 { var t: array<vec4<f32>, 32>; t[k] = vec4(1.0); return t[0].x; }",
        "    var a: array<vec4<f32>, 32>;\n    out.specular += helper(1) + a[0].x;",
    ));
}

#[test]
fn the_weights_follow_the_values() {
    let scalar = work("    out.specular += input.time;");
    let vector = work("    out.base_color += vec4(input.time);");
    assert!(vector > scalar, "{vector} vs {scalar}");
    let product = work(
        "    let m = mat4x4<f32>(vec4(1.0), vec4(2.0), vec4(3.0), vec4(input.time));\n    let p = m * m;\n    out.specular += p[0].x;",
    );
    assert!(product >= 64, "{product}");
    let read = accepted(&surface("    out.base_color = textureSample(diffuse, diffuse_sampler, input.uv);"));
    assert_eq!(read.texture_reads, 1);
}

#[test]
fn constant_bounds_steps_and_starts() {
    refused(
        &surface("    for (var i = 0; i < i32(input.time); i++) { out.specular += 0.1; }"),
        "bound is not a constant",
    );
    accepted(&with("const N: i32 = 12;", "    for (var i = 0; i < N; i++) { out.specular += 0.1; }"));
    refused(
        &surface("    var s = 2;\n    for (var i = 0; i < 8; i += s) { out.specular += 0.1; }"),
        "is not a constant (`i++`",
    );
    accepted(&surface("    for (var i = 0; i < 8; i += 2) { out.specular += 0.1; }"));
    refused(
        &surface("    var i = i32(input.time);\n    for (; i < 8; i++) { out.specular += 0.1; }"),
        "not a constant",
    );
    let both =
        work("    var i = 3;\n    if input.time > 1.0 { i = 7; }\n    for (; i < 10; i++) { out.specular += 0.1; }");
    let seven = work("    var i = 3;\n    for (; i < 10; i++) { out.specular += 0.1; }");
    assert!(both >= seven, "{both} vs {seven}");
}

#[test]
fn the_counter_is_written_by_its_increment_only() {
    refused(&surface("    for (var i = 0; i < 8; i++) { i = 2; }"), "changed in the loop's body");
    refused(
        &surface("    for (var i = 0; i < 8; i++) { if input.time > 1.0 { i -= 1; } }"),
        "changed in the loop's body",
    );
    accepted(&surface("    var k = 0;\n    for (var i = 0; i < 8; i++) { k = i * 2; out.specular += f32(k); }"));
    let bump = "fn bump(p: ptr<function, i32>) { *p = *p - 1; }";
    refused(&with(bump, "    for (var i = 0; i < 8; i++) { bump(&i); }"), "pointer to the loop counter");
    refused(
        &with(bump, "    var i = 0;\n    bump(&i);\n    for (; i < 8; i++) { out.specular += 0.1; }"),
        "pointer to the loop counter",
    );
    accepted(&with(bump, "    var k = 0;\n    for (var i = 0; i < 8; i++) { bump(&k); }\n    out.specular += f32(k);"));
}

#[test]
fn directions_wraps_and_the_trip_cap() {
    refused(&surface("    for (var i = 0; i < 8; i--) { out.specular += 0.1; }"), "away from its bound");
    accepted(&surface("    for (var i = 8; i > 0; i--) { out.specular += 0.1; }"));
    refused(&surface("    for (var i = 5u; i >= 0u; i--) { out.specular += 0.1; }"), "wrap");
    accepted(&surface("    for (var i = 5u; i > 0u; i--) { out.specular += 0.1; }"));
    refused(&surface("    for (var i = 0; i < 257; i++) { out.specular += 0.1; }"), "over the cap of 256 per loop");
    accepted(&surface("    for (var i = 0; i < 256; i++) { out.specular += 0.1; }"));
}

#[test]
fn loops_entered_again_by_a_loop_around_them() {
    let nest = work("    for (var i = 0; i < 8; i++) { for (var j = 0; j < 8; j++) { out.specular += 0.01; } }");
    let one = work("    for (var j = 0; j < 8; j++) { out.specular += 0.01; }");
    assert!(nest >= 8 * one, "{nest} vs {one}");
    accepted(&surface(
        "    var j = 0;\n    for (var i = 0; i < 8; i++) { for (; j < 8; j++) { out.specular += 0.01; } }",
    ));
    refused(
        &surface(
            "    var j = 0;\n    for (var i = 0; i < 8; i++) { j = i32(input.time); for (; j < 8; j++) { out.specular += 0.01; } }",
        ),
        "not a constant",
    );
    refused(
        &surface("    var i = 0;\n    for (; i < 4; i++) { for (; i < 8; i++) { out.specular += 0.1; } }"),
        "counts 2 loops",
    );
}

#[test]
fn early_exits_and_fall_through() {
    accepted(&surface("    for (var i = 0; i < 8; i++) { if input.time > 2.0 { break; } out.specular += 0.1; }"));
    accepted(&surface("    for (var i = 0; i < 8; i++) { if input.time > 2.0 { continue; } out.specular += 0.1; }"));
    accepted(&surface("    for (var i = 0; i < 8; i++) { if input.time > 2.0 { return out; } out.specular += 0.1; }"));
    let one = work("    for (var i = 0; i < 8; i++) { out.specular += 0.1; }");
    let cases = work(
        "    switch u32(input.time) {\n      case 0u: { for (var i = 0; i < 8; i++) { out.specular += 0.1; } }\n      case 1u: { for (var j = 0; j < 8; j++) { out.specular += 0.1; } }\n      default: { for (var k = 0; k < 8; k++) { out.specular += 0.1; } }\n    }",
    );
    assert!(cases >= 3 * one, "{cases} vs {one}");
}

#[test]
fn calls_carry_their_callee() {
    let helper = "fn lots() -> f32 { var s = 0.0; for (var i = 0; i < 64; i++) { s += 0.5; } return s; }";
    let called = accepted(&with(helper, "    out.specular += lots();")).operations;
    let bare = accepted(&surface("    out.specular += 1.0;")).operations;
    assert!(called >= bare + 64, "{called} vs {bare}");
    refused(&with("fn f(x: i32) -> i32 { return f(x - 1); }", "    out.specular = f32(f(3));"), "recursive");
}

fn composed(src: &str) -> Result<Composed, Vec<String>> {
    let module = naga::front::wgsl::parse_str(src).expect("parses");
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .expect("validates");
    check_composed(
        HookFamily::Object,
        &module,
        &info,
        &|n| n.strip_prefix("hook_").map(str::to_string),
        &Limits::default(),
    )
}

const FAMILY: &str =
    "@group(0) @binding(0) var<uniform> secret: vec4<f32>;\nfn family_help() -> f32 { return secret.x; }\n";

#[test]
fn the_composed_module_is_analysed() {
    let ok = composed(&format!(
        "{FAMILY}fn hook_surface(x: f32) -> f32 {{ var s = x; for (var i = 0; i < 4; i++) {{ s += 1.0; }} return s; }}\n\
         @fragment fn main() -> @location(0) vec4<f32> {{ return vec4(hook_surface(family_help())); }}\n"
    ))
    .expect("accepted");
    assert!(ok.accepted.surface.is_some_and(|c| c.operations > 4), "{ok:?}");
    assert!(ok.entry_frames["main"] >= 4);
    let calls = composed(&format!(
        "{FAMILY}fn hook_surface(x: f32) -> f32 {{ return x + family_help(); }}\n\
         @fragment fn main() -> @location(0) vec4<f32> {{ return vec4(hook_surface(1.0)); }}\n"
    ))
    .unwrap_err();
    assert!(calls.iter().any(|m| m.contains("not the hook's")), "{calls:?}");
    let reads = composed(&format!(
        "{FAMILY}fn hook_surface(x: f32) -> f32 {{ return x + secret.y; }}\n\
         @fragment fn main() -> @location(0) vec4<f32> {{ return vec4(hook_surface(family_help())); }}\n"
    ))
    .unwrap_err();
    assert!(reads.iter().any(|m| m.contains("module-scope variable or binding")), "{reads:?}");
    let spins = composed(&format!(
        "{FAMILY}fn hook_surface(x: f32) -> f32 {{ var s = x; loop {{ s += 1.0; if s > 1e30 {{ break; }} }} return s; }}\n\
         @fragment fn main() -> @location(0) vec4<f32> {{ return vec4(hook_surface(1.0)); }}\n"
    ))
    .unwrap_err();
    assert!(spins.iter().any(|m| m.contains("loops must be")), "{spins:?}");
    let full = composed(&format!(
        "{FAMILY}fn hook_surface(x: f32) -> f32 {{ var t: array<vec4<f32>, 96>; t[1] = vec4(x); return t[1].x; }}\n\
         @fragment fn main() -> @location(0) vec4<f32> {{ var big: array<vec4<f32>, 448>; big[2] = vec4(hook_surface(1.0)); return big[2]; }}\n"
    ))
    .unwrap_err();
    assert!(full.iter().any(|m| m.contains("over WGSL's 8192")), "{full:?}");
}
