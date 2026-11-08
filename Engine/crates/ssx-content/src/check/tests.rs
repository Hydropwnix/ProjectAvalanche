use super::*;
use crate::test_dir::{TestDir, png_rgba};

const HEAD: &str = "[resource]\nname = \"pack\"\nversion = \"1.0.0\"\napi = 1\n";

fn run(dir: &TestDir) -> Checked {
    check_package(&dir.0, &texture::Limits::default())
}

fn texts(c: &Checked, severity: Severity) -> Vec<String> {
    c.findings
        .iter()
        .filter(|f| f.severity == severity)
        .map(|f| format!("{}:{}: {}", f.file, f.line, f.message))
        .collect()
}

fn has(c: &Checked, severity: Severity, want: &str) {
    let all = texts(c, severity);
    assert!(all.iter().any(|t| t.contains(want)), "{want:?} not in {all:#?}");
}

#[test]
fn a_clean_texture_pack_passes() {
    let d = TestDir::new("check-ok");
    let manifest = format!("{HEAD}[override]\n\"ssx:texture/worlds/textures/rock\" = \"textures/rock.png\"\n");
    d.write("resource.toml", &manifest);
    d.write_bytes("textures/rock.png", &png_rgba(8, 8, [9, 8, 7, 255]));
    d.write("textures/textures.toml", "[\"rock.png\"]\nkind = \"color\"\n");
    let c = run(&d);
    assert_eq!(c.errors(), 0, "{:#?}", c.findings);
    assert!(texts(&c, Severity::Warning).is_empty(), "{:#?}", c.findings);
    let t = &c.textures[&"ssx:texture/worlds/textures/rock".parse().unwrap()];
    assert_eq!((t.width, t.kind), (8, Some(TextureKind::Color)));
}

#[test]
fn missing_mis_cased_and_broken_files_are_errors_at_their_line() {
    let d = TestDir::new("check-files");
    let manifest = format!(
        "{HEAD}[override]\n\"ssx:texture/a\" = \"textures/a.png\"\n\"ssx:texture/b\" = \"textures/B.png\"\n\
         \"ssx:texture/c\" = \"textures/c.png\"\n\"ssx:texture/d\" = \"textures/d.png\"\n\
         [add]\n\"track/x\" = \"data/x.toml\"\n"
    );
    d.write("resource.toml", &manifest);
    d.write_bytes("textures/b.png", &png_rgba(2, 2, [0; 4]));
    d.write_bytes("textures/c.png", b"\xFF\xD8\xFF\xE0 not a png");
    d.write_bytes("textures/d.png", &png_rgba(65, 1, [0; 4]));
    d.write("data/x.toml", "a = 1\nb = [\n");
    let c = check_package(&d.0, &texture::Limits { max_side: 64, ..texture::Limits::default() });
    has(
        &c,
        Severity::Error,
        "resource.toml:6: ssx:texture/a: names \"textures/a.png\", which the package does not have",
    );
    has(&c, Severity::Error, "resource.toml:7: ssx:texture/b: names \"textures/B.png\"");
    has(&c, Severity::Error, "resource.toml:8: ssx:texture/c: textures/c.png: named .png, but its content is JPEG");
    has(&c, Severity::Error, "resource.toml:9: ssx:texture/d: textures/d.png: 65×1 is larger than 64×64");
    has(&c, Severity::Error, "data/x.toml:");
    has(&c, Severity::Note, "the game does not read this kind from packages; only the files' presence and records");
    has(&c, Severity::Warning, "textures/b.png:0: named by no entry");
}

#[test]
fn documentation_at_the_root_is_packed_without_a_warning() {
    let d = TestDir::new("check-docs");
    d.write("resource.toml", HEAD);
    for f in ["README.md", "LICENSE", "licence.txt", "docs/README.txt", "readme.png", "NOTES.txt"] {
        d.write(f, "text\n");
    }
    let c = run(&d);
    let warned = texts(&c, Severity::Warning);
    for f in ["README.md", "LICENSE", "licence.txt"] {
        assert!(!warned.iter().any(|t| t.starts_with(&format!("{f}:"))), "{f} warned: {warned:#?}");
    }
    for f in ["docs/README.txt", "readme.png", "NOTES.txt"] {
        has(&c, Severity::Warning, &format!("{f}:0: named by no entry"));
    }
}

