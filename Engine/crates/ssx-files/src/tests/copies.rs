use std::io::Write;

use super::{Scratch, failing_at};
use crate::{Error, Identity, Options, Step, copy_new, create_with, keep_copy, open_plain, rename_over};

type Keep = fn(&std::path::Path, &std::path::Path) -> Result<(), Error>;

#[test]
fn a_rename_over_replaces_and_keeps_the_files_identity() {
    let s = Scratch::new("rename-over");
    let (from, to) = (s.join("new"), s.join("game"));
    std::fs::write(&from, b"new").unwrap();
    std::fs::write(&to, b"old").unwrap();
    let id = Identity::of(&from).unwrap().unwrap();
    rename_over(&from, &to).unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), b"new");
    assert_eq!(Identity::of(&to).unwrap(), Some(id));
    assert_eq!(s.names(), ["game"]);
    std::fs::write(&from, b"newer").unwrap();
    std::fs::create_dir(s.join("dir")).unwrap();
    std::fs::write(s.join("dir/inside"), b"x").unwrap();
    assert_eq!(rename_over(&from, &s.join("dir")).unwrap_err().path(), s.join("dir"));
    assert_eq!(std::fs::read(&from).unwrap(), b"newer");
    assert_eq!(failing_at(Step::Rename, || rename_over(&from, &to)).unwrap_err().path(), to);
    assert_eq!(std::fs::read(&to).unwrap(), b"new");
}

#[test]
fn a_file_is_kept_twice_by_a_link_or_a_copy() {
    let s = Scratch::new("keep-copy");
    let file = s.join("launcher");
    std::fs::write(&file, b"old launcher").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o750)).unwrap();
    }
    for (keep, name) in [(keep_copy as Keep, "backup"), (copy_new, "copied")] {
        let kept = s.join(name);
        keep(&file, &kept).unwrap();
        assert_eq!(std::fs::read(&kept).unwrap(), b"old launcher", "{name}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&kept).unwrap().permissions().mode() & 0o777, 0o750, "{name}");
        }
        assert_eq!(keep(&file, &kept).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists, "{name}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"old launcher");
    assert_ne!(Identity::of(&s.join("copied")).unwrap(), Identity::of(&file).unwrap());
    #[cfg(unix)]
    assert_eq!(Identity::of(&s.join("backup")).unwrap(), Identity::of(&file).unwrap());
    assert_eq!(s.names(), ["backup", "copied", "launcher"]);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&file, s.join("link")).unwrap();
        for keep in [keep_copy as Keep, copy_new] {
            assert_eq!(keep(&s.join("link"), &s.join("x")).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        }
        assert!(!s.join("x").exists());
    }
}

#[test]
fn a_failed_copy_leaves_nothing() {
    let s = Scratch::new("copy-fails");
    std::fs::write(s.join("game"), b"old game").unwrap();
    for step in [Step::Write, Step::Sync, Step::Rename] {
        assert!(failing_at(step, || copy_new(&s.join("game"), &s.join("kept"))).is_err(), "{step:?}");
        assert_eq!(s.names(), ["game"], "{step:?}");
    }
}

#[test]
fn a_new_file_from_a_writer_is_whole_or_absent() {
    let s = Scratch::new("create-with");
    let path = s.join("journal");
    create_with(&path, &Options::owner_only(), |f| f.write_all(b"state 1").map_err(|e| Error::new("journal", e)))
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"state 1");
    let again = create_with(&path, &Options::new(), |f| f.write_all(b"x").map_err(|e| Error::new("journal", e)));
    assert_eq!(again.unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    let own = create_with(&s.join("other"), &Options::new(), |f| {
        f.write_all(b"half").map_err(|e| Error::new("other", e))?;
        Err(Error::new("other", std::io::Error::other("the writer stopped")))
    });
    assert!(own.is_err());
    assert_eq!(s.names(), ["journal"]);
}

#[test]
fn identities_and_plain_files() {
    let s = Scratch::new("identity");
    std::fs::write(s.join("f"), b"x").unwrap();
    let id = Identity::of(&s.join("f")).unwrap().unwrap();
    let open = open_plain(&s.join("f")).unwrap().expect("a plain file");
    assert_eq!(Identity::of_file(&open).unwrap(), id);
    std::fs::remove_file(s.join("f")).unwrap();
    std::fs::write(s.join("f"), b"y").unwrap();
    assert_ne!(Identity::of(&s.join("f")).unwrap(), Some(Identity::of_file(&open).unwrap()));
    assert_eq!(Identity::of(&s.join("missing")).unwrap(), None);
    assert_eq!(open_plain(&s.join("missing")).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    std::fs::create_dir(s.join("d")).unwrap();
    assert_eq!(Identity::of(&s.join("d")).unwrap(), None);
    assert!(open_plain(&s.join("d")).unwrap().is_none());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(s.join("f"), s.join("link")).unwrap();
        assert_eq!(Identity::of(&s.join("link")).unwrap(), None);
        assert!(open_plain(&s.join("link")).unwrap().is_none());
    }
}
