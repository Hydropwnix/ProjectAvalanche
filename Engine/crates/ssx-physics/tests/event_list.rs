use std::path::Path;
use std::sync::Arc;

use ssx_assets::content::dlc::set_active_roots;
use ssx_assets::data_roots::{TunerVault, set_tuner};
use ssx_formats::attrib::fixtures::{ui_drop_schema_pair, ui_drops_pair};
use ssx_formats::attrib::fnv1;

fn write_pair(dir: &Path, stem: &str, (vlt, bin): (Vec<u8>, Vec<u8>)) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{stem}.vlt")), vlt).unwrap();
    std::fs::write(dir.join(format!("{stem}.bin")), bin).unwrap();
}

fn info(name: &str) -> Vec<u8> {
    let mut v = b"DLC_INFO".to_vec();
    v.push(2);
    let string = |v: &mut Vec<u8>, s: &str| {
        v.push(s.len() as u8);
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    };
    string(&mut v, name);
    v.extend_from_slice(&[1, 0]);
    string(&mut v, "Track Content");
    v.push(2);
    for (k, val) in [("vaultname", name), ("bigname", name)] {
        string(&mut v, k);
        string(&mut v, val);
    }
    v.extend_from_slice(&[1, 0]);
    string(&mut v, "SSX Test");
    string(&mut v, "a test package");
    v
}

fn ride_drops(big: &Path, names: &[&str]) -> Vec<Option<u32>> {
    let events = ssx_physics::event::load_events(big).expect("the ride's events load");
    names.iter().map(|n| events.iter().find(|e| e.key == fnv1(n.as_bytes())).map(|e| e.drop_key)).collect()
}

#[test]
fn the_rides_events_are_the_runtime_databases() {
    let dir = std::env::temp_dir().join(format!("ssx-event-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let big = dir.join("big");
    let disc = big.join("bootcache/data/xenon/attribdb");
    write_pair(&disc, "attribdb_schema", ui_drop_schema_pair());
    let drops = [("a", None, Some(0x1111)), ("b", None, Some(0x2222)), ("e", Some("b"), None)];
    write_pair(&disc, "attribdb", ui_drops_pair(&drops));
    write_pair(&disc, "ge_attribdb", ui_drops_pair(&[]));
    let (vlt, bin) = ui_drops_pair(&[("b", None, Some(0x6666))]);
    set_tuner(Some(TunerVault { vlt: Arc::from(vlt), bin: Arc::from(bin), version: 4, source: "test".into() }));
    let package = dir.join("dlc/dlc_p");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("info.dlc"), info("dlc_p")).unwrap();
    let delta = package.join("data/xenon/attribdb");
    write_pair(&delta, "dlc_p", ui_drops_pair(&[("a", None, Some(0x3333)), ("c", None, Some(0x4444))]));
    write_pair(&delta, "dlc_other", ui_drops_pair(&[("c", None, Some(0x7777)), ("f", None, Some(0x8888))]));
    set_active_roots(vec![package.clone()]);

    let want = [Some(0x3333), Some(0x6666), Some(0x4444), Some(0x6666), None];
    assert_eq!(ride_drops(&big, &["a", "b", "c", "e", "f"]), want);

    write_pair(
        &delta,
        "dlc_p",
        ui_drops_pair(&[("a", None, Some(0x3333)), ("c", None, Some(0x4444)), ("d", Some("b"), None)]),
    );
    set_active_roots(vec![package]);
    assert_eq!(ride_drops(&big, &["a", "b", "c", "d"]), [Some(0x3333), Some(0x6666), Some(0x4444), Some(0x6666)]);

    set_active_roots(Vec::new());
    set_tuner(None);
    let _ = std::fs::remove_dir_all(&dir);
}