#[test]
fn the_tree_refuses_links_reserved_names_and_bad_settings() {
    let d = TestDir::new("check-tree");
    d.write("resource.toml", HEAD);
    d.write("package-index.tsv", "path\tsize\tblake3\n");
    d.write("textures/textures.toml", "\n[\"gone.png\"]\nkind = \"normal\"\n");
    d.write("other/textures.toml", "[\"a.png\"]\nkind = \"colour\"\n");
    d.write_bytes("other/a.png", &png_rgba(1, 1, [0; 4]));
    d.write("a b.txt", "x");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", d.0.join("textures/evil.png")).unwrap();
    let c = run(&d);
    has(&c, Severity::Error, "package-index.tsv:0: package-index.tsv is the packed archive's content index");
    has(&c, Severity::Error, "textures/textures.toml:2: [\"gone.png\"]: this folder has no file of that name");
    has(&c, Severity::Error, "other/textures.toml:2:");
    has(&c, Severity::Error, "\"a b.txt\": a name packages cannot use");
    #[cfg(unix)]
    has(&c, Severity::Error, "textures/evil.png:0: a link");
}

#[test]
fn a_broken_manifest_still_gets_the_tree_checked() {
    let d = TestDir::new("check-manifest");
    d.write("resource.toml", "[resource]\nname = \"Bad Name\"\nversion = \"1\"\napi = 1\n");
    d.write("t/textures.toml", "[\"x.png\"]\n");
    let c = run(&d);
    has(&c, Severity::Error, "resource.toml:2: bad package name");
    has(&c, Severity::Error, "resource.toml:3:");
    has(&c, Severity::Error, "t/textures.toml:1: [\"x.png\"]: this folder has no file of that name");
    assert!(c.manifest.is_none());
}

#[test]
fn patterns_that_match_nothing_or_cost_too_much() {
    let d = TestDir::new("check-patterns");
    let manifest =
        format!("{HEAD}[scripts]\nclient = [\"client/*.luau\", \"none/*.luau\"]\n[files]\ninclude = [\"ui/[a-\"]\n");
    d.write("resource.toml", &manifest);
    d.write("client/hud.luau", "print(1)");
    let c = run(&d);
    has(&c, Severity::Warning, "resource.toml:5: [scripts.client] \"none/*.luau\" matches no file");
    has(&c, Severity::Error, "resource.toml:7: [files.include]");
    has(&c, Severity::Note, "compiled and their `require`s resolved when the scripts are loaded");
    assert!(!texts(&c, Severity::Warning).iter().any(|t| t.contains("hud.luau")), "matched by a pattern");
}

fn prop_glb(lods: usize) -> Vec<u8> {
    use crate::model::write::{WriteModel, glb};
    let quad = crate::model::Mesh {
        name: "q".into(),
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
        normals: Some(vec![[0.0, 0.0, 1.0]; 4]),
        indices: vec![0, 1, 2, 0, 2, 3],
        ..Default::default()
    };
    let corners = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
    let (col, _) =
        crate::model::collision::build(&[crate::model::CollisionTriangle { corners, surface_id: 30, group: 0 }]);
    let w = WriteModel {
        name: "boulder",
        lods: (0..lods).map(|_| vec![(&quad, Some(0))]).collect(),
        materials: vec!["rock".into()],
        collision: Some(&col),
    };
    glb(&w).unwrap()
}

#[test]
fn a_prop_package_passes_and_names_the_base_game_through_references() {
    let d = TestDir::new("check-prop");
    d.write(
        "resource.toml",
        &format!(
            "{HEAD}[add]\n\"model/boulder\" = \"models/boulder.glb\"\n\"material/rock\" = \"materials/rock.toml\"\n\
             \"object/boulder\" = \"objects/boulder.toml\"\n[override]\n\"ssx:object/rock_gameobject\" = \"objects/boulder.toml\"\n"
        ),
    );
    d.write_bytes("models/boulder.glb", &prop_glb(2));
    d.write(
        "materials/rock.toml",
        "family = \"object\"\neffect = \"object_default\"\n[textures]\ndiffuseTexture = \"ssx:texture/worlds/textures/rock\"\n",
    );
    d.write(
        "objects/boulder.toml",
        "model = \"pack:model/boulder\"\nlod_thresholds = [0.4, 0.02]\ncontroller = \"ssx:script/scripts/x\"\n\
         [materials]\n\"rock\" = \"pack:material/rock\"\n",
    );
    let c = run(&d);
    assert_eq!(c.errors(), 0, "{:#?}", c.findings);
    let mf = &c.models[&"pack:model/boulder".parse().unwrap()];
    assert_eq!((mf.lods.len(), mf.collision_triangles), (2, 1));
    let refs: Vec<String> = c.references.iter().map(|r| format!("{} {}:{}", r.id, r.file, r.at.0)).collect();
    assert_eq!(
        refs,
        ["ssx:texture/worlds/textures/rock materials/rock.toml:4", "ssx:script/scripts/x objects/boulder.toml:3"],
        "the base game's ids are the caller's to check, each at its line"
    );
}

