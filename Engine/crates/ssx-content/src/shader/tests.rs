use super::*;

fn check(src: &str) -> Result<Accepted, Vec<Problem>> {
    check_hook(HookFamily::Object, src.as_bytes(), &Limits::default())
}

fn refused(src: &str, want: &str) {
    match check(src) {
        Ok(a) => panic!("accepted ({}), wanted a refusal with {want:?}:\n{src}", a.summary()),
        Err(p) => assert!(p.iter().any(|(_, _, m)| m.contains(want)), "{p:?} has no {want:?}"),
    }
}

const SURFACE_HEAD: &str = "fn surface(input: SurfaceIn, base: Surface, diffuse: texture_2d<f32>, diffuse_sampler: \
                            sampler, normal_map: texture_2d<f32>, specular_map: texture_2d<f32>, map_sampler: sampler) \
                            -> Surface {";

fn surface(body: &str) -> String {
    format!("{SURFACE_HEAD}\n    var out = base;\n{body}\n    return out;\n}}\n")
}

#[test]
fn the_templates_pass_with_their_worst_case() {
    let a = check(OBJECT_TEMPLATE).expect("the object template");
    assert!(a.surface.is_some() && a.vertex.is_some(), "{a:?}");
    let s = check_hook(HookFamily::Sky, SKY_TEMPLATE.as_bytes(), &Limits::default()).expect("the sky template");
    assert!(s.surface.is_some() && s.vertex.is_none());
    assert!(a.summary().contains("surface: at most"), "{}", a.summary());
    for f in [HookFamily::Object, HookFamily::Sky] {
        let i = check_hook(f, f.identity().as_bytes(), &Limits::default()).expect("the identity hooks");
        assert!(i.surface.is_some() && i.vertex.is_some());
    }
}

#[test]
fn a_counted_loop_is_bounded_by_its_trips_and_a_nest_by_their_product() {
    let one = check(&surface("    for (var i = 0; i < 8; i++) { out.specular += 0.01; }")).unwrap();
    let two = check(&surface("    for (var i = 0; i < 16; i++) { out.specular += 0.01; }")).unwrap();
    let (a, b) = (one.surface.unwrap().operations, two.surface.unwrap().operations);
    assert!(b > a && b - a >= 8 * 3, "{a} vs {b}");
    let reads = check(&surface(
        "    for (var i = 0u; i < 8u; i += 1u) { out.base_color += textureSampleLevel(diffuse, diffuse_sampler, input.uv + vec2(f32(i)), 0.0); }",
    ))
    .unwrap();
    assert_eq!(reads.surface.unwrap().texture_reads, 8);
    check(&surface("    for (var i = 10; i > 0; i -= 2) { out.specular += 0.1; }")).unwrap();
    check(&surface("    for (var i = 0; 12 > i; i = i + 3) { out.specular += 0.1; }")).unwrap();
    const NAMED: &str = "const N: i32 = 5;\n";
    check(&(NAMED.to_string() + &surface("    for (var i = 0; i <= N; i++) { out.specular += 0.1; }"))).unwrap();
    refused(
        &surface("    for (var i = 0; i < 256; i++) { for (var j = 0; j < 256; j++) { out.specular += f32(i * j); } }"),
        "operations per invocation, over the cap",
    );
    refused(
        &surface(
            "    for (var i = 0; i < 65; i++) { out.base_color += textureSampleLevel(diffuse, diffuse_sampler, input.uv, 0.0); }",
        ),
        "texture reads per invocation, over the cap",
    );
}

