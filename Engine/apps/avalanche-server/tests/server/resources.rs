use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avalanche_server::{ServerOptions, ServerThread};
use ssx_content::ContentHash;
use ssx_content::delivery::{ClientManifest, FileStore, Mirror};
use ssx_net_join::resources::{Progress, ResourceOptions, fetch_all, fetch_manifest};

use crate::common::{LOCAL, get, post, raw, temp_dir};

fn package(root: &Path, name: &str, files: &[(&str, Vec<u8>)]) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = format!("[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\n");
    std::fs::write(dir.join("resource.toml"), manifest).unwrap();
    for (path, bytes) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

fn bytes(tag: u8, n: usize) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(tag)).collect()
}

fn server(folder: &Path, start: &[&str], mirrors: Vec<Mirror>) -> (ServerThread, std::io::PipeWriter) {
    let mut config = crate::common::defaults();
    config.bind = std::net::Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.resources.folder = folder.to_path_buf();
    config.resources.start = start.iter().map(|s| s.to_string()).collect();
    config.resources.mirrors = mirrors;
    let (input, writer) = std::io::pipe().unwrap();
    let options = ServerOptions {
        config,
        data_dir: temp_dir("resources-data"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: Some(Box::new(move |app| {
            avalanche_server::console::attach(app, input, false).expect("the console's thread");
        })),
        rules: avalanche_server::rules::RulesSource::Unchecked,
    };
    (avalanche_server::spawn(options).expect("the server starts"), writer)
}

fn manifest(addr: SocketAddr) -> ClientManifest {
    fetch_manifest(addr, &AtomicBool::new(false)).expect("the manifest").expect("the server has resources").manifest
}

fn file_of<'a>(m: &'a ClientManifest, path: &str) -> &'a ssx_content::delivery::File {
    m.resources.iter().flat_map(|r| &r.files).find(|f| f.path == path).unwrap()
}

fn ranged(addr: SocketAddr, hash: &ContentHash, range: &str) -> (u16, Vec<u8>) {
    let request = format!("GET /files/{hash} HTTP/1.1\r\nHost: x\r\nRange: {range}\r\n\r\n");
    raw(addr, LOCAL, request.as_bytes(), Duration::from_secs(5))
}

#[test]
fn the_server_serves_only_its_files_by_range() {
    let folder = temp_dir("resources");
    let big = bytes(1, 300_000);
    package(&folder, "pack", &[("data/big.bin", big.clone()), ("data/small.txt", b"hello".to_vec())]);
    let (s, _console) = server(&folder, &["pack"], Vec::new());
    let addr = s.started.addr;
    let m = manifest(addr);
    let f = file_of(&m, "data/big.bin");
    assert_eq!((f.size, f.hash), (big.len() as u64, ContentHash::of(&big)));
    let (status, body) = get(addr, LOCAL, &format!("/files/{}", f.hash));
    assert_eq!((status, body == big), (200, true));
    assert_eq!(ranged(addr, &f.hash, "bytes=2-5"), (206, big[2..6].to_vec()));
    assert_eq!(ranged(addr, &f.hash, "bytes=-3"), (206, big[big.len() - 3..].to_vec()));
    assert_eq!(ranged(addr, &f.hash, "bytes=299990-"), (206, big[299_990..].to_vec()));
    assert_eq!(ranged(addr, &f.hash, "bytes=300000-").0, 416);
    assert_eq!(ranged(addr, &f.hash, "bytes=0-1,3-4").0, 416);
    assert_eq!(ranged(addr, &f.hash, "lines=0-1").0, 416);
    assert_eq!(get(addr, LOCAL, &format!("/files/{}", ContentHash::of(b"not served"))).0, 404);
    assert_eq!(get(addr, LOCAL, "/files/../resource-state/store").0, 404);
    assert_eq!(get(addr, LOCAL, "/files/").0, 404);
    assert_eq!(post(addr, LOCAL, &format!("/files/{}", f.hash), b"").0, 405);
    assert_eq!(post(addr, LOCAL, "/report", format!("{} server", f.hash).as_bytes()).0, 200);
    assert_eq!(post(addr, LOCAL, "/report", format!("{} mirror 0", f.hash).as_bytes()).0, 400);
    assert_eq!(post(addr, LOCAL, "/report", format!("{} server", ContentHash::of(b"x")).as_bytes()).0, 404);
    assert_eq!(post(addr, LOCAL, "/report", &[b'a'; 300]).0, 413);
    assert_eq!(s.started.shared.file_reports.load(Ordering::Relaxed), 1);
    s.stop();
    s.join().unwrap();
}