#[test]
fn hostile_and_inconsistent_props_are_reported_at_their_lines() {
    let d = TestDir::new("check-prop-bad");
    d.write(
        "resource.toml",
        &format!(
            "{HEAD}[add]\n\"model/boulder\" = \"models/boulder.glb\"\n\"model/broken\" = \"models/broken.glb\"\n\
             \"material/rock\" = \"materials/rock.toml\"\n\"object/a\" = \"objects/a.toml\"\n\"object/b\" = \"objects/b.toml\"\n\
             [override]\n\"ssx:model/worlds/geometry/x_geo\" = \"models/boulder.glb\"\n"
        ),
    );
    d.write_bytes("models/boulder.glb", &prop_glb(2));
    let mut broken = prop_glb(1);
    broken.truncate(broken.len() - 8);
    d.write_bytes("models/broken.glb", &broken);
    d.write("materials/rock.toml", "family = \"object\"\neffect = \"object_default\"\n[parameters]\nrimMult = 1.0\n");
    d.write("objects/a.toml", "model = \"pack:model/missing\"\nlod_thresholds = [0.4]\n");
    d.write(
        "objects/b.toml",
        "model = \"pack:model/boulder\"\nlod_thresholds = [0.4]\ncontroller = \"pack:script/brain\"\n[materials]\n\"moss\" = \"pack:material/rock\"\n",
    );
    let c = run(&d);
    has(&c, Severity::Error, "models/broken.glb:0: the header states");
    has(&c, Severity::Error, "materials/rock.toml:4: \"rimMult\" is not a parameter of the object family");
    has(&c, Severity::Error, "objects/a.toml:1: model pack:model/missing: this package's [add] has no such id");
    has(
        &c,
        Severity::Error,
        "objects/b.toml:3: controller pack:script/brain: a controller is one of the game's own Lua",
    );
    has(&c, Severity::Warning, "objects/b.toml:2: lod_thresholds has 1 values for the model's 2 LODs");
    has(&c, Severity::Warning, "objects/b.toml:5: [materials] \"moss\": the model has no glTF material of that name");
    has(
        &c,
        Severity::Warning,
        "resource.toml:12: ssx:model/worlds/geometry/x_geo: in place of one disc mesh only LOD 0",
    );
}

