use super::*;
use crate::test_dir::{TestDir, png_rgba};
use crate::zip::testkit::{assemble, deflated, stored};
use crate::zip::{PackFile, write_zip};

fn manifest(name: &str, version: &str) -> String {
    format!(
        "[resource]\nname = \"{name}\"\nversion = \"{version}\"\napi = 1\n[override]\n\
         \"ssx:texture/worlds/textures/rock\" = \"textures/rock.png\"\n"
    )
}

fn files(name: &str, version: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        ("resource.toml".into(), manifest(name, version).into_bytes()),
        ("textures/rock.png".into(), png_rgba(4, 4, [1, 2, 3, 255])),
    ]
}

fn packed(mut files: Vec<(String, Vec<u8>)>, index: bool) -> Vec<u8> {
    if index {
        let mut idx = ContentIndex::default();
        for (n, b) in &files {
            idx.entries.insert(n.clone(), (b.len() as u64, ContentHash::of(b)));
        }
        files.push((INDEX_FILE.into(), idx.to_tsv().into_bytes()));
    }
    files.sort();
    let mut out = Vec::new();
    write_zip(&mut out, files.into_iter().map(|(name, bytes)| Ok(PackFile { name, bytes })), &ZipLimits::default())
        .unwrap();
    out
}

fn listing(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            out.push(p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"));
            if e.file_type().unwrap().is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

fn try_install(zip: &[u8]) -> (TestDir, Result<Installed, InstallError>) {
    let t = TestDir::new("install");
    t.write_bytes("in/package.zip", zip);
    std::fs::create_dir_all(t.0.join("Mods")).unwrap();
    let before = listing(&t.0);
    let r = install_zip(&t.0.join("in/package.zip"), &t.0.join("Mods"), false);
    if r.is_err() {
        assert_eq!(listing(&t.0), before, "a failed install leaves nothing behind");
    }
    (t, r)
}

fn refused_with(zip: Vec<u8>, want: &str) {
    let (_t, r) = try_install(&zip);
    let e = r.expect_err("refused").to_string();
    assert!(e.contains(want), "{e:?} lacks {want:?}");
}

#[test]
fn a_packed_archive_installs_under_its_name() {
    let (t, r) = try_install(&packed(files("neon-pack", "1.0.0"), true));
    let done = r.unwrap();
    assert_eq!((done.folder.as_str(), done.files, done.indexed), ("neon-pack", 2, true));
    assert_eq!(
        listing(&t.0.join("Mods")),
        ["neon-pack", "neon-pack/resource.toml", "neon-pack/textures", "neon-pack/textures/rock.png"]
    );
    let wrapped: Vec<(String, Vec<u8>)> =
        files("other", "1.0.0").into_iter().map(|(n, b)| (format!("other-1.0/{n}"), b)).collect();
    let (t, r) = try_install(&packed(wrapped, false));
    assert!(!r.unwrap().indexed);
    assert!(t.0.join("Mods/other/textures/rock.png").is_file());
}

#[test]
fn an_installed_name_is_replaced_only_when_asked() {
    let t = TestDir::new("install-replace");
    let mods = t.0.join("Mods");
    t.write_bytes("a.zip", &packed(files("neon-pack", "1.0.0"), true));
    t.write_bytes("b.zip", &packed(files("neon-pack", "1.1.0"), true));
    install_zip(&t.0.join("a.zip"), &mods, false).unwrap();
    let e = install_zip(&t.0.join("b.zip"), &mods, false).unwrap_err().to_string();
    assert!(e.contains("neon-pack 1.0.0 is already installed"), "{e}");
    let done = install_zip(&t.0.join("b.zip"), &mods, true).unwrap();
    let (old, aside) = done.previous.unwrap();
    assert_eq!(old, "neon-pack");
    assert!(std::fs::read_to_string(aside.join("resource.toml")).unwrap().contains("1.0.0"));
    assert!(std::fs::read_to_string(mods.join("neon-pack/resource.toml")).unwrap().contains("1.1.0"));
}

#[test]
fn hostile_archives_are_refused_and_leave_nothing() {
    let m = manifest("p", "1.0.0");
    let png = png_rgba(4, 4, [0; 4]);
    let pkg = |extra: crate::zip::testkit::Raw| {
        assemble(b"", &[stored("resource.toml", m.as_bytes()), stored("textures/rock.png", &png), extra], b"")
    };
    refused_with(pkg(stored("../evil.toml", b"x")), "\"..\" segment");
    refused_with(pkg(stored("/tmp/evil.toml", b"x")), "absolute");
    refused_with(pkg(stored("nul.txt", b"x")), "reserved device name");
    let mut link = stored("textures/link.png", b"/etc/passwd");
    link.attr = 0o120_777 << 16;
    refused_with(pkg(link), "symbolic link");
    refused_with(pkg(stored("Textures/x.png", b"x")), "differ only in case");
    refused_with(pkg(deflated("bomb.dds", &vec![0; 4 << 20])), "decompression bomb");
    refused_with(assemble(b"", &[stored("readme.txt", b"x")], b""), "not a package");
    let bad =
        assemble(b"", &[stored("resource.toml", m.as_bytes()), stored("textures/rock.png", b"\xFF\xD8\xFF not")], b"");
    refused_with(bad, "named .png, but its content is JPEG");
}

#[test]
fn the_content_index_must_match() {
    let orig = files("p", "1.0.0");
    let mut idx = ContentIndex::default();
    for (n, b) in &orig {
        idx.entries.insert(n.clone(), (b.len() as u64, ContentHash::of(b)));
    }
    let with_index = |mut rows: Vec<(String, Vec<u8>)>| {
        rows.push((INDEX_FILE.into(), idx.to_tsv().into_bytes()));
        rows.sort();
        let mut zip = Vec::new();
        let it = rows.into_iter().map(|(name, bytes)| Ok(PackFile { name, bytes }));
        write_zip(&mut zip, it, &ZipLimits::default()).unwrap();
        zip
    };
    let mut changed = orig.clone();
    changed[1].1 = png_rgba(4, 4, [9, 9, 9, 255]);
    refused_with(with_index(changed), "differs from package-index.tsv's");
    let mut more = orig.clone();
    more.push(("textures/more.png".into(), png_rgba(1, 1, [0; 4])));
    refused_with(with_index(more), "not in its package-index.tsv");
    refused_with(with_index(vec![orig[0].clone()]), "which the archive does not hold");
}

#[test]
fn folders_with_links_are_refused() {
    let t = TestDir::new("install-folder");
    for (n, b) in files("p", "1.0.0") {
        t.write_bytes(&format!("src/{n}"), &b);
    }
    let mods = t.0.join("Mods");
    std::fs::create_dir_all(&mods).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc", t.0.join("src/textures/etc")).unwrap();
        let e = install_folder(&t.0.join("src"), &mods, false).unwrap_err().to_string();
        assert!(e.contains("a link"), "{e}");
        assert!(listing(&mods).is_empty(), "{:?}", listing(&mods));
        std::fs::remove_file(t.0.join("src/textures/etc")).unwrap();
    }
    let done = install_folder(&t.0.join("src"), &mods, false).unwrap();
    assert_eq!((done.folder.as_str(), done.files), ("p", 2));
}

#[test]
fn a_failed_first_install_leaves_no_mods_folder() {
    let t = TestDir::new("install-first");
    let bad = assemble(
        b"",
        &[stored("resource.toml", manifest("p", "1.0.0").as_bytes()), stored("textures/rock.png", b"\xFF\xD8\xFF not")],
        b"",
    );
    t.write_bytes("p.zip", &bad);
    let mods = t.0.join("Mods");
    assert!(install_zip(&t.0.join("p.zip"), &mods, false).is_err());
    assert!(!mods.exists(), "the mods folder this install made is gone again");
}

#[test]
fn a_servers_install_refuses_gpu_code_and_movies_before_its_check() {
    let t = TestDir::new("install-server");
    let mods = t.0.join("Mods");
    std::fs::create_dir_all(&mods).unwrap();
    let endless = "fn surface(input: SurfaceIn, base: Surface, d: texture_2d<f32>, s: sampler, n: texture_2d<f32>, \
                   sp: texture_2d<f32>, m: sampler) -> Surface {\n    var out = base;\n    loop { out.specular += 1.0; \
                   if out.specular > 2.0 { break; } }\n    return out;\n}\n";
    let gpu: Vec<(String, Vec<u8>)> = vec![
        (
            "resource.toml".into(),
            b"[resource]\nname = \"gpu\"\nversion = \"1.0.0\"\napi = 1\n[shaders]\nfiles = [\"shaders/*.wgsl\"]\n\
              [add]\n\"material/glow\" = \"materials/glow.toml\"\n"
                .to_vec(),
        ),
        (
            "materials/glow.toml".into(),
            b"family = \"object\"\neffect = \"object_default\"\nshader = \"shaders/glow.wgsl\"\n".to_vec(),
        ),
        ("shaders/glow.wgsl".into(), endless.as_bytes().to_vec()),
    ];
    for (n, b) in &gpu {
        t.write_bytes(&format!("gpu/{n}"), b);
    }
    t.write_bytes("gpu.zip", &packed(gpu, false));
    let declared = "a server's package with GPU code or movies is refused: it declares GPU code ([shaders]); shader \
                    code comes only from client mods the player installs";
    for r in [
        install_folder_as(&t.0.join("gpu"), &mods, false, Origin::Server),
        install_zip_as(&t.0.join("gpu.zip"), &mods, false, Origin::Server),
    ] {
        match r {
            Err(InstallError::Refused(m)) => assert_eq!(m, declared),
            r => panic!("{r:?}"),
        }
        assert!(listing(&mods).is_empty(), "{:?}", listing(&mods));
    }
    let Err(InstallError::Check(findings)) = install_folder(&t.0.join("gpu"), &mods, false) else { panic!() };
    let text: Vec<String> = findings.iter().map(ToString::to_string).collect();
    #[cfg(feature = "shader")]
    assert!(text.iter().any(|f| f.starts_with("shaders/glow.wgsl:3:") && f.contains("loops must be")), "{text:?}");
    #[cfg(not(feature = "shader"))]
    assert_eq!(text, [format!("shaders/glow.wgsl: error: {}", crate::shader::NOT_CHECKED)]);
    for (file, why) in [
        ("deep/x.spv", "deep/x.spv: GPU code; shader code comes only from client mods the player installs"),
        ("movies/intro.swf", "movies/intro.swf: a movie; movies come only from client mods the player installs"),
    ] {
        let src = t.0.join(format!("src-{}", file.replace('/', "-")));
        for (n, b) in files("quiet", "1.0.0") {
            std::fs::create_dir_all(src.join(&n).parent().unwrap()).unwrap();
            std::fs::write(src.join(n), b).unwrap();
        }
        std::fs::create_dir_all(src.join(file).parent().unwrap()).unwrap();
        std::fs::write(src.join(file), b"bytes").unwrap();
        match install_folder_as(&src, &mods, false, Origin::Server) {
            Err(InstallError::Refused(m)) => {
                assert_eq!(m, format!("a server's package with GPU code or movies is refused: {why}"));
            }
            r => panic!("{file}: {r:?}"),
        }
    }
    let bad = t.0.join("bad");
    for (n, b) in files("bad", "1.0.0") {
        std::fs::create_dir_all(bad.join(&n).parent().unwrap()).unwrap();
        std::fs::write(bad.join(n), b).unwrap();
    }
    std::fs::write(bad.join("textures/rock.png"), b"\x89PNG not").unwrap();
    assert!(matches!(install_folder_as(&bad, &mods, false, Origin::Server), Err(InstallError::Check(_))));
    std::fs::remove_file(t.0.join("src-movies-intro.swf/movies/intro.swf")).unwrap();
    let done = install_folder_as(&t.0.join("src-movies-intro.swf"), &mods, false, Origin::Server).unwrap();
    assert_eq!((done.folder.as_str(), done.files), ("quiet", 2));
}
