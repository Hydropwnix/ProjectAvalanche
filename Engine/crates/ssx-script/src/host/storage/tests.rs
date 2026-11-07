use std::path::PathBuf;

use super::*;
use crate::value::{ElementId, NetValue};

const CAPS: StorageCaps = StorageCaps { keys: 8, bytes: 4096, flush_every: Duration::from_secs(1) };

fn root(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ssx-script-storage-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(t: &str) -> NetValue {
    NetValue::String(t.as_bytes().to_vec())
}

fn write(store: &Store) {
    let (dir, bytes) = store.snapshot().unwrap();
    write_synced(&dir, &bytes).unwrap();
}

#[test]
fn values_are_written_synced_and_read_back() {
    let r = root("round");
    let mut st = Store::open(&r, "pkg", &CAPS);
    assert!(st.unavailable.is_none());
    st.set("best", Some(&NetValue::Number(12.5)), &CAPS).unwrap();
    st.set("name", Some(&s("elise")), &CAPS).unwrap();
    st.set("gone", Some(&s("x")), &CAPS).unwrap();
    st.set("gone", None, &CAPS).unwrap();
    write(&st);
    assert!(!r.join("pkg/storage.bin.partial").exists(), "the partial file was renamed");
    let back = Store::open(&r, "pkg", &CAPS);
    assert_eq!(
        (back.get("best"), back.get("name"), back.get("gone")),
        (Some(NetValue::Number(12.5)), Some(s("elise")), None)
    );
    assert_eq!((back.keys(), back.bytes()), (st.keys(), st.bytes()));
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn keys_values_and_totals_are_capped() {
    let r = root("caps");
    let mut st = Store::open(&r, "pkg", &CAPS);
    for bad in [&b""[..], b"a/b", b"k\x00", &[b'k'; 65]] {
        assert!(check_key(bad).is_err(), "{bad:?}");
    }
    assert!(st.set("e", Some(&NetValue::Element(ElementId(5))), &CAPS).unwrap_err().contains("element"));
    assert!(st.set("big", Some(&NetValue::String(vec![b'x'; 70_000])), &CAPS).is_err());
    for i in 0..8 {
        st.set(&format!("k{i}"), Some(&NetValue::Bool(true)), &CAPS).unwrap();
    }
    assert!(st.set("k8", Some(&NetValue::Bool(true)), &CAPS).unwrap_err().contains("8 keys"));
    st.set("k0", Some(&NetValue::String(vec![b'x'; 3000])), &CAPS).unwrap();
    assert!(st.set("k1", Some(&NetValue::String(vec![b'x'; 2000])), &CAPS).unwrap_err().contains("at most 4096"));
    let _ = std::fs::remove_dir_all(&r);
}

#[test]
fn a_file_that_does_not_read_back_is_left_alone() {
    let r = root("corrupt");
    let mut st = Store::open(&r, "pkg", &CAPS);
    st.set("a", Some(&s("b")), &CAPS).unwrap();
    write(&st);
    let file = r.join("pkg/storage.bin");
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[10] ^= 0x40;
    std::fs::write(&file, &bytes).unwrap();
    let mut broken = Store::open(&r, "pkg", &CAPS);
    assert!(broken.unavailable.as_deref().unwrap().contains("does not read back"));
    assert!(broken.set("a", Some(&s("c")), &CAPS).unwrap_err().contains("no storage"));
    assert!(broken.snapshot().is_none(), "nothing would be written over it");
    assert_eq!(std::fs::read(&file).unwrap(), bytes);
    let _ = std::fs::remove_dir_all(&r);
}

#[cfg(unix)]
#[test]
fn links_are_never_followed() {
    let r = root("links");
    let elsewhere = root("links-elsewhere");
    std::os::unix::fs::symlink(&elsewhere, r.join("pkg")).unwrap();
    let st = Store::open(&r, "pkg", &CAPS);
    assert!(st.unavailable.as_deref().unwrap().contains("without following links"), "{:?}", st.unavailable);
    std::fs::create_dir_all(r.join("other")).unwrap();
    std::fs::write(elsewhere.join("secret"), b"x").unwrap();
    std::os::unix::fs::symlink(elsewhere.join("secret"), r.join("other/storage.bin")).unwrap();
    let st = Store::open(&r, "other", &CAPS);
    assert!(st.unavailable.as_deref().unwrap().contains("without following links"), "{:?}", st.unavailable);
    let _ = std::fs::remove_dir_all(&r);
    let _ = std::fs::remove_dir_all(&elsewhere);
}

#[test]
fn the_writer_writes_off_the_caller_and_reports_back() {
    let r = root("writer");
    let writer = Writer::new(r.clone()).unwrap();
    let mut st = Store::open(&r, "pkg", &CAPS);
    st.set("a", Some(&NetValue::Number(1.0)), &CAPS).unwrap();
    assert!(st.dirty && writer.queue("pkg", &mut st) && !st.dirty);
    assert!(writer.settled("pkg"), "the write finished within the bound");
    assert_eq!(Store::open(&r, "pkg", &CAPS).get("a"), Some(NetValue::Number(1.0)));
    st.set("a", Some(&NetValue::Number(2.0)), &CAPS).unwrap();
    assert!(writer.queue_final("pkg", &mut st));
    drop(writer);
    assert_eq!(Store::open(&r, "pkg", &CAPS).get("a"), Some(NetValue::Number(2.0)), "written before the writer went");
    let _ = std::fs::remove_dir_all(&r);
}
