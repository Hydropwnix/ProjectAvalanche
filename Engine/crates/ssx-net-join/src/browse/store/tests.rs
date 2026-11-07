use super::*;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ava-servers-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn addr(t: &str) -> ServerAddress {
    ServerAddress::parse(t, None).unwrap()
}

fn entry(a: &str) -> String {
    format!("{{\"address\": \"{a}\", \"name\": \"Alps\", \"when\": 1}}")
}

fn file(favourites: &[String], recent: &[String]) -> String {
    format!("{{\"format\": 1, \"favourites\": [{}], \"recent\": [{}]}}", favourites.join(","), recent.join(","))
}

#[test]
fn the_lists_round_trip_and_keep_their_rules() {
    let d = dir("trip");
    let mut f = ServersFile::load(&d).file;
    assert!(f.favourites.is_empty() && f.recent.is_empty());
    assert_eq!(f.toggle_favourite(&addr("127.0.0.1:27450"), Some("Alps Night Riders"), 5), Ok(true));
    f.joined(&addr("[::1]:27500"), None, 6);
    for i in 0..(MAX_RECENT + 5) {
        f.joined(&addr(&format!("host{i}.example:1")), Some("Peak"), 7 + i as u64);
    }
    assert_eq!(f.recent.len(), MAX_RECENT);
    assert_eq!(f.recent[0].address.to_string(), format!("host{}.example:1", MAX_RECENT + 4));
    f.joined(&addr("host20.example:1"), Some("Peak"), 99);
    assert_eq!((f.recent.len(), f.recent[0].when), (MAX_RECENT, 99), "joined again: moved to the front, once");
    assert_eq!(f.save(&d), Ok(None));
    assert_eq!(ServersFile::load(&d).file, f);
    assert_eq!(f.toggle_favourite(&addr("127.0.0.1:27450"), None, 9), Ok(false));
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn hostile_files_are_refused_with_the_reason() {
    let good = entry("127.0.0.1:27450");
    let many: Vec<String> = (0..=MAX_FAVOURITES).map(|i| entry(&format!("10.0.{}.{}:1", i / 250, i % 250))).collect();
    let cases: Vec<(String, &str)> = vec![
        ("{".into(), "EOF"),
        (file(std::slice::from_ref(&good), &[]).replace("\"format\": 1", "\"format\": 2"), "format 2"),
        (file(std::slice::from_ref(&good), &[]).replace("\"recent\"", "\"tags\": [], \"recent\""), "unknown field"),
        (file(&[good.clone(), good.clone()], &[]), "listed twice"),
        (file(&many, &[]), "more than 100"),
        (file(&[entry("127.0.0.1")], &[]), "no port"),
        (file(&[entry("../../etc/passwd:1")], &[]), "not a host name"),
        (file(&[good.replace("Alps", "Al\\u0007ps")], &[]), "name rule"),
        (file(&[good.replace("Alps", &"N".repeat(41))], &[]), "name rule"),
        (file(&[good.replace("\"when\": 1", "\"when\": -1")], &[]), "invalid value"),
        (file(&[good.replace("}", ", \"password\": \"x\"}")], &[]), "unknown field"),
        (format!("{} {}", file(&[], &[]), "x".repeat(300 * 1024)), "more than"),
    ];
    for (text, why) in &cases {
        let e = ServersFile::parse(text.as_bytes()).expect_err(why);
        assert!(e.contains(why), "{why}: {e}");
    }
    let huge = file(&(0..100_000).map(|i| entry(&format!("h{i}.x:1"))).collect::<Vec<_>>(), &[]);
    assert!(ServersFile::parse(huge.as_bytes()).expect_err("huge").contains("more than"));
}

#[test]
fn a_broken_file_is_moved_aside_and_reported_never_replaced() {
    let d = dir("broken");
    let path = ServersFile::path(&d);
    std::fs::write(&path, "{\"format\": 1, \"favourites\": [").unwrap();
    let loaded = ServersFile::load(&d);
    let problem = loaded.problem.expect("reported");
    assert!(problem.contains("servers.json.broken-1") && problem.contains("could not be read"), "{problem}");
    assert_eq!(std::fs::read_to_string(d.join("servers.json.broken-1")).unwrap(), "{\"format\": 1, \"favourites\": [");
    assert!(!path.exists() && loaded.file.favourites.is_empty());
    std::fs::write(&path, vec![b'x'; (MAX_FILE + 10) as usize]).unwrap();
    let mut f = loaded.file;
    f.toggle_favourite(&addr("127.0.0.1:1"), None, 1).unwrap();
    let note = f.save(&d).expect("saved").expect("noted");
    assert!(note.contains("servers.json.broken-2"), "{note}");
    assert_eq!(ServersFile::load(&d).file, f);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let p = ServersFile::load(&d).problem.expect("a folder");
    assert!(p.contains("not a plain file"), "{p}");
    std::fs::remove_dir_all(&d).unwrap();
}
