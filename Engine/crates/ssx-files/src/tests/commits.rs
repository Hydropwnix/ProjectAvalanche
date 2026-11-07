use std::io::{self, Write};

use super::{Scratch, failing_at};
use crate::{Error, Options, Step, committing, prepare_with, replace};

fn prepared(path: &std::path::Path, bytes: &[u8]) -> crate::Prepared {
    prepare_with(path, &Options::new(), |f| f.write_all(bytes).map_err(|e| Error::new(path, e))).unwrap()
}

#[test]
fn a_prepared_file_replaces_its_target_on_commit_and_nothing_when_dropped() {
    let s = Scratch::new("prepared");
    let path = s.join("member.bin");
    replace(&path, b"old", &Options::new()).unwrap();
    let p = prepared(&path, b"new");
    assert_eq!(p.target(), path);
    assert_eq!(std::fs::read(&path).unwrap(), b"old", "not before the commit");
    assert_eq!(s.names(), ["member.bin", "member.bin.partial"]);
    let q = prepared(&path, b"other");
    assert_eq!(s.names(), ["member.bin", "member.bin.partial", "member.bin.partial-1"]);
    drop(q);
    p.commit().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(s.names(), ["member.bin"]);
    drop(prepared(&path, b"dropped"));
    assert_eq!(std::fs::read(&path).unwrap(), b"new");
    assert_eq!(s.names(), ["member.bin"]);
}

#[test]
fn a_failure_while_preparing_leaves_the_target_and_no_partial_file() {
    let s = Scratch::new("prepare-fail");
    let path = s.join("member.bin");
    replace(&path, b"old", &Options::new()).unwrap();
    let e = failing_at(Step::Write, || prepare_with(&path, &Options::new(), |_| Ok::<(), Error>(()))).err().unwrap();
    assert_eq!(e.path(), s.join("member.bin.partial"));
    let own = prepare_with(&path, &Options::new(), |f| {
        f.write_all(b"half").map_err(|e| Error::new(&path, e))?;
        Err(Error::new(&path, io::Error::other("the source ended early")))
    });
    assert!(own.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    assert_eq!(s.names(), ["member.bin"]);
}

#[test]
fn every_file_handed_to_the_committing_threads_is_in_place_when_they_return() {
    let s = Scratch::new("committing");
    let count = committing(4, |c| {
        for i in 0..200 {
            let path = s.join(&format!("f{i:03}"));
            c.commit(prepared(&path, format!("file {i}").as_bytes()))?;
        }
        Ok::<_, Error>(200)
    })
    .unwrap();
    assert_eq!(count, 200);
    let names = s.names();
    assert_eq!(names.len(), 200, "no partial file left: {names:?}");
    for i in 0..200 {
        assert_eq!(std::fs::read(s.join(&format!("f{i:03}"))).unwrap(), format!("file {i}").as_bytes());
    }
}

#[test]
fn a_failed_commit_stops_the_writer_and_leaves_whole_files_only() {
    let s = Scratch::new("commit-fail");
    let blocked = s.join("f050");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("inside"), b"x").unwrap();
    let mut handed = 0;
    let e = committing(4, |c| {
        for i in 0..400 {
            c.commit(prepared(&s.join(&format!("f{i:03}")), format!("file {i}").as_bytes()))?;
            handed += 1;
        }
        Ok::<_, Error>(())
    })
    .unwrap_err();
    assert_eq!(e.path(), blocked, "{e}");
    assert!(handed < 400, "the writer was stopped");
    assert!(blocked.join("inside").is_file());
    for name in s.names() {
        assert!(!name.contains(".partial"), "{name} left");
        if name != "f050" {
            let i: usize = name[1..].parse().unwrap();
            assert_eq!(std::fs::read(s.join(&name)).unwrap(), format!("file {i}").as_bytes());
        }
    }
}

#[test]
fn the_writers_error_is_returned_and_nothing_partial_is_left() {
    let s = Scratch::new("commit-writer-fails");
    let e = committing(2, |c| {
        for i in 0..50 {
            c.commit(prepared(&s.join(&format!("f{i:02}")), b"whole"))?;
        }
        Err::<(), _>(Error::new(s.join("source"), io::Error::other("cancelled")))
    })
    .unwrap_err();
    assert_eq!(e.path(), s.join("source"));
    for name in s.names() {
        assert!(!name.contains(".partial"), "{name} left");
        assert_eq!(std::fs::read(s.join(&name)).unwrap(), b"whole");
    }
}
