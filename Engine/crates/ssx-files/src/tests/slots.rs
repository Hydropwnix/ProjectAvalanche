use std::fs::File;

use super::Scratch;
use crate::{Options, SLOTS, partial_target, replace};

#[test]
fn partial_names_map_back_to_their_file() {
    assert_eq!(partial_target("settings.json.partial"), Some("settings.json"));
    assert_eq!(partial_target("settings.json.partial-7"), Some("settings.json"));
    assert_eq!(partial_target("a.partial.partial-1"), Some("a.partial"));
    for other in ["settings.json", ".partial", "x.partial-0", "x.partial-8", "x.partial-01", "x.partial-", "x.partials"]
    {
        assert_eq!(partial_target(other), None, "{other}");
    }
    assert_eq!(SLOTS, 8);
}

#[test]
fn a_leftover_partial_file_is_removed_and_its_name_taken() {
    let s = Scratch::new("leftover");
    let path = s.join("mods.toml");
    std::fs::write(s.join("mods.toml.partial"), "half a list from a crash").unwrap();
    std::fs::write(s.join("mods.toml.partial-3"), "another one").unwrap();
    replace(&path, b"format = 1\n", &Options::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"format = 1\n");
    assert_eq!(s.names(), ["mods.toml", "mods.toml.partial-3"], "a slot not needed is left for its own writer");
}

#[test]
fn a_live_writers_partial_file_is_left_alone() {
    let s = Scratch::new("live");
    let path = s.join("servers.json");
    let mut held = Vec::new();
    for slot in 0..3 {
        let name = crate::slot::slot_name("servers.json".as_ref(), slot);
        std::fs::write(s.0.join(&name), format!("writer {slot}")).unwrap();
        let f = File::options().read(true).write(true).open(s.0.join(&name)).unwrap();
        f.try_lock().expect("the test's writer holds the lock");
        held.push(f);
    }
    replace(&path, b"[]", &Options::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    for (slot, mut f) in held.iter().enumerate() {
        let mut text = String::new();
        std::io::Seek::rewind(&mut f).unwrap();
        std::io::Read::read_to_string(&mut f, &mut text).unwrap();
        assert_eq!(text, format!("writer {slot}"));
    }
    for slot in 3..SLOTS {
        let name = crate::slot::slot_name("servers.json".as_ref(), slot);
        let f = File::create_new(s.0.join(&name)).unwrap();
        f.try_lock().unwrap();
        held.push(f);
    }
    let e = replace(&path, b"[1]", &Options::new()).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::WouldBlock, "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    drop(held);
    replace(&path, b"[2]", &Options::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"[2]");
    let mut want = vec!["servers.json".to_string()];
    want.extend((1..SLOTS).map(|k| format!("servers.json.partial-{k}")));
    assert_eq!(s.names(), want);
}

#[cfg(unix)]
#[test]
fn a_planted_link_at_a_partial_name_is_never_followed() {
    let s = Scratch::new("links");
    let path = s.join("known_servers.txt");
    std::fs::write(s.join("victim"), "the user's own file").unwrap();
    std::os::unix::fs::symlink(s.join("victim"), s.join("known_servers.txt.partial")).unwrap();
    std::os::unix::fs::symlink(s.join("nowhere"), s.join("known_servers.txt.partial-1")).unwrap();
    replace(&path, b"keys", &Options::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"keys");
    assert_eq!(std::fs::read(s.join("victim")).unwrap(), b"the user's own file");
    assert!(!s.join("nowhere").exists(), "nothing made through the dangling link");
    assert!(std::fs::symlink_metadata(s.join("known_servers.txt.partial")).unwrap().file_type().is_symlink());
    assert!(std::fs::symlink_metadata(s.join("known_servers.txt.partial-1")).unwrap().file_type().is_symlink());
}

#[cfg(unix)]
#[test]
fn permissions_are_set_at_the_creation_and_never_widened() {
    use std::os::unix::fs::PermissionsExt;
    let s = Scratch::new("modes");
    let key = s.join("identity.key");
    let r: Result<(), crate::Error> = crate::replace_with(&key, &Options::owner_only(), |f| {
        assert_eq!(f.metadata().unwrap().len(), 0, "nothing written yet");
        assert_eq!(f.metadata().unwrap().permissions().mode() & 0o077, 0, "owner-only before the first byte");
        std::io::Write::write_all(f, b"secret").map_err(|e| crate::Error::new("x", e))
    });
    r.unwrap();
    assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
    crate::create(&s.join("netcode.key"), b"k", &Options::owner_only()).unwrap();
    assert_eq!(std::fs::metadata(s.join("netcode.key")).unwrap().permissions().mode() & 0o777, 0o600);
    let settings = s.join("settings.json");
    std::fs::write(&settings, "{}").unwrap();
    std::fs::set_permissions(&settings, std::fs::Permissions::from_mode(0o600)).unwrap();
    replace(&settings, b"{\"vsync\": true}", &Options::new()).unwrap();
    assert_eq!(std::fs::metadata(&settings).unwrap().permissions().mode() & 0o077, 0);
}