struct TestMirror {
    addr: SocketAddr,
    asked: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl TestMirror {
    fn start(answers: HashMap<String, (u16, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let (asked, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (a, st) = (Arc::clone(&asked), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !st.load(Ordering::Acquire) {
                let Ok((mut c, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                c.set_nonblocking(false).unwrap();
                c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut head = Vec::new();
                let mut b = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && c.read(&mut b).is_ok_and(|n| n == 1) {
                    head.push(b[0]);
                }
                let line = String::from_utf8_lossy(&head).lines().next().unwrap_or("").to_string();
                let hash = line.split(' ').nth(1).and_then(|p| p.strip_prefix("/m/")).unwrap_or("").to_string();
                a.lock().unwrap().push(hash.clone());
                let (status, body) = answers.get(&hash).cloned().unwrap_or((404, b"no".to_vec()));
                let _ = write!(c, "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = c.write_all(&body);
            }
        });
        TestMirror { addr, asked, stop }
    }
}

impl Drop for TestMirror {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn options(cache: PathBuf) -> ResourceOptions {
    ResourceOptions { cache, quota: 1 << 30, limit: 1 << 30, blocklists: ssx_content::blocklist::Follow::Nothing }
}

fn fetch(addr: SocketAddr, cache: &Path) -> Result<ssx_net_join::resources::Fetched, String> {
    let m = fetch_manifest(addr, &AtomicBool::new(false))?.expect("the server has resources");
    let progress = Progress::default();
    assert_eq!(progress.step(), ssx_net_join::resources::Step::Reading);
    let fetched = fetch_all(addr, m, &options(cache.to_path_buf()), &progress, &Arc::new(AtomicBool::new(false)));
    if fetched.is_ok() {
        assert_eq!(progress.step(), ssx_net_join::resources::Step::CheckingFiles);
    }
    fetched
}

#[test]
fn a_client_fetches_caches_resumes_and_follows_a_restart() {
    let folder = temp_dir("resources");
    let (one, two) = (bytes(2, 200_000), bytes(3, 50_000));
    package(&folder, "pack", &[("data/one.bin", one.clone()), ("data/two.bin", two.clone())]);
    let (s, mut console) = server(&folder, &["pack"], Vec::new());
    let addr = s.started.addr;
    let cache = temp_dir("cache");
    let first = fetch(addr, &cache).unwrap();
    let required = first.manifest.required_bytes();
    assert_eq!(first.fetched_bytes, required);
    let laid = &first.packages[0].1;
    assert_eq!(std::fs::read(laid.join("data/one.bin")).unwrap(), one);
    drop(first);
    let again = fetch(addr, &cache).unwrap();
    assert_eq!((again.fetched_bytes, again.cached_bytes), (0, required));
    drop(again);
    let store = FileStore::open(&cache).unwrap();
    let h = ContentHash::of(&one);
    std::fs::remove_file(cache.join(h.to_string())).unwrap();
    let mut part = store.partial(&h).unwrap();
    part.append(&one[..120_000]).unwrap();
    drop(part);
    assert_eq!(fetch(addr, &cache).unwrap().fetched_bytes, 80_000);
    std::fs::write(cache.join(h.to_string()), vec![0u8; one.len()]).unwrap();
    let torn = fetch(addr, &cache).unwrap();
    assert_eq!(torn.fetched_bytes, one.len() as u64);
    assert_eq!(std::fs::read(torn.packages[0].1.join("data/one.bin")).unwrap(), one);
    drop(torn);
    for (left, fetched) in [(one.clone(), 0), (vec![0u8; one.len()], one.len() as u64)] {
        std::fs::remove_file(cache.join(h.to_string())).unwrap();
        let mut part = store.partial(&h).unwrap();
        part.append(&left).unwrap();
        drop(part);
        assert_eq!(fetch(addr, &cache).unwrap().fetched_bytes, fetched);
    }
    assert_eq!(s.started.shared.file_reports.load(Ordering::Relaxed), 0, "no source sent a wrong file");
    let changed = bytes(4, 50_000);
    std::fs::write(folder.join("pack/data/two.bin"), &changed).unwrap();
    let before = manifest(addr);
    writeln!(console, "restart pack").unwrap();
    let end = Instant::now() + Duration::from_secs(20);
    while manifest(addr) == before {
        assert!(Instant::now() < end, "the restart is published");
        std::thread::sleep(Duration::from_millis(50));
    }
    let after = fetch(addr, &cache).unwrap();
    assert_eq!(after.fetched_bytes, changed.len() as u64);
    assert_eq!(std::fs::read(after.packages[0].1.join("data/two.bin")).unwrap(), changed);
    writeln!(console, "stop pack").unwrap();
    while !manifest(addr).resources.is_empty() {
        assert!(Instant::now() < end, "the stop is published");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(get(addr, LOCAL, &format!("/files/{}", ContentHash::of(&changed))).0, 404);
    s.stop();
    s.join().unwrap();
}

#[test]
fn a_mirror_comes_first_and_a_bad_file_is_reported() {
    let folder = temp_dir("resources");
    let (good, bad, missing) = (bytes(5, 70_000), bytes(6, 40_000), bytes(7, 10_000));
    let answers = HashMap::from([
        (ContentHash::of(&good).to_string(), (200, good.clone())),
        (ContentHash::of(&bad).to_string(), (200, bytes(9, 40_000))),
    ]);
    let mirror = TestMirror::start(answers);
    let m = Mirror::parse(&format!("http://{}/m/", mirror.addr)).unwrap();
    package(&folder, "pack", &[("a.bin", good.clone()), ("b.bin", bad.clone()), ("c.bin", missing.clone())]);
    let (s, _console) = server(&folder, &["pack"], vec![m]);
    let cache = temp_dir("cache");
    let f = fetch(s.started.addr, &cache).unwrap();
    for (p, b) in [("a.bin", &good), ("b.bin", &bad), ("c.bin", &missing)] {
        assert_eq!(&std::fs::read(f.packages[0].1.join(p)).unwrap(), b, "{p}");
    }
    let asked = mirror.asked.lock().unwrap().clone();
    assert!(asked.contains(&ContentHash::of(&good).to_string()), "{asked:?}");
    assert_eq!(s.started.shared.file_reports.load(Ordering::Relaxed), 1, "the bad file is reported");
    s.stop();
    s.join().unwrap();
}

#[test]
fn a_busy_server_is_asked_for_its_manifest_again() {
    let folder = temp_dir("resources-busy");
    package(&folder, "pack", &[("data/one.bin", bytes(4, 1000))]);
    let (s, _console) = server(&folder, &["pack"], Vec::new());
    let addr = s.started.addr;
    let spend = || (0..12).filter(|_| get(addr, LOCAL, "/resources").0 == 429).count();
    assert!(spend() > 0, "the burst is spent");
    let m = fetch_manifest(addr, &AtomicBool::new(false)).expect("asked again").expect("the server has resources");
    assert_eq!(m.manifest.resources[0].name, "pack");
    assert!(spend() > 0, "spent again");
    let cancelled = fetch_manifest(addr, &AtomicBool::new(true)).err();
    assert_eq!(cancelled.as_deref(), Some("cancelled"));
    s.stop();
}

#[test]
fn the_join_states_the_content_and_a_connected_client_is_told_of_a_change() {
    use crate::common::{Player, connected, step_until, udp_client};
    use ssx_net_join::{Granted, JoinFailure, JoinParams};
    use ssx_net_proto::join::{JoinContent, RefusalCode};
    let folder = temp_dir("resources-join");
    package(&folder, "pack", &[("data/one.bin", bytes(5, 10_000))]);
    let (s, mut console) = server(&folder, &["pack"], Vec::new());
    let addr = s.started.addr;
    let mut player = Player::new();
    let manifest_hash = || {
        let m = fetch_manifest(addr, &AtomicBool::new(false)).expect("the manifest").expect("the server has resources");
        assert_eq!(m.manifest.content_id, None, "a server without rules data states no content id");
        m.hash.0
    };
    let hash = manifest_hash();
    let current = JoinContent { content_id: None, manifest: Some(hash) };
    let mut join = |content: JoinContent| -> Result<Granted, JoinFailure> {
        let params = JoinParams { name: "Elise", password: b"", cancel: None, content };
        let r = ssx_net_join::join(addr, &player.identity, &mut player.known, &params);
        std::thread::sleep(Duration::from_millis(1100));
        r
    };
    let refusal = |r: Result<Granted, JoinFailure>| match r {
        Err(JoinFailure::Refused(r)) => Some((r.code, r.text)),
        other => panic!("not a refusal: {:?}", other.map(|g| g.info.name)),
    };
    let r = refusal(join(JoinContent { content_id: None, manifest: None }));
    assert!(matches!(&r, Some((RefusalCode::Content, t)) if t.contains("GET /resources")), "{r:?}");
    let r = refusal(join(JoinContent { content_id: None, manifest: Some([0; 32]) }));
    assert!(matches!(&r, Some((RefusalCode::ResourcesChanged, t)) if t.contains("changed")), "{r:?}");
    let r = refusal(join(JoinContent { content_id: Some([1; 32]), manifest: Some(hash) }));
    let differs = |t: &str| t.contains("content differs") && t.contains(&"01".repeat(8)) && t.contains("none");
    assert!(matches!(&r, Some((RefusalCode::Content, t)) if differs(t)), "{r:?}");
    let granted = join(current).expect("the content as the server states it");
    let (mut client, entity) = udp_client(&granted.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut client], Duration::from_secs(10), |a| connected(a[0], entity)), "connected");
    std::fs::write(folder.join("pack/data/one.bin"), bytes(6, 10_000)).unwrap();
    writeln!(console, "restart pack").unwrap();
    let said = |a: &bevy::app::App| a.world().resource::<ssx_net::ServerSaid>().resources_changed;
    assert!(step_until(&mut [&mut client], Duration::from_secs(20), |a| said(a[0]).is_some()), "the change is told");
    let new = manifest_hash();
    assert_ne!(new, hash, "the restart changed the manifest");
    assert_eq!(said(&client), Some(new));
    step_until(&mut [&mut client], Duration::from_secs(1), |_| false);
    assert_eq!(client.world().resource::<ssx_net::ServerSaid>().resources_changes, 1);
    s.stop();
    s.join().unwrap();
}