#[test]
fn skies_records_and_shader_hooks_are_checked() {
    let d = TestDir::new("check-m4");
    let manifest = format!(
        "{HEAD}[shaders]\nfiles = [\"shaders/*.wgsl\"]\n[add]\n\"sky/hdr\" = \"skies/sky.hdr\"\n\"sky/faces\" = \"skies/faces.toml\"\n\
         \"postfx/warm_cube\" = \"postfx/warm.png\"\n\"particles/spark\" = \"particles/spark.toml\"\n\
         \"material/glow\" = \"materials/glow.toml\"\n[patch]\n\"ssx:lighting/day_sunny_lighting\" = \"lighting/day.toml\"\n\
         \"ssx:postfx/day_sunny_postfx\" = \"postfx/day.toml\"\n\"ssx:particles/snowspray/base_snowspray\" = \"particles/spray.toml\"\n"
    );
    d.write("resource.toml", &manifest);
    d.write_bytes("skies/sky.hdr", &crate::sky::tests::hdr_file(16, 8, |_, _| [1.0, 0.5, 0.25]));
    for f in ["px", "nx", "py", "ny", "pz", "nz"] {
        d.write_bytes(&format!("skies/{f}.png"), &png_rgba(8, 8, [10, 20, 30, 255]));
    }
    d.write(
        "skies/faces.toml",
        "type = \"cubemap\"\nfaces = [\"skies/px.png\", \"skies/nx.png\", \"skies/py.png\", \"skies/ny.png\", \"skies/pz.png\", \"skies/nz.png\"]\n",
    );
    d.write_bytes("postfx/warm.png", &png_rgba(1024, 32, [100, 90, 80, 255]));
    d.write("postfx/day.toml", "cube = \"pack:postfx/warm_cube\"\nbloom_blur_x = 2.0\n");
    d.write("lighting/day.toml", "sky = \"pack:sky/hdr\"\nrampIntensity = 1.2\n");
    d.write(
        "particles/spark.toml",
        "[[effect]]\nname = \"spark\"\n[[effect.group]]\nname = \"g\"\n[[effect.group.action]]\nclass = \"ParticleActionEmitBox\"\nparameters = [[\"iEmitRate\", 16]]\n",
    );
    d.write("particles/spray.toml", "[snow.carve]\nparticles = \"pack:particles/spark\"\neffect = \"spark\"\n");
    d.write(
        "materials/glow.toml",
        "family = \"object\"\neffect = \"object_default\"\nshader = \"shaders/glow.wgsl\"\n",
    );
    d.write("shaders/glow.wgsl", crate::shader::OBJECT_TEMPLATE);
    let c = run(&d);
    assert_eq!(c.errors(), 0, "{:#?}", c.findings);
    has(&c, Severity::Note, "skies/sky.hdr:0: a cubemap sky: faces 4², 3 levels");
    has(&c, Severity::Note, "shaders/glow.wgsl:0: accepted: object family hooks, surface: at most");
    has(&c, Severity::Note, "contains GPU code");
    assert_eq!((c.m4.skies.len(), c.m4.lighting.len(), c.m4.postfx.len(), c.m4.particles.len()), (2, 1, 1, 2));
    d.write(
        "shaders/glow.wgsl",
        "fn surface(input: SurfaceIn, base: Surface, d: texture_2d<f32>, s: sampler, n: texture_2d<f32>, sp: texture_2d<f32>, m: sampler) -> Surface {\n    var out = base;\n    loop { out.specular += 1.0; if out.specular > 2.0 { break; } }\n    return out;\n}\n",
    );
    d.write("shaders/stray.wgsl", "fn x() {}");
    d.write("shaders.wgsl", "fn y() {}");
    let hdr = crate::sky::tests::hdr_file(16, 8, |_, _| [1.0; 3]);
    d.write_bytes("skies/sky.hdr", &hdr[..hdr.len() / 2]);
    d.write_bytes("skies/nz.png", &png_rgba(16, 16, [0; 4]));
    d.write("lighting/day.toml", "sky = \"pack:texture/x\"\n");
    d.write_bytes("postfx/warm.png", &png_rgba(32, 32, [0; 4]));
    let c = run(&d);
    has(&c, Severity::Error, "shaders/glow.wgsl:3:");
    has(&c, Severity::Error, "loops must be");
    has(&c, Severity::Error, "shaders.wgsl:0: GPU code the manifest does not declare");
    has(&c, Severity::Error, "skies/sky.hdr:0: a damaged Radiance HDR image");
    has(&c, Severity::Error, "the faces are squares of one size");
    has(&c, Severity::Error, "lighting/day.toml:1: sky: pack:texture/x is not a sky id");
    has(&c, Severity::Error, "1024×32");
}

#[test]
fn a_server_refuses_gpu_code() {
    use crate::package::{Origin, PackageSource};
    let d = TestDir::new("check-server-gpu");
    d.write("resource.toml", &format!("{HEAD}[shaders]\nfiles = [\"shaders/*.wgsl\"]\n"));
    d.write("shaders/a.wgsl", "fn vertex_offset(input: VertexIn) -> vec3<f32> { return vec3(0.0); }");
    assert!(PackageSource::open_as(&d.0, Origin::Client).is_ok());
    let e = PackageSource::open_as(&d.0, Origin::Server).err().expect("refused");
    assert!(e.to_string().contains("GPU code"), "{e}");
    let quiet = TestDir::new("check-server-gpu-quiet");
    quiet.write("resource.toml", HEAD);
    quiet.write("deep/x.spv", "spirv");
    let e = PackageSource::open_as(&quiet.0, Origin::Server).err().expect("refused");
    assert!(e.to_string().contains("deep/x.spv: GPU code"), "{e}");
    quiet.write("deep/x.spv", "");
    std::fs::remove_file(quiet.0.join("deep/x.spv")).expect("removed");
    assert!(PackageSource::open_as(&quiet.0, Origin::Server).is_ok());
}

