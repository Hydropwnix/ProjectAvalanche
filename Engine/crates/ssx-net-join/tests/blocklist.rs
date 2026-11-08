use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ssx_content::ContentHash;
use ssx_content::blocklist::{Follow, cache, load, now_secs};
use ssx_net_join::blocklist::{Refreshed, join, refresh};
use ssx_net_proto::master::{BLOCKLIST_VERSION, Blocklist};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn temp(tag: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ssx-net-join-blocklist-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).expect("a temporary folder");
    p
}

fn hex(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn master(dir: &Path, resources: &[String]) -> (avalanche_master::MasterThread, String) {
    let file = dir.join("blocked-resources.txt");
    let mut text = String::from("Project Avalanche blocked resources v1\n");
    for r in resources {
        text += &format!("{r} a reason the master keeps to itself\n");
    }
    std::fs::write(&file, text).expect("the master's file");
    let mut config = avalanche_master::config::Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.blocked_resources = Some(file);
    let m = avalanche_master::spawn(avalanche_master::MasterOptions {
        config,
        data_dir: dir.to_path_buf(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    })
    .expect("a master");
    let url = format!("http://127.0.0.1:{}", m.started.addr.port());
    (m, url)
}

fn read_head(s: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

fn fake(serve: impl Fn(&str, &mut TcpStream) + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let addr = listener.local_addr().expect("addr");
    let serve = Arc::new(serve);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let serve = Arc::clone(&serve);
            std::thread::spawn(move || {
                let mut s = stream;
                let head = read_head(&mut s);
                serve(&head, &mut s);
            });
        }
    });
    format!("http://{addr}")
}

fn answer(s: &mut TcpStream, status: &str, extra: &str, body: &[u8]) {
    let h = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len());
    let _ = s.write_all(h.as_bytes());
    let _ = s.write_all(body);
}

fn follow(dir: &Path, masters: &[&str]) -> Follow {
    Follow::Masters { dir: dir.to_path_buf(), masters: masters.iter().map(|m| m.to_string()).collect() }
}

fn h(n: u8) -> ContentHash {
    hex(n).parse().expect("a hash")
}

