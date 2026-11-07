use super::*;
use crate::registry::{Layer, MemorySource, Registry};
use crate::test_dir::TestDir;

const MANIFEST: &str = r#"
[resource]
name = "neon-pack"
version = "1.0.0"
api = 1
[add]
"board/neon" = { model = "models/neon.glb", data = "data/neon.toml" }
[override]
"ssx:texture/boards/elise_d" = "textures/elise_d.png"
"ssx:lighting/dawn_lighting" = "data/dawn.toml"
[patch]
"ssx:board/elise_default" = "patches/elise.toml"
"#;

fn id(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn png_2x1(rgba: [u8; 8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 2, 1);
    e.set_color(png::ColorType::Rgba);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&rgba).unwrap();
    w.finish().unwrap();
    out
}

fn texture(asset: &Asset) -> &crate::texture::Texture {
    asset.value::<crate::texture::Texture>().expect("a texture")
}

fn package() -> (TestDir, PathBuf) {
    let dir = TestDir::new("pkg");
    dir.write("pkg/resource.toml", MANIFEST);
    dir.write("pkg/models/neon.glb", "glTF");
    dir.write("pkg/data/neon.toml", "speed = 1");
    dir.write_bytes("pkg/textures/elise_d.png", &png_2x1([10, 20, 30, 255, 40, 50, 60, 255]));
    dir.write("pkg/data/dawn.toml", "fog = [1, 2\n");
    dir.write("pkg/patches/elise.toml", "[stats]\nspeed = 8");
    let root = dir.0.join("pkg");
    (dir, root)
}

#[test]
fn a_package_overrides_adds_and_patches() {
    let (_dir, root) = package();
    let mut r = Registry::new();
    let base = MemorySource::new("ssx")
        .with(id("ssx:texture/boards/elise_d"), Asset::Bytes(Arc::from(&b"disc"[..])))
        .with(id("ssx:board/elise_default"), Asset::Record(toml::from_str("[stats]\nspeed = 3").unwrap()));
    r.mount(Layer::Base, 0, Box::new(base));
    r.mount(Layer::ClientMods, 0, Box::new(PackageSource::open(&root).unwrap()));
    let tex = r.load(&id("ssx:texture/boards/elise_d")).unwrap();
    assert_eq!(tex.source, "neon-pack");
    let t = texture(&tex.asset);
    assert_eq!((t.width(), t.height(), t.levels.len()), (2, 1, 2));
    assert_eq!(t.levels[0].images[0], [10, 20, 30, 255, 40, 50, 60, 255]);
    let neon = r.load(&id("neon-pack:board/neon")).unwrap();
    let entry = neon.asset.value::<crate::rider::RiderEntry>().expect("a rider entry");
    assert_eq!(entry.model.as_deref(), Some("models/neon.glb"));
    let (file, table) = entry.data.as_ref().unwrap();
    assert_eq!((file.as_str(), table["speed"].as_integer()), ("data/neon.toml", Some(1)));
    let board = r.load(&id("ssx:board/elise_default")).unwrap();
    assert_eq!(board.asset.record().unwrap()["stats"]["speed"].as_integer(), Some(8));
    let e = r.load(&id("ssx:lighting/dawn_lighting")).unwrap_err().to_string();
    assert!(e.starts_with("ssx:lighting/dawn_lighting: data/dawn.toml:1:"), "{e}");
    assert_eq!(r.list(Kind::Board).len(), 2);
    assert!(r.report().dangling_patches.is_empty());
}

#[test]
fn a_missing_manifest_is_an_error_with_the_file() {
    let dir = TestDir::new("nomanifest");
    let e = PackageSource::open(&dir.0).err().unwrap();
    assert!(e.file.ends_with("resource.toml") && e.diagnostics[0].line == 0);
}

#[test]
fn hostile_files_are_refused() {
    let (dir, root) = package();
    dir.write("pkg/patches/elise.toml", &format!("x = {}1{}", "[".repeat(40), "]".repeat(40)));
    std::fs::rename(root.join("models/neon.glb"), root.join("models/Neon.glb")).unwrap();
    dir.write("outside.png", "secret");
    #[cfg(unix)]
    {
        std::fs::remove_file(root.join("textures/elise_d.png")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("outside.png"), root.join("textures/elise_d.png")).unwrap();
    }
    let pkg = PackageSource::open(&root).unwrap();
    let e = pkg.patch(&id("ssx:board/elise_default")).unwrap_err().to_string();
    assert!(e.contains("nested deeper than 32"), "{e}");
    let e = pkg.load(&id("neon-pack:board/neon")).unwrap_err().to_string();
    assert!(e.contains("case-sensitive"), "{e}");
    let e = pkg.load(&id("ssx:lighting/dawn_lighting")).unwrap_err().to_string();
    assert!(e.contains("data/dawn.toml:1:"), "{e}");
    #[cfg(unix)]
    {
        let e = pkg.load(&id("ssx:texture/boards/elise_d")).unwrap_err().to_string();
        assert!(e.contains("is a link"), "{e}");
    }
    assert!(pkg.patch(&id("ssx:board/other")).unwrap().is_none());
}

