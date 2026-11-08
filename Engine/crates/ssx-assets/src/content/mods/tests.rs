use super::*;
use ssx_content::texture::{Texture, Want, resolve};
use ssx_content::{Asset, ContentId, MemorySource};

struct App(std::path::PathBuf);

impl App {
    fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("ssx-mods-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join(MODS_DIR)).unwrap();
        App(p)
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn package(&self, folder: &str, name: &str, priority: i32, v: u8, extra: &str) {
        let manifest = format!(
            "[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\npriority = {priority}\n{extra}\n[override]\n\
             \"ssx:texture/worlds/textures/rock\" = \"t/rock.png\"\n"
        );
        self.write(&format!("{MODS_DIR}/{folder}/resource.toml"), manifest.as_bytes());
        self.write(&format!("{MODS_DIR}/{folder}/t/rock.png"), &png1(v));
    }

    fn list(&self, entries: &[(&str, bool)]) {
        let rows: String =
            entries.iter().map(|(f, e)| format!("[[mods]]\nfolder = \"{f}\"\nenabled = {e}\n\n")).collect();
        self.write(ENABLED_FILE, format!("format = 1\n\n{rows}").as_bytes());
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn png1(v: u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, 1, 1);
    e.set_color(png::ColorType::Rgba);
    let mut w = e.write_header().unwrap();
    w.write_image_data(&[v, v, v, 255]).unwrap();
    w.finish().unwrap();
    out
}

fn rock() -> ContentId {
    "ssx:texture/worlds/textures/rock".parse().unwrap()
}

fn registry() -> Registry {
    let t = Texture {
        shape: ssx_content::texture::Shape::Plane,
        levels: vec![ssx_content::texture::Level { width: 1, height: 1, images: vec![vec![1, 1, 1, 255]] }],
        blocks: None,
        kind: None,
        source: None,
    };
    let mut r = Registry::new();
    r.mount(Layer::Base, 0, Box::new(MemorySource::new("ssx").with(rock(), Asset::Value(std::sync::Arc::new(t)))));
    r
}

fn texel(r: &Registry) -> (u8, String) {
    let found = resolve(r, &rock(), &Want::PLANE, |_| {}).unwrap();
    (found.texture.levels[0].images[0][0], found.source)
}

#[test]
fn no_list_mounts_nothing() {
    let app = App::new("none");
    let mut r = registry();
    let report = mount_enabled(&mut r, &app.0);
    assert!(report.mounted.is_empty() && report.list_error.is_none() && report.skipped.is_empty());
    assert_eq!(texel(&r), (1, "ssx".into()));
}

#[test]
fn enabled_packages_mount_in_order_with_priorities() {
    let app = App::new("order");
    app.package("a", "pack-a", 0, 10, "");
    app.package("b", "pack-b", 0, 20, "");
    app.package("c", "pack-c", 0, 30, "");
    app.list(&[("a", true), ("b", true), ("c", false)]);
    let mut r = registry();
    let report = mount_enabled(&mut r, &app.0);
    let names: Vec<&str> = report.mounted.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["pack-a", "pack-b"]);
    assert_eq!(texel(&r), (20, "pack-b".into()));
    let c = &report.registry.conflicts;
    assert_eq!((c.len(), c[0].winner.as_str(), c[0].losers.clone()), (1, "pack-b", vec!["pack-a".to_string()]));
    assert!(
        report
            .lines()
            .iter()
            .any(|l| l == "mods: conflict on ssx:texture/worlds/textures/rock: pack-b wins over pack-a")
    );
    app.package("a", "pack-a", 5, 10, "");
    let mut r = registry();
    mount_enabled(&mut r, &app.0);
    assert_eq!(texel(&r), (10, "pack-a".into()));
}

#[test]
fn broken_packages_are_skipped_with_their_reason() {
    let app = App::new("broken");
    app.package("good", "good", 0, 40, "");
    app.write(&format!("{MODS_DIR}/badmanifest/resource.toml"), b"[resource]\nnme = \"x\"\n");
    app.package("needs", "needs", 0, 50, "[dependencies]\nrequire = [\"absent >= 1.0\"]\n");
    app.package("twin1", "twin", 0, 60, "");
    app.package("twin2", "twin", 0, 70, "");
    app.package("future", "future", 0, 80, "");
    let m = std::fs::read_to_string(app.0.join(MODS_DIR).join("future/resource.toml")).unwrap();
    app.write(&format!("{MODS_DIR}/future/resource.toml"), m.replace("api = 1", "api = 9").as_bytes());
    app.write(&format!("{MODS_DIR}/pack.zip"), b"PK\x03\x04");
    app.list(&[
        ("good", true),
        ("badmanifest", true),
        ("needs", true),
        ("twin1", true),
        ("twin2", true),
        ("future", true),
        ("pack.zip", true),
        ("absent", true),
    ]);
    let mut r = registry();
    let report = mount_enabled(&mut r, &app.0);
    assert_eq!(report.mounted.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["good"]);
    let reason = |f: &str| report.skipped.iter().find(|k| k.folder == f).map(|k| k.reason.clone()).unwrap();
    assert!(reason("badmanifest").contains("resource.toml:2:1"), "{}", reason("badmanifest"));
    assert!(reason("needs").contains("requires absent"), "{}", reason("needs"));
    assert!(reason("twin1").contains("two packages are named twin"));
    assert!(reason("twin2").contains("two packages are named twin"));
    assert!(reason("future").contains("API level 9"));
    assert!(reason("pack.zip").contains(".zip package"));
    assert!(reason("absent").contains("absent"));
    assert_eq!(report.skipped.len(), 7);
    assert_eq!(texel(&r), (40, "good".into()));
    assert!(report.summary().contains("1 mounted (good), 7 skipped"), "{}", report.summary());
}

#[test]
fn a_hostile_list_mounts_nothing() {
    for (text, says) in [
        ("format = 1\n[[mods]]\nfolder = \"../escape\"\nenabled = true\n", "not a folder name"),
        ("format = 1\n[[mods]]\nfolder = \"..\"\nenabled = true\n", "segment"),
        ("format = 1\n[[mods]]\nfolder = \"con\"\nenabled = true\n", "reserved device name"),
        ("format = 1\n[[mods]]\nfolder = \"a\"\nenabled = true\n[[mods]]\nfolder = \"A\"\nenabled = false\n", "twice"),
        ("format = 2\n", "format 2"),
        ("format = 1\n[[mods]]\nfolder = \"a\"\nenabled = \"yes\"\n", "mods.toml:4:"),
        ("format = 1\nextra = 1\n", "mods.toml:2:"),
        ("format = 1\n[[mods]]\nfolder = \"a\"\n", "enabled"),
    ] {
        let app = App::new("hostile");
        app.package("a", "pack-a", 0, 10, "");
        app.write(ENABLED_FILE, text.as_bytes());
        let mut r = registry();
        let report = mount_enabled(&mut r, &app.0);
        let e = report.list_error.clone().unwrap_or_default();
        assert!(e.contains(says), "{text:?}: {e}");
        assert!(report.mounted.is_empty());
        assert_eq!(texel(&r), (1, "ssx".into()));
    }
    let app = App::new("big");
    app.write(ENABLED_FILE, &vec![b'#'; MAX_ENABLED_BYTES as usize + 1]);
    assert!(mount_enabled(&mut registry(), &app.0).list_error.unwrap().contains("bytes"));
}

#[test]
fn the_list_round_trips() {
    let l = EnabledList {
        entries: vec![
            EnabledEntry { folder: "a".into(), enabled: true },
            EnabledEntry { folder: "b-2".into(), enabled: false },
        ],
    };
    assert_eq!(EnabledList::parse(l.to_toml().as_bytes(), "m").unwrap(), l);
}

#[test]
fn a_hand_placed_package_over_the_caps_is_refused_at_the_mount() {
    let app = App::new("caps");
    app.package("big", "big", 0, 7, "");
    for i in 0..6 {
        let p = app.0.join(MODS_DIR).join("big").join(format!("t/fill{i}.bin"));
        std::fs::File::create(&p).unwrap().set_len(512 << 20).unwrap();
    }
    app.package("huge", "huge", 0, 8, "");
    std::fs::File::create(app.0.join(MODS_DIR).join("huge/t/one.bin")).unwrap().set_len((512 << 20) + 1).unwrap();
    app.package("fine", "fine", 0, 9, "");
    app.list(&[("big", true), ("huge", true), ("fine", true)]);
    let mut registry = Registry::new();
    let report = mount_enabled_except(&mut registry, &app.0, None);
    let reason = |folder: &str| report.skipped.iter().find(|s| s.folder == folder).map(|s| s.reason.clone());
    assert!(reason("big").is_some_and(|r| r.contains("in all (a package is at most 2147483648)")), "{report:?}");
    assert!(reason("huge").is_some_and(|r| r.contains("a file is at most 536870912")), "{report:?}");
    assert!(reason("fine").is_none(), "{report:?}");
    assert!(registry.has(&rock()));
}

#[test]
fn routed_kinds_are_the_kinds_the_check_calls_read() {
    let read: Vec<Kind> = Kind::ALL.into_iter().filter(|k| k.read_from_packages()).collect();
    let mut routed = ROUTED_KINDS.to_vec();
    routed.sort_by_key(|k| Kind::ALL.iter().position(|a| a == k));
    assert_eq!(routed, read);
}