#[test]
fn a_material_hook_is_read_with_its_text() {
    use crate::package::PackageSource;
    use crate::registry::Source;
    let d = TestDir::new("check-hook-text");
    d.write(
        "resource.toml",
        &format!(
            "{HEAD}[shaders]\nfiles = [\"shaders/*.wgsl\"]\n[add]\n\"material/glow\" = \"materials/glow.toml\"\n\
             [override]\n\"ssx:material/rock_mat\" = \"materials/glow.toml\"\n\"ssx:material/wood_mat\" = \"materials/bare.toml\"\n"
        ),
    );
    d.write(
        "materials/glow.toml",
        "family = \"object\"\neffect = \"object_default\"\nshader = \"shaders/glow.wgsl\"\n",
    );
    d.write("materials/bare.toml", "family = \"object\"\neffect = \"object_default\"\nshader = \"hidden/x.wgsl\"\n");
    d.write("shaders/glow.wgsl", crate::shader::OBJECT_TEMPLATE);
    d.write("hidden/x.wgsl", crate::shader::OBJECT_TEMPLATE);
    let src = PackageSource::open(&d.0).expect("opens");
    for id in ["pack:material/glow", "ssx:material/rock_mat"] {
        let asset = src.load(&id.parse().unwrap()).expect("loads");
        let def = asset.value::<crate::material::MaterialDef>().expect("a material");
        let hook = def.shader.as_ref().expect("its hook");
        assert_eq!(hook.source.as_deref(), Some(crate::shader::OBJECT_TEMPLATE), "{id}");
        assert!(hook.problem.is_none(), "{id}: {:?}", hook.problem);
    }
    let asset = src.load(&"ssx:material/wood_mat".parse().unwrap()).expect("loads");
    let def = asset.value::<crate::material::MaterialDef>().expect("a material");
    let hook = def.shader.as_ref().expect("its hook");
    assert!(hook.source.is_none() && hook.problem.as_deref().is_some_and(|p| p.contains("does not declare")));
}

#[test]
fn a_clip_is_read_at_check_as_the_game_loads_it_and_its_problems_are_at_their_lines() {
    use crate::anim::{Clip, template};
    use crate::package::PackageSource;
    use crate::registry::Source;
    let d = TestDir::new("check-anim");
    let manifest = format!("{HEAD}[override]\n\"{}\" = \"anims/carve_sway.toml\"\n", template::TARGET);
    d.write("resource.toml", &manifest);
    d.write("anims/carve_sway.toml", template::ANIM_TOML);
    d.write_bytes(template::GLTF_PATH, &template::glb().expect("the template's glTF"));
    let c = run(&d);
    assert_eq!(c.errors(), 0, "{:#?}", c.findings);
    assert!(texts(&c, Severity::Warning).is_empty(), "the glTF the record names is referenced: {:#?}", c.findings);
    has(&c, Severity::Note, "5 joint tracks, 0 float tracks, 3.000 s (180 ticks), additive");
    let id: ContentId = template::TARGET.parse().unwrap();
    let (file, checked) = &c.anims[&id];
    assert_eq!(file, "anims/carve_sway.toml");
    let loaded = PackageSource::open(&d.0).expect("opens").load(&id).expect("loads");
    assert_eq!(loaded.value::<Clip>().expect("a clip"), checked);

    d.write("anims/carve_sway.toml", &format!("{}speed = 2\n", template::ANIM_TOML));
    let c = run(&d);
    let line = template::ANIM_TOML.lines().count() + 1;
    has(&c, Severity::Error, &format!("anims/carve_sway.toml:{line}: unknown field `speed`"));
    assert!(c.anims.is_empty());
    d.write("anims/carve_sway.toml", &template::ANIM_TOML.replace("carve_sway.glb", "gone.glb"));
    let c = run(&d);
    has(&c, Severity::Error, "anims/gone.glb");
    assert!(c.anims.is_empty());
}