#[test]
fn hostile_manifests_are_refused() {
    let dir = TestDir::new("badmanifest");
    dir.write("resource.toml", &"#".repeat(MAX_MANIFEST_BYTES + 1));
    let e = PackageSource::open(&dir.0).err().unwrap().to_string();
    assert!(e.contains("bytes"), "{e}");
    dir.write("resource.toml", "[resource]\nname = \"\u{feff}x\"");
    assert!(PackageSource::open(&dir.0).is_err());
    std::fs::write(dir.0.join("resource.toml"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(PackageSource::open(&dir.0).err().unwrap().to_string().contains("not UTF-8"));
}

#[test]
fn textures_take_their_folder_settings_and_fail_with_the_file() {
    let (dir, root) = package();
    dir.write("pkg/textures/textures.toml", "[\"elise_d.png\"]\nkind = \"normal\"\n");
    let pkg = PackageSource::open(&root).unwrap();
    let t = pkg.load(&id("ssx:texture/boards/elise_d")).unwrap();
    let t = texture(&t);
    assert_eq!(
        (t.kind, &t.levels[0].images[0][..4]),
        (Some(crate::texture::TextureKind::Normal), &[10, 20, 0, 255][..])
    );
    dir.write("pkg/textures/textures.toml", "[\"elise_d.png\"]\nkind = 3\n");
    let pkg = PackageSource::open(&root).unwrap();
    let e = pkg.load(&id("ssx:texture/boards/elise_d")).unwrap_err().to_string();
    assert!(e.contains("textures/textures.toml:2:"), "{e}");
    std::fs::remove_file(root.join("textures/textures.toml")).unwrap();
    dir.write_bytes("pkg/textures/elise_d.png", b"\xFF\xD8\xFF\xE0rest");
    let pkg = PackageSource::open(&root).unwrap();
    let e = pkg.load(&id("ssx:texture/boards/elise_d")).unwrap_err().to_string();
    assert!(e.contains("textures/elise_d.png: named .png, but its content is JPEG"), "{e}");
    dir.write_bytes("pkg/textures/elise_d.png", &png_2x1([0; 8]));
    let small = crate::texture::Limits { max_file_bytes: 16, ..Default::default() };
    let pkg = PackageSource::open(&root).unwrap().with_texture_limits(small);
    assert!(pkg.load(&id("ssx:texture/boards/elise_d")).is_err());
}

#[test]
fn a_texture_is_one_file() {
    let dir = TestDir::new("texroles");
    dir.write(
        "resource.toml",
        "[resource]\nname = \"p\"\nversion = \"1.0.0\"\napi = 1\n[add]\n\"texture/a\" = { a = \"a.png\", b = \"b.png\" }\n",
    );
    dir.write_bytes("a.png", &png_2x1([0; 8]));
    dir.write_bytes("b.png", &png_2x1([0; 8]));
    let pkg = PackageSource::open(&dir.0).unwrap();
    let e = pkg.load(&id("p:texture/a")).unwrap_err().to_string();
    assert!(e.contains("a texture is one file"), "{e}");
}

#[test]
fn model_ids_naming_one_file_share_one_decode() {
    let dir = TestDir::new("models");
    let mut manifest = String::from("[resource]\nname = \"many\"\nversion = \"1.0.0\"\napi = 1\n[add]\n");
    for i in 0..256 {
        manifest.push_str(&format!("\"model/m{i}\" = \"models/m.glb\"\n"));
    }
    dir.write("pkg/resource.toml", &manifest);
    let mesh = model::Mesh {
        positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        indices: vec![0, 1, 2],
        ..model::Mesh::default()
    };
    let w = model::write::WriteModel { name: "m", lods: vec![vec![(&mesh, None)]], materials: vec![], collision: None };
    dir.write_bytes("pkg/models/m.glb", &model::write::glb(&w).unwrap());
    let p = PackageSource::open(&dir.0.join("pkg")).unwrap();
    let models: Vec<Arc<dyn std::any::Any + Send + Sync>> = (0..256)
        .map(|i| match p.load(&id(&format!("many:model/m{i}"))).unwrap() {
            Asset::Value(v) => v,
            _ => panic!("a decoded model"),
        })
        .collect();
    assert!(models.iter().all(|m| Arc::ptr_eq(m, &models[0])), "one decode, shared");
    dir.write_bytes("pkg/models/m.glb", b"glTF broken");
    let p = PackageSource::open(&dir.0.join("pkg")).unwrap();
    for i in [0, 255] {
        let e = p.load(&id(&format!("many:model/m{i}"))).unwrap_err().to_string();
        assert!(e.contains("models/m.glb"), "{e}");
    }
}
