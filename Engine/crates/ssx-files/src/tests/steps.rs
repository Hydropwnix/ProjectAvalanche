use std::io::Write;

use super::{Scratch, failing_at};
use crate::{Error, Options, Step, create, replace, replace_with, update};

#[test]
fn a_replace_is_whole_and_leaves_no_partial_file() {
    let s = Scratch::new("whole");
    let path = s.join("settings.json");
    replace(&path, b"first", &Options::new()).unwrap();
    replace(&path, b"second", &Options::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"second");
    assert_eq!(s.names(), ["settings.json"]);
}

#[test]
fn a_failure_at_each_step_leaves_the_old_file_and_no_partial_file() {
    let s = Scratch::new("steps");
    let path = s.join("save.bin");
    replace(&path, b"old", &Options::new()).unwrap();
    for (step, named) in
        [(Step::Write, "save.bin.partial"), (Step::Sync, "save.bin.partial"), (Step::Rename, "save.bin")]
    {
        let e = failing_at(step, || replace(&path, b"new", &Options::new())).unwrap_err();
        assert_eq!(e.path(), s.join(named), "{step:?}: {e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"old", "{step:?}");
        assert_eq!(s.names(), ["save.bin"], "{step:?}");
    }
    let e = failing_at(Step::FolderSync, || replace(&path, b"new", &Options::new())).unwrap_err();
    assert_eq!(e.path(), s.0);
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(s.names(), ["save.bin"]);
}

#[test]
fn a_writer_error_or_a_refused_rename_leaves_everything_as_it_was() {
    let s = Scratch::new("natural");
    let path = s.join("pack.zip");
    replace(&path, b"old", &Options::new()).unwrap();
    let r: Result<(), std::io::Error> = replace_with(&path, &Options::new(), |f| {
        f.write_all(b"half")?;
        Err(std::io::Error::other("the writer stops"))
    });
    assert_eq!(r.unwrap_err().to_string(), "the writer stops");
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    let folder = s.join("taken");
    std::fs::create_dir_all(folder.join("inside")).unwrap();
    let e = replace(&folder, b"x", &Options::new()).unwrap_err();
    assert_eq!(e.path(), folder);
    assert!(folder.join("inside").is_dir(), "the folder in the way is untouched");
    assert_eq!(s.names(), ["pack.zip", "taken"]);
    let missing = s.join("no-folder").join("x");
    assert!(replace(&missing, b"x", &Options::new()).is_err(), "a missing folder is the caller's to make");
}

#[test]
fn a_created_file_never_replaces_one_and_is_whole_or_absent() {
    let s = Scratch::new("create");
    let path = s.join("identity.key");
    for step in [Step::Write, Step::Sync, Step::Rename] {
        assert!(failing_at(step, || create(&path, b"key", &Options::owner_only())).is_err(), "{step:?}");
        assert!(s.names().is_empty(), "{step:?}: {:?}", s.names());
    }
    create(&path, b"key", &Options::owner_only()).unwrap();
    let e = create(&path, b"other", &Options::owner_only()).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), b"key");
    std::fs::create_dir(s.join("a-folder")).unwrap();
    let e = create(&s.join("a-folder"), b"x", &Options::new()).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists, "a folder at the name: {e}");
    assert_eq!(s.names(), ["a-folder", "identity.key"]);
}

#[test]
fn an_update_changes_the_file_as_read_or_nothing() {
    let s = Scratch::new("update");
    let path = s.join("exports.tsv");
    let add = |line: &'static str| {
        move |old: Option<Vec<u8>>| -> Result<Vec<u8>, Error> {
            let mut text = old.unwrap_or_else(|| b"header\n".to_vec());
            text.extend_from_slice(line.as_bytes());
            Ok(text)
        }
    };
    update(&path, &Options::new(), 1024, add("a\n")).unwrap();
    update(&path, &Options::new(), 1024, add("b\n")).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"header\na\nb\n");
    for step in [Step::Write, Step::Sync, Step::Rename] {
        assert!(failing_at(step, || update(&path, &Options::new(), 1024, add("c\n"))).is_err(), "{step:?}");
    }
    let refused: Result<(), String> = update(&path, &Options::new(), 1024, |_| Err("refused".to_string()));
    assert_eq!(refused.unwrap_err(), "refused");
    let e = update(&path, &Options::new(), 4, add("d\n")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "a file over the cap: {e}");
    assert_eq!(std::fs::read(&path).unwrap(), b"header\na\nb\n");
    assert_eq!(s.names(), ["exports.tsv"]);
}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}
