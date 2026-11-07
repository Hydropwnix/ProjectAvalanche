use super::*;

fn problems(text: &str) -> Vec<String> {
    parse_toml(text.as_bytes()).err().unwrap_or_default().into_iter().map(|(l, c, m)| format!("{l}:{c}: {m}")).collect()
}

#[test]
fn a_full_marker_reads_with_its_values() {
    let m = parse_toml(
        br##"shape = "gate"
size = 8.5
color = [255, 128, 0]
glow = 2.0
facing = "camera"
draw_distance = 300.0
icon = "pack:texture/icons/gate"
sound = "pack:sound/gate"
hit = "ring"

[animation]
spin = 90.0
bob = 0.5
bob_period = 3.0
pulse = 0.2
pulse_period = 0.5
"##,
    )
    .unwrap();
    assert_eq!(m.look, Look::Shape(Shape::Gate));
    assert_eq!((m.size, m.colour, m.glow, m.facing, m.draw_distance), (8.5, 0xFFFF_8000, 2.0, Facing::Camera, 300.0));
    assert_eq!(m.icon.unwrap().as_str(), "pack:texture/icons/gate");
    assert_eq!(m.hit, Hit::Ring, "the file's hit over the shape's");
    assert_eq!(m.animation.spin, 90.0);
    let model = parse_toml(b"model = \"pack:model/flag\"\ncolor = \"#11223344\"\n").unwrap();
    assert_eq!((model.colour, model.hit, model.size), (0x4411_2233, Hit::Sphere, 4.0));
    assert!(matches!(model.look, Look::Model(ref id) if id.as_str() == "pack:model/flag"));
}

#[test]
fn every_problem_is_reported_with_its_line() {
    let p = problems(
        "shape = \"star\"\nsize = 0.0\ncolor = \"#12345\"\nglow = 9\nfacing = \"up\"\ndraw_distance = 1e9\nicon = \"pack:sound/x\"\nhit = \"box\"\n[animation]\nspin = 1000.0\npulse = 2.0\n",
    );
    for (line, needle) in [
        ("1:9", "shape: one of"),
        ("2:8", "size: a number from 0.1 to 100"),
        ("3:9", "color: a colour is #RRGGBB"),
        ("4:8", "glow: a number from 0 to 8"),
        ("5:10", "facing:"),
        ("6:17", "draw_distance"),
        ("7:8", "is not a texture id"),
        ("8:7", "hit: one of"),
        ("10:8", "animation.spin"),
        ("11:9", "animation.pulse"),
    ] {
        assert!(p.iter().any(|l| l.starts_with(line) && l.contains(needle)), "{line} {needle}: {p:?}");
    }
    assert!(problems("size = 2.0\n")[0].contains("names a `shape` or a `model`"));
    assert!(problems("shape = \"ring\"\nmodel = \"a:model/b\"\n")[0].contains("not both"));
    assert!(problems("shape = \"ring\"\nlabel = \"x\"\n")[0].contains("unknown field"));
    assert!(problems("shape = \"ring\"\ncolor = [1, 2, 300]\n")[0].contains("0–255"));
    assert!(problems("shape = \"ring\"\nsize = nan\n")[0].contains("size"));
    assert!(problems("shape = \"ring\"\n[animation]\nwobble = 1.0\n")[0].contains("unknown field"));
}

#[test]
fn a_patched_record_reads_as_its_file() {
    let mut t: toml::Table = toml::from_str("shape = \"ring\"\nsize = 6.0\n").unwrap();
    t.insert("size".into(), toml::Value::Float(12.0));
    assert_eq!(from_table(&t).unwrap().size, 12.0);
    t.insert("size".into(), toml::Value::Float(1000.0));
    assert!(from_table(&t).is_err());
}

#[test]
fn the_shapes_read_back_through_the_model_reader() {
    let limits = crate::model::Limits::default();
    for shape in Shape::ALL {
        let text = shapes::gltf(shape);
        let file = format!("{}.gltf", shape.name());
        let model = crate::model::decode_file(&file, text.as_bytes(), &(), &limits)
            .unwrap_or_else(|e| panic!("{}: {}", shape.name(), e.message));
        let mesh = &model.lods[0].parts[0].mesh;
        let built = shapes::mesh(shape);
        assert_eq!(mesh.positions, built.positions, "{}", shape.name());
        assert_eq!(mesh.indices, built.indices, "{}", shape.name());
        assert!(mesh.triangle_count() >= 8 && mesh.positions.len() < 4096, "{}", shape.name());
        let def = parse_toml(shapes::marker_toml(shape).as_bytes()).unwrap();
        assert_eq!(def.look, Look::Shape(shape));
        assert_eq!(def.hit, shape.default_hit());
    }
}

#[test]
fn the_committed_base_set_is_the_generated_one() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/markers");
    for shape in Shape::ALL {
        for (ext, made) in [("gltf", shapes::gltf(shape)), ("toml", shapes::marker_toml(shape))] {
            let path = dir.join(format!("{}.{ext}", shape.name()));
            let committed = std::fs::read_to_string(&path).unwrap_or_default();
            assert!(
                committed == made,
                "{} differs from the generator: run `cargo run -p ssx-content --example marker_shapes`",
                path.display()
            );
        }
    }
}
