use serde_json::json;

use super::*;

fn obj(v: Json) -> Map<String, Json> {
    match v {
        Json::Object(m) => m,
        _ => unreachable!("a test's object"),
    }
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssx-settings-file-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_writer_writes_only_what_it_changed() {
    let base = obj(
        json!({"vsync": true, "host": {"port": 1}, "display": {"mode": "Windowed", "window": {"width": 1280, "height": 720}}, "gone": 1}),
    );
    let ours = obj(
        json!({"vsync": false, "host": {"port": 1}, "display": {"mode": "Borderless", "window": {"width": 1600, "height": 720}}}),
    );
    let theirs = json!({"vsync": true, "player_name": "Elise", "language": "x", "gone": 1,
        "display": {"mode": "Windowed", "monitor": "DP-2", "window": {"width": 1280, "height": 1000}}});
    let mut two = obj(theirs.clone());
    merge_changes(&mut two, &base, &ours, 2);
    assert_eq!(
        Json::Object(two),
        json!({"vsync": false, "player_name": "Elise", "language": "x",
            "display": {"mode": "Borderless", "monitor": "DP-2", "window": {"width": 1600, "height": 720}}})
    );
    let mut one = obj(theirs);
    merge_changes(&mut one, &base, &ours, 1);
    assert_eq!(one["display"], ours["display"], "one level: the changed member whole");
    assert_eq!(one["player_name"], "Elise");
    let mut spoilt = obj(json!({"display": 7}));
    merge_changes(&mut spoilt, &base, &ours, 2);
    assert_eq!(spoilt["display"], ours["display"]);
}

#[test]
fn an_unreadable_file_is_never_written_over() {
    let dir = temp("unreadable");
    let path = dir.join("sub").join("settings.json");
    let m = update_object(&path, |m| {
        m.insert("vsync".into(), json!(false));
        Ok(())
    })
    .unwrap();
    assert_eq!(read_object(&path).unwrap(), m);
    let large = vec![b' '; MAX_FILE as usize + 1];
    for bytes in [&b"{\"vsync\": tru"[..], b"[1]", &large[..]] {
        std::fs::write(&path, bytes).unwrap();
        let e = update_object(&path, |_| Ok(())).unwrap_err();
        assert!(matches!(e, UpdateError::Unreadable(_)), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "left as it was");
        assert!(read_object(&path).is_err());
    }
    std::fs::write(&path, b"{\"a\": 1}").unwrap();
    assert!(matches!(update_object(&path, |_| Err("no".into())), Err(UpdateError::Failed(_))));
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\": 1}");
    let folder = dir.join("folder.json");
    std::fs::create_dir_all(&folder).unwrap();
    assert!(matches!(update_object(&folder, |_| Ok(())), Err(UpdateError::Unreadable(_))));
    assert!(folder.is_dir());
    #[cfg(unix)]
    {
        let link = dir.join("link.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(matches!(update_object(&link, |_| Ok(())), Err(UpdateError::Unreadable(_))));
        assert!(read_object(&link).is_err(), "never read through a link");
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn writers_at_the_same_moment_lose_nothing() {
    let dir = temp("writers");
    let path = dir.join("settings.json");
    std::fs::write(&path, br#"{"unknown": {"kept": true}}"#).unwrap();
    const ROUNDS: u64 = 25;
    let threads: Vec<_> = (0..6)
        .map(|t| {
            let path = path.clone();
            std::thread::spawn(move || {
                for _ in 0..ROUNDS {
                    update_object(&path, |m| {
                        let key = format!("writer{t}");
                        let n = m.get(&key).and_then(Json::as_u64).unwrap_or(0);
                        m.insert(key, json!(n + 1));
                        Ok(())
                    })
                    .unwrap();
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let m = read_object(&path).unwrap();
    for t in 0..6 {
        assert_eq!(m[&format!("writer{t}")], json!(ROUNDS), "{m:?}");
    }
    assert_eq!(m["unknown"], json!({"kept": true}));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_unreadable_file_is_kept_aside_on_the_players_yes() {
    let dir = temp("aside");
    let path = dir.join("settings.json");
    std::fs::write(&path, b"not json").unwrap();
    std::fs::write(dir.join("settings.json.broken-1"), b"older").unwrap();
    let aside = replace_unreadable(&path, obj(json!({"vsync": false}))).unwrap();
    assert_eq!(aside, dir.join("settings.json.broken-2"));
    assert_eq!(std::fs::read(&aside).unwrap(), b"not json");
    assert_eq!(std::fs::read(dir.join("settings.json.broken-1")).unwrap(), b"older");
    assert_eq!(read_object(&path).unwrap(), obj(json!({"vsync": false})));
    std::fs::remove_dir_all(&dir).unwrap();
}
