use super::{Scratch, failing_at, names};
use crate::{Step, create_dir_all, install_folder, rename_new};

#[test]
fn folders_are_made_and_their_names_synced() {
    let s = Scratch::new("mkdir");
    create_dir_all(&s.join("app/mods/pack")).unwrap();
    assert!(s.join("app/mods/pack").is_dir());
    create_dir_all(&s.join("app/mods")).unwrap();
    let e = failing_at(Step::FolderSync, || create_dir_all(&s.join("app/saves"))).unwrap_err();
    assert_eq!(e.path(), s.join("app"), "{e}");
    assert!(s.join("app/saves").is_dir());
    failing_at(Step::FolderSync, || create_dir_all(&s.join("app/saves"))).expect("nothing made: nothing synced");
    std::fs::write(s.join("file"), "x").unwrap();
    assert!(create_dir_all(&s.join("file/sub")).is_err());
}

#[test]
fn a_rename_never_replaces() {
    let s = Scratch::new("rename");
    std::fs::write(s.join("save.bin"), "damaged").unwrap();
    std::fs::write(s.join("save.bin.damaged1"), "older").unwrap();
    let e = rename_new(&s.join("save.bin"), &s.join("save.bin.damaged1")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "{e}");
    assert_eq!(std::fs::read(s.join("save.bin.damaged1")).unwrap(), b"older");
    rename_new(&s.join("save.bin"), &s.join("save.bin.damaged2")).unwrap();
    assert_eq!(std::fs::read(s.join("save.bin.damaged2")).unwrap(), b"damaged");
    std::fs::create_dir_all(s.join("mods/pack/inside")).unwrap();
    std::fs::create_dir_all(s.join("mods/other")).unwrap();
    let e = rename_new(&s.join("mods/pack"), &s.join("mods/other")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "an empty folder is not replaced either: {e}");
    rename_new(&s.join("mods/pack"), &s.join("mods/.replaced-pack")).unwrap();
    assert!(s.join("mods/.replaced-pack/inside").is_dir());
    assert!(failing_at(Step::Rename, || rename_new(&s.join("save.bin.damaged2"), &s.join("x"))).is_err());
    assert_eq!(s.names(), ["mods", "save.bin.damaged1", "save.bin.damaged2"]);
}

#[test]
fn a_folder_is_installed_whole_or_not_at_all() {
    let s = Scratch::new("install");
    let staged = s.join(".install-1");
    std::fs::create_dir_all(staged.join("models/sub")).unwrap();
    std::fs::write(staged.join("resource.toml"), "[resource]\n").unwrap();
    std::fs::write(staged.join("models/sub/a.glb"), [1u8; 4096]).unwrap();
    assert!(failing_at(Step::Sync, || install_folder(&staged, &s.join("pack"))).is_err());
    assert!(failing_at(Step::FolderSync, || install_folder(&staged, &s.join("pack"))).is_err());
    assert_eq!(s.names(), [".install-1"], "nothing moved");
    install_folder(&staged, &s.join("pack")).unwrap();
    assert_eq!(names(&s.join("pack")), ["models", "resource.toml"]);
    assert_eq!(std::fs::read(s.join("pack/models/sub/a.glb")).unwrap(), [1u8; 4096]);
    std::fs::create_dir_all(s.join(".install-2")).unwrap();
    let e = install_folder(&s.join(".install-2"), &s.join("pack")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "{e}");
    assert_eq!(s.names(), [".install-2", "pack"]);
}

#[cfg(unix)]
#[test]
fn a_link_in_a_staged_folder_is_refused() {
    let s = Scratch::new("install-link");
    std::fs::create_dir_all(s.join("staged")).unwrap();
    std::os::unix::fs::symlink("/etc", s.join("staged/etc")).unwrap();
    let e = install_folder(&s.join("staged"), &s.join("pack")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{e}");
    assert_eq!(s.names(), ["staged"]);
}

#[cfg(feature = "cap-std")]
#[test]
fn a_write_inside_a_folder_handle_has_the_same_steps() {
    use crate::{Options, replace_in};
    use cap_std::fs::Dir;
    let s = Scratch::new("cap");
    let dir = Dir::open_ambient_dir(&s.0, cap_std::ambient_authority()).unwrap();
    replace_in(&dir, "storage.bin", b"one", &Options::new()).unwrap();
    replace_in(&dir, "storage.bin", b"two", &Options::new()).unwrap();
    for step in [Step::Write, Step::Sync, Step::Rename] {
        assert!(failing_at(step, || replace_in(&dir, "storage.bin", b"three", &Options::new())).is_err());
    }
    assert_eq!(std::fs::read(s.join("storage.bin")).unwrap(), b"two");
    assert_eq!(s.names(), ["storage.bin"]);
    for bad in ["", ".", "..", "a/b", "a\\b"] {
        assert!(replace_in(&dir, bad, b"x", &Options::new()).is_err(), "{bad:?}");
    }
    #[cfg(unix)]
    {
        std::fs::write(s.join("victim"), "elsewhere").unwrap();
        std::os::unix::fs::symlink(s.join("victim"), s.join("storage.bin.partial")).unwrap();
        replace_in(&dir, "storage.bin", b"four", &Options::new()).unwrap();
        assert_eq!(std::fs::read(s.join("victim")).unwrap(), b"elsewhere");
        assert_eq!(std::fs::read(s.join("storage.bin")).unwrap(), b"four");
        assert_eq!(s.names(), ["storage.bin", "storage.bin.partial", "victim"]);
    }
}