#[test]
fn every_unbounded_loop_is_refused() {
    refused(&surface("    loop { out.specular += 1.0; }"), "loops must be");
    refused(&surface("    loop { if out.specular > 1.0 { break; } out.specular += 1.0; }"), "loops must be");
    refused(&surface("    while true { out.specular += 1.0; }"), "loops must be");
    refused(&surface("    var i = 0;\n    while i < 8 { i++; }"), "loops must be");
    refused(&surface("    var i = 0;\n    loop { i++; continuing { break if i >= 8; } }"), "loops must be");
    refused(
        &surface("    for (var i = 0.0; i < input.time; i += 1.0) { out.specular += 1.0; }"),
        "bound is not a constant",
    );
    refused(
        &surface("    for (var i = 0; i < i32(input.time); i++) { out.specular += 1.0; }"),
        "bound is not a constant",
    );
    refused(
        &surface(
            "    let n = i32(textureSampleLevel(diffuse, diffuse_sampler, input.uv, 0.0).r * 100.0);\n    for (var i = 0; i < n; i++) { out.specular += 1.0; }",
        ),
        "bound is not a constant",
    );
    refused(
        &surface("    var s = 1;\n    for (var i = 0; i < 8; i += s) { out.specular += 1.0; }"),
        "is not a constant (`i++`",
    );
    refused(&surface("    for (var i = 0; i < 8; i++) { i = 0; }"), "changed in the loop's body");
    refused(&surface("    for (var i = 0; i < 8; i++) { i -= 1; }"), "changed in the loop's body");
    refused(
        &surface("    var i = i32(input.time);\n    for (; i < 8; i++) { out.specular += 1.0; }"),
        "not a constant",
    );
    refused(&surface("    for (var i = 0; i < 8; i--) { out.specular += 1.0; }"), "away from its bound");
    refused(&surface("    for (var i = 0; i < 8; i += 0) { out.specular += 1.0; }"), "away from its bound");
    refused(&surface("    for (var i = 0u; i <= 4294967295u; i++) { out.specular += 1.0; }"), "wrap");
    refused(&surface("    for (var i = 8u; i >= 0u; i--) { out.specular += 1.0; }"), "wrap");
    refused(&surface("    for (var i = 0; i < 100000; i++) { out.specular += 1.0; }"), "over the cap of 256 per loop");
    refused(
        &("fn bump(p: ptr<function, i32>) { *p = 0; }\n".to_string()
            + &surface("    for (var i = 0; i < 8; i++) { bump(&i); }")),
        "pointer to the loop counter",
    );
    refused(
        &surface("    var i = 0;\n    for (; i < 4; i++) { for (; i < 8; i++) { out.specular += 1.0; } }"),
        "counts 2 loops",
    );
    refused(&("fn spin() { loop { } }\n".to_string() + &surface("    out.specular = 0.5;")), "loops must be");
}

#[test]
fn bindings_entry_points_and_device_features_are_refused() {
    refused(&("@group(2) @binding(0) var<storage, read> data: array<f32>;\n".to_string() + &surface("")), "a binding");
    refused(&("@group(2) @binding(0) var<uniform> u: vec4<f32>;\n".to_string() + &surface("")), "a binding");
    refused(&("var<private> state: f32;\n".to_string() + &surface("")), "module-scope variable");
    refused(&("override k: f32 = 1.0;\n".to_string() + &surface("")), "override");
    refused(
        &("@fragment fn main() -> @location(0) vec4<f32> { return vec4(1.0); }\n".to_string() + &surface("")),
        "entry point",
    );
    refused(&("@compute @workgroup_size(1) fn cs() { }\n".to_string() + &surface("")), "entry point");
    refused(&("diagnostic(off, derivative_uniformity);\n".to_string() + &surface("")), "a directive");
    refused(&("enable f16;\nfn h(x: f16) -> f16 { return x; }\n".to_string() + &surface("")), "a directive");
    refused(
        &("@diagnostic(off, derivative_uniformity) fn quiet() {}\n".to_string() + &surface("")),
        "a `diagnostic` filter",
    );
    refused(&("fn b() { workgroupBarrier(); }\n".to_string() + &surface("")), "barrier");
    refused(&("fn a(p: ptr<storage, atomic<u32>, read_write>) { atomicAdd(p, 1u); }\n".to_string() + &surface("")), "");
}

#[test]
fn the_interface_and_the_file_are_checked() {
    let p = check("fn surface(\n  oops").unwrap_err();
    assert!(p[0].2.starts_with("not valid WGSL"), "{p:?}");
    assert!(p[0].0 >= 1 && p[0].0 <= 2, "the line in the file, not the composed module: {p:?}");
    refused("fn surface(input: SurfaceIn) -> Surface { return Surface(); }", "`surface` must be");
    refused("fn vertex_offset(input: VertexIn) -> vec4<f32> { return vec4(0.0); }", "`vertex_offset` must be");
    refused("fn helper() -> f32 { return 1.0; }", "defines no hook");
    refused("struct Surface { x: f32 }\nfn vertex_offset(input: VertexIn) -> vec3<f32> { return vec3(0.0); }", "");
    let big = format!("// {}\n{}", "x".repeat(70_000), OBJECT_TEMPLATE);
    refused(&big, "over the shader cap");
    refused("fn vertex_offset(input: VertexIn) -> vec3<f32> { return vec3(0.0); }\0", "NUL");
    assert!(check_hook(HookFamily::Object, &[0xFF, 0xFE], &Limits::default()).is_err());
    refused("import a::b;\nfn vertex_offset(input: VertexIn) -> vec3<f32> { return vec3(0.0); }", "not valid WGSL");
    let v = check("fn vertex_offset(input: VertexIn) -> vec3<f32> { return input.normal * 0.1; }").unwrap();
    assert!(v.surface.is_none() && v.vertex.is_some());
}
