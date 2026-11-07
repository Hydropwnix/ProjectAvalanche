use super::*;
use crate::test_dir::TestDir;

#[test]
fn package_paths() {
    for ok in ["a.png", "textures/Neon_Board-2.d.png", "data/x+y.toml", ".hidden/a.toml", "a/b/c/d.glb"] {
        assert!(check_package_path(ok, false).is_ok(), "{ok}");
    }
    for ok in ["client/*.luau", "server/data/**", "ui/icon_?.png", "ui/[ab].png"] {
        assert!(check_package_path(ok, true).is_ok(), "{ok}");
    }
    for (bad, why) in [
        ("", "empty"),
        ("/etc/passwd", "absolute"),
        ("../a.png", "\"..\" segment"),
        ("a/./b.png", "\".\" segment"),
        ("a//b.png", "\"\" segment"),
        ("a/", "\"\" segment"),
        ("a\\b.png", "use `/`"),
        ("C:/a.png", "contains ':'"),
        ("a b.png", "contains ' '"),
        ("a\u{0}.png", "contains"),
        ("ä.png", "contains 'ä'"),
        ("a.png.", "ending in '.'"),
        ("con.png", "reserved device name"),
        ("x/NUL", "reserved device name"),
        ("Lpt1.txt", "reserved device name"),
        ("t/*.png", "names one file"),
    ] {
        let e = check_package_path(bad, false).unwrap_err();
        assert!(e.contains(why), "{bad:?}: {e}");
    }
    assert!(check_package_path(&"a/".repeat(300), false).unwrap_err().contains("bytes"));
    assert!(check_package_path(&"a".repeat(129), false).unwrap_err().contains("segment over"));
    assert!(check_package_path("console.png", false).is_ok() && check_package_path("com10.png", false).is_ok());
}

#[test]
fn reads_stay_inside_the_package() {
    let dir = TestDir::new("path-read");
    dir.write("pkg/textures/Board.png", "png");
    dir.write("pkg/big.bin", &"x".repeat(100));
    dir.write("pkg/a b.png", "not a portable name");
    dir.write("outside/secret.txt", "secret");
    let root = dir.0.join("pkg");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.0.join("outside/secret.txt"), root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(dir.0.join("outside"), root.join("linkdir")).unwrap();
    }
    let tree = PackageTree::open(&root).unwrap();
    assert_eq!(tree.read("textures/Board.png", 10).unwrap(), b"png");
    let e = tree.read("textures/board.png", 10).unwrap_err().to_string();
    assert!(e.contains("case-sensitive"), "{e}");
    assert!(tree.read("textures", 10).unwrap_err().to_string().contains("not a regular file"));
    assert!(tree.read("textures/Board.png/x", 10).unwrap_err().to_string().contains("no entry"));
    assert!(tree.read("missing.png", 10).unwrap_err().to_string().contains("no entry"));
    assert!(tree.read("big.bin", 99).unwrap_err().to_string().contains("100 bytes (at most 99)"));
    assert_eq!(tree.read("big.bin", 100).unwrap().len(), 100);
    assert!(matches!(tree.read("../outside/secret.txt", 99), Err(ReadError::BadPath(_))));
    assert_eq!((tree.kind("a b.png"), tree.unnamed), (None, 1), "unportable names are counted, not indexed");
    #[cfg(unix)]
    {
        assert_eq!(tree.kind("link.txt"), Some(EntryKind::Link));
        assert_eq!(tree.kind("linkdir"), Some(EntryKind::Link));
        assert_eq!(tree.kind("linkdir/secret.txt"), None, "a linked folder is not walked");
        assert!(tree.read("link.txt", 99).unwrap_err().to_string().contains("is a link"));
    }
    let mut files: Vec<&str> = tree.files().collect();
    files.sort_unstable();
    assert_eq!(files, ["big.bin", "textures/Board.png"]);
}

#[cfg(unix)]
#[test]
fn swaps_after_opening_are_refused() {
    let dir = TestDir::new("path-swap");
    dir.write("pkg/a.png", "a");
    dir.write("pkg/sub/b.png", "b");
    dir.write("pkg/c.png", "c");
    dir.write("outside/secret.txt", "secret");
    let root = dir.0.join("pkg");
    let tree = PackageTree::open(&root).unwrap();
    std::fs::remove_file(root.join("a.png")).unwrap();
    std::os::unix::fs::symlink(dir.0.join("outside/secret.txt"), root.join("a.png")).unwrap();
    let e = tree.read("a.png", 99).unwrap_err().to_string();
    assert!(e.contains("without following links"), "{e}");
    std::fs::remove_dir_all(root.join("sub")).unwrap();
    std::os::unix::fs::symlink(dir.0.join("outside"), root.join("sub")).unwrap();
    dir.write("outside/b.png", "outside");
    let e = tree.read("sub/b.png", 99).unwrap_err().to_string();
    assert!(e.contains("as a folder without following links"), "{e}");
    std::fs::remove_file(root.join("c.png")).unwrap();
    let fifo = std::ffi::CString::new(root.join("c.png").into_os_string().into_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let start = std::time::Instant::now();
    let e = tree.read("c.png", 99).unwrap_err().to_string();
    assert!(e.contains("not a regular file"), "{e}");
    assert!(start.elapsed() < std::time::Duration::from_secs(1), "the FIFO did not block the read");
}

#[test]
fn package_trees_are_bounded() {
    let dir = TestDir::new("path-bounds");
    let deep = (0..MAX_PACKAGE_DEPTH).map(|i| format!("d{i}")).collect::<Vec<_>>().join("/");
    dir.write(&format!("pkg/{deep}/x.png"), "x");
    let e = PackageTree::open(&dir.0.join("pkg")).err().unwrap().to_string();
    assert!(e.contains("nested deeper than 32"), "{e}");
    let wide = dir.0.join("wide");
    std::fs::create_dir_all(&wide).unwrap();
    for i in 0..=MAX_PACKAGE_ENTRIES {
        std::fs::File::create(wide.join(format!("{i}.png"))).unwrap();
    }
    let e = PackageTree::open(&wide).err().unwrap().to_string();
    assert!(e.contains("more than 65536 entries"), "{e}");
}