#[test]
fn two_masters_united_and_a_304_keeps_the_list() {
    let dir = temp("two");
    let (d1, d2) = (dir.join("m1"), dir.join("m2"));
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::create_dir_all(&d2).unwrap();
    let (m1, u1) = master(&d1, &[hex(1), hex(2)]);
    let (m2, u2) = master(&d2, &[hex(2), hex(3)]);
    let cache_dir = dir.join("Blocklists");
    let f = follow(&cache_dir, &[&u1, &u2]);
    let now = now_secs();
    let first = refresh(&f, &None, true, now);
    assert!(first.iter().all(|o| matches!(o.result, Ok(Refreshed::Kept(2)))), "{first:?}");
    let united = load(&cache_dir, &[u1.clone(), u2.clone()], now);
    assert_eq!(united.blocked.entries(), 4);
    assert_eq!(united.blocked.source_of(&h(2)), Some(u1.as_str()), "the first master that lists it");
    assert_eq!(united.blocked.source_of(&h(3)), Some(u2.as_str()));
    assert_eq!(united.blocked.source_of(&h(4)), None, "no false refusal");
    let later = now + 86_400;
    let again = refresh(&f, &None, true, later);
    assert!(again.iter().all(|o| matches!(o.result, Ok(Refreshed::Current(2)))), "{again:?}");
    let kept = cache::read(&cache_dir, &u1).unwrap().unwrap();
    assert_eq!((kept.fetched, kept.checked), (now, later));
    let quiet = refresh(&f, &None, false, later + 60);
    assert!(quiet.iter().all(|o| o.result == Ok(Refreshed::NotDue)), "{quiet:?}");
    m1.stop();
    m2.stop();
    let gone = refresh(&f, &None, true, later + 120);
    assert!(gone.iter().all(|o| o.result.is_err()), "{gone:?}");
    assert_eq!(load(&cache_dir, &[u1, u2], later + 120).blocked.entries(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_masters_are_refused_and_the_kept_list_stays() {
    let dir = temp("hostile");
    let cache_dir = dir.join("Blocklists");
    let good = |n: u8| Blocklist { blocklist_version: BLOCKLIST_VERSION, resources: vec![hex(n)] }.to_json();
    let cases: Vec<(&str, String)> = vec![
        ("oversized", fake(|_, s| answer(s, "200 OK", "", &vec![b' '; 9 << 20]))),
        ("malformed", fake(|_, s| answer(s, "200 OK", "", b"{\"blocklist_version\":1,\"resources\":["))),
        ("another version", fake(|_, s| answer(s, "200 OK", "", br#"{"blocklist_version":2,"resources":[]}"#))),
        (
            "upper case",
            fake(move |_, s| {
                let body = format!(r#"{{"blocklist_version":1,"resources":["{}"]}}"#, "AB".repeat(32));
                answer(s, "200 OK", "", body.as_bytes())
            }),
        ),
        (
            "twice",
            fake(move |_, s| {
                let body = format!(r#"{{"blocklist_version":1,"resources":["{0}","{0}"]}}"#, hex(5));
                answer(s, "200 OK", "", body.as_bytes())
            }),
        ),
        ("unknown member", fake(|_, s| answer(s, "200 OK", "", br#"{"blocklist_version":1,"resources":[],"x":1}"#))),
        ("304 without a list", fake(|_, s| answer(s, "304 Not Modified", "", b""))),
        ("redirect", fake(|_, s| answer(s, "302 Found", "Location: http://127.0.0.1:1/\r\n", b""))),
        (
            "silent",
            fake(|_, _| {
                std::thread::sleep(Duration::from_secs(30));
            }),
        ),
        (
            "slow",
            fake(move |_, s| {
                let body = good(9);
                let _ = s.write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                        .as_bytes(),
                );
                for b in body {
                    if s.write_all(&[b]).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }),
        ),
    ];
    let now = now_secs();
    for (_, url) in &cases {
        ssx_content::blocklist::record(
            &cache_dir,
            url,
            ssx_content::blocklist::Answer::List { hashes: vec![h(7)], etag: None },
            now - 60,
        )
        .expect("kept");
    }
    let masters: Vec<&str> = cases.iter().map(|(_, u)| u.as_str()).collect();
    let t0 = Instant::now();
    let mut outcomes = refresh(&follow(&cache_dir, &masters[..5]), &None, true, now);
    outcomes.extend(refresh(&follow(&cache_dir, &masters[5..]), &None, true, now));
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(40), "within the bounds: {took:?}");
    for (what, url) in &cases {
        let o = outcomes.iter().find(|o| o.master == *url).expect("an outcome");
        assert!(o.result.is_err(), "{what}: {o:?}");
        let kept = cache::read(&cache_dir, url).unwrap().unwrap();
        assert_eq!(kept.hashes, vec![h(7)], "{what}: the kept list stays");
        println!("{what}: {}", o.line());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_joins_check() {
    let dir = temp("join");
    let cache_dir = dir.join("Blocklists");
    let m = "http://127.0.0.1:27900";
    let blocked = ContentHash::of(b"a blocked texture");
    ssx_content::blocklist::record(
        &cache_dir,
        m,
        ssx_content::blocklist::Answer::List { hashes: vec![blocked], etag: Some("\"t\"".into()) },
        now_secs(),
    )
    .expect("kept");
    let file = |path: &str, bytes: &[u8], lazy: bool| ssx_content::delivery::File {
        path: path.into(),
        hash: ContentHash::of(bytes),
        size: bytes.len() as u64,
        role: ssx_content::delivery::FileRole::Data,
        lazy,
    };
    let resource = |name: &str, files| ssx_content::delivery::Resource {
        name: name.into(),
        version: "1.0.0".parse().expect("a version"),
        title: String::new(),
        group: 0,
        scripts: false,
        files,
    };
    let mut manifest = ssx_content::delivery::ClientManifest {
        client_mods: ssx_content::policy::ClientMods::Presentation,
        client_scripts: false,
        mirrors: vec![],
        content_id: None,
        resources: vec![
            resource("fine", vec![file("a.png", b"fine", false)]),
            resource("skins", vec![file("ok.png", b"ok", false), file("lazy.png", b"a blocked texture", true)]),
        ],
        levels: vec![],
    };
    let store = ssx_content::delivery::FileStore::open(&dir.join("ResourceCache")).expect("a store");
    store.put(&blocked, 17, &mut &b"a blocked texture"[..]).expect("put");
    let f = follow(&cache_dir, &[m]);
    let why = join::refuse_blocked(&manifest, &f, &store).expect_err("refused");
    assert!(why.contains("\"skins\"") && why.contains("lazy.png") && why.contains(m), "{why}");
    assert!(!store.has(&blocked, 17), "the blocked file left the cache");
    assert!(join::refuse_blocked(&manifest, &Follow::Nothing, &store).is_ok(), "no master followed: no blocklist");
    manifest.resources.pop();
    assert!(join::refuse_blocked(&manifest, &f, &store).is_ok(), "no false refusal");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_first_join_asks_the_masters_before_its_check() {
    let dir = temp("first-join");
    let md = dir.join("m");
    std::fs::create_dir_all(&md).unwrap();
    let blocked = b"a texture the master blocks";
    let (m, url) = master(&md, &[ContentHash::of(blocked).to_string()]);
    let cache_dir = dir.join("Blocklists");
    let manifest = ssx_content::delivery::ClientManifest {
        client_mods: ssx_content::policy::ClientMods::Presentation,
        client_scripts: false,
        mirrors: vec![],
        content_id: None,
        resources: vec![ssx_content::delivery::Resource {
            name: "pack".into(),
            version: "1.0.0".parse().expect("a version"),
            title: String::new(),
            group: 0,
            scripts: false,
            files: vec![ssx_content::delivery::File {
                path: "t.png".into(),
                hash: ContentHash::of(blocked),
                size: blocked.len() as u64,
                role: ssx_content::delivery::FileRole::Data,
                lazy: false,
            }],
        }],
        levels: vec![],
    };
    let store = ssx_content::delivery::FileStore::open(&dir.join("ResourceCache")).expect("a store");
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let f = follow(&cache_dir, &[&url]);
    let checked = join::check(&manifest, &f, &store, &cancel);
    let why = checked.verdict.expect_err("refused at the first join");
    assert!(why.contains("\"pack\"") && why.contains(&url) && why.contains("t.png"), "{why}");
    let asked = &checked.asked;
    assert!(asked.len() == 1 && asked[0].contains(&url) && asked[0].contains("a new list"), "{asked:?}");
    m.stop();
    let checked = join::check(&manifest, &f, &store, &cancel);
    let why = checked.verdict.expect_err("refused from the kept list");
    assert!(why.contains(&url), "{why}");
    assert!(checked.asked.is_empty(), "not due: not asked, no line: {:?}", checked.asked);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_browser_marks_a_server_whose_kept_manifest_holds_a_blocked_file() {
    use ssx_net_join::resources::{CacheView, Need};
    let dir = temp("browser");
    let cache_dir = dir.join("Blocklists");
    let m = "http://127.0.0.1:27901";
    let blocked = b"a blocked skin";
    let toml = b"[package]\nname = \"skins\"\n";
    ssx_content::blocklist::record(
        &cache_dir,
        m,
        ssx_content::blocklist::Answer::List { hashes: vec![ContentHash::of(blocked)], etag: None },
        now_secs(),
    )
    .expect("kept");
    let manifest = ssx_content::delivery::ClientManifest {
        client_mods: ssx_content::policy::ClientMods::Presentation,
        client_scripts: false,
        mirrors: vec![],
        content_id: None,
        resources: vec![ssx_content::delivery::Resource {
            name: "skins".into(),
            version: "1.0.0".parse().expect("a version"),
            title: String::new(),
            group: 0,
            scripts: false,
            files: vec![
                ssx_content::delivery::File {
                    path: "resource.toml".into(),
                    hash: ContentHash::of(toml),
                    size: toml.len() as u64,
                    role: ssx_content::delivery::FileRole::Manifest,
                    lazy: false,
                },
                ssx_content::delivery::File {
                    path: "skin.png".into(),
                    hash: ContentHash::of(blocked),
                    size: blocked.len() as u64,
                    role: ssx_content::delivery::FileRole::Data,
                    lazy: false,
                },
            ],
        }],
        levels: vec![],
    };
    let body = manifest.to_json().into_bytes();
    ssx_content::delivery::from_json(&body).expect("the manifest within the reader's rules");
    let both = (toml.len() + blocked.len()) as u64;
    let hash = ContentHash::of(&body);
    let root = dir.join("ResourceCache");
    let summary = |manifest: &ContentHash| ssx_net_proto::resources::ResourceSummary {
        content_id: String::new(),
        client_mods: "presentation".into(),
        client_scripts: false,
        count: 1,
        download: both,
        manifest: manifest.to_string(),
        manifest_bytes: body.len() as u64,
    };
    let view = CacheView::new(root.clone(), follow(&cache_dir, &[m]));
    let look = view.look(&summary(&hash));
    assert_eq!(look.blocked, None);
    assert!(matches!(look.need, Some(Need::Between { .. })), "{look:?}");
    let store = ssx_content::delivery::FileStore::open(&root).expect("a store");
    store.put(&hash, body.len() as u64, &mut &body[..]).expect("put");
    let look = view.look(&summary(&hash));
    let why = look.blocked.expect("marked");
    assert!(why.contains("\"skins\"") && why.contains("skin.png") && why.contains(m), "{why}");
    assert_eq!(look.need, Some(Need::Exactly(both)));
    let look = CacheView::new(root.clone(), Follow::Nothing).look(&summary(&hash));
    assert_eq!((look.blocked, look.need), (None, Some(Need::Exactly(both))));
    assert_eq!(view.look(&summary(&ContentHash::of(b"another body"))).blocked, None);
    let _ = std::fs::remove_dir_all(&dir);
}
