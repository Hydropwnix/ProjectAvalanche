use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avalanche_server::config::Config;
use avalanche_server::{ServerOptions, ServerThread};
use ssx_content::ContentHash;
use ssx_net_join::resources::fetch_manifest;

use crate::common::temp_dir;

fn package(root: &Path, name: &str, file: &[u8]) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let manifest = format!("[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\n");
    std::fs::write(dir.join("resource.toml"), manifest).unwrap();
    std::fs::write(dir.join("data/file.bin"), file).unwrap();
}

fn master(list: Arc<Mutex<Vec<ContentHash>>>) -> String {
    counting_master(list).0
}

fn counting_master(list: Arc<Mutex<Vec<ContentHash>>>) -> (String, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (asks, others) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let counts = (Arc::clone(&asks), Arc::clone(&others));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let list = Arc::clone(&list);
            let counts = (Arc::clone(&counts.0), Arc::clone(&counts.1));
            std::thread::spawn(move || serve(stream, &list, &counts));
        }
    });
    (format!("http://{addr}"), asks, others)
}

fn serve(mut s: TcpStream, list: &Mutex<Vec<ContentHash>>, counts: &(Arc<AtomicUsize>, Arc<AtomicUsize>)) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    if !head.starts_with("get /v1/blocklist ") {
        counts.1.fetch_add(1, Ordering::SeqCst);
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    counts.0.fetch_add(1, Ordering::SeqCst);
    let mut hashes: Vec<String> = list.lock().unwrap().iter().map(ToString::to_string).collect();
    hashes.sort();
    let body = format!("{{\"blocklist_version\":1,\"resources\":[{}]}}", {
        let quoted: Vec<String> = hashes.iter().map(|h| format!("\"{h}\"")).collect();
        quoted.join(",")
    });
    let etag = format!("\"{}\"", ContentHash::of(body.as_bytes()));
    if head.contains(&format!("if-none-match: {}", etag.to_ascii_lowercase())) {
        let _ = write!(s, "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nConnection: close\r\n\r\n");
        return;
    }
    let _ = write!(
        s,
        "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn options(config: Config, data_dir: &Path, console: Option<std::io::PipeReader>) -> ServerOptions {
    ServerOptions {
        config,
        data_dir: data_dir.to_path_buf(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: Some(Box::new(move |app| {
            app.insert_resource(avalanche_server::blocklist::Interval(Duration::from_millis(300)));
            if let Some(input) = console {
                avalanche_server::console::attach(app, input, false).expect("the console's thread");
            }
        })),
        rules: avalanche_server::rules::RulesSource::Unchecked,
    }
}

fn config(folder: &Path, start: &[&str]) -> Config {
    let mut config = crate::common::defaults();
    config.bind = std::net::Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.resources.folder = folder.to_path_buf();
    config.resources.start = start.iter().map(|s| s.to_string()).collect();
    config
}

fn names(addr: SocketAddr) -> Vec<String> {
    match fetch_manifest(addr, &AtomicBool::new(false)) {
        Ok(Some(m)) => m.manifest.resources.iter().map(|r| r.name.clone()).collect(),
        Ok(None) => Vec::new(),
        Err(e) => panic!("the manifest: {e}"),
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < end, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_resource_the_servers_own_list_names_does_not_start() {
    let folder = temp_dir("blocklist-resources");
    let file = b"a file the operator blocks".to_vec();
    package(&folder, "pack", &file);
    let data = temp_dir("blocklist-data");
    let text = format!("Project Avalanche blocked resources v1\n{} a reason\n", ContentHash::of(&file));
    std::fs::write(data.join("blocked.txt"), text).unwrap();
    let mut c = config(&folder, &["pack"]);
    c.discovery.blocklist = Some("blocked.txt".into());
    let error = match avalanche_server::spawn(options(c.clone(), &data, None)) {
        Ok(s) => {
            s.stop();
            let _ = s.join();
            panic!("the server started a blocked resource");
        }
        Err(e) => e.to_string(),
    };
    for part in ["\"pack\"", "the server's own list", "data/file.bin"] {
        assert!(error.contains(part), "{part} in: {error}");
    }
    std::fs::write(data.join("blocked.txt"), "Project Avalanche blocked resources v1\nnot-a-hash\n").unwrap();
    let error = avalanche_server::spawn(options(c, &data, None)).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(error.contains("[discovery] blocklist"), "{error}");
}

#[test]
fn an_unlisted_server_follows_the_masters_blocklist_and_sends_no_heartbeat() {
    use avalanche_server::config::Discovery;
    let folder = temp_dir("blocklist-resources");
    let blocked = b"a file a master blocks".to_vec();
    package(&folder, "pack", &blocked);
    package(&folder, "other", b"a file no list names");
    let (url, asks, others) = counting_master(Arc::new(Mutex::new(vec![ContentHash::of(&blocked)])));
    let unlisted = || Discovery { public: false, query: false, ..crate::common::listed_on(&url) };
    let mut c = config(&folder, &["pack"]);
    c.discovery = unlisted();
    assert!(c.discovery.master.is_some() && c.discovery.heartbeat().is_none());
    let error = match avalanche_server::spawn(options(c, &temp_dir("blocklist-data"), None)) {
        Ok(s) => {
            s.stop();
            let _ = s.join();
            panic!("an unlisted server started a resource the master blocks");
        }
        Err(e) => e.to_string(),
    };
    for part in ["[resources] start", "\"pack\"", url.as_str()] {
        assert!(error.contains(part), "{part} in: {error}");
    }
    let mut c = config(&folder, &["other"]);
    c.discovery = unlisted();
    let data = temp_dir("blocklist-data");
    let s = avalanche_server::spawn(options(c, &data, None)).expect("an unlisted server starts");
    let addr = s.started.addr;
    wait_for("other runs", || names(addr) == ["other"]);
    std::thread::sleep(Duration::from_millis(1000));
    assert!(asks.load(Ordering::SeqCst) >= 2, "the blocklist asked at each start");
    assert_eq!(others.load(Ordering::SeqCst), 0, "no challenge or heartbeat reached the master");
    s.stop();
    let _ = s.join();
    let mut c = config(&folder, &["other"]);
    c.discovery = crate::common::listed_on(&url);
    let s = avalanche_server::spawn(options(c, &temp_dir("blocklist-data"), None)).expect("a listed server starts");
    wait_for("the listed server's challenge", || others.load(Ordering::SeqCst) > 0);
    s.stop();
    let _ = s.join();
}

#[test]
fn a_resource_a_master_names_does_not_start_on_a_first_start() {
    let folder = temp_dir("blocklist-resources");
    let file = b"a file a master blocks".to_vec();
    package(&folder, "pack", &file);
    let url = master(Arc::new(Mutex::new(vec![ContentHash::of(&file)])));
    let mut c = config(&folder, &["pack"]);
    c.discovery = crate::common::listed_on(&url);
    let data = temp_dir("blocklist-data");
    let error = match avalanche_server::spawn(options(c, &data, None)) {
        Ok(s) => {
            s.stop();
            let _ = s.join();
            panic!("the server started a resource its master blocks");
        }
        Err(e) => e.to_string(),
    };
    for part in ["[resources] start", "\"pack\"", url.as_str(), "data/file.bin"] {
        assert!(error.contains(part), "{part} in: {error}");
    }
    assert_eq!(std::fs::read_dir(data.join("Blocklists")).unwrap().count(), 1, "the list kept");
}

#[test]
fn a_running_resource_a_master_comes_to_name_is_stopped_at_the_next_fetch() {
    let folder = temp_dir("blocklist-resources");
    let (file, other) = (b"the resource's file".to_vec(), b"another resource's file".to_vec());
    package(&folder, "pack", &file);
    package(&folder, "other", &other);
    let list = Arc::new(Mutex::new(Vec::new()));
    let url = master(Arc::clone(&list));
    let mut c = config(&folder, &["pack", "other"]);
    c.discovery = crate::common::listed_on(&url);
    let data = temp_dir("blocklist-data");
    let (input, mut console) = std::io::pipe().unwrap();
    let s: ServerThread = avalanche_server::spawn(options(c, &data, Some(input))).expect("the server starts");
    let addr = s.started.addr;
    wait_for("both run", || names(addr).len() == 2);
    list.lock().unwrap().push(ContentHash::of(&file));
    wait_for("pack stopped", || names(addr) == ["other"]);
    writeln!(console, "start pack").unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(names(addr), ["other"]);
    let kept = data.join("Blocklists");
    assert_eq!(std::fs::read_dir(&kept).unwrap().count(), 1, "one kept list");
    list.lock().unwrap().clear();
    std::thread::sleep(Duration::from_millis(800));
    writeln!(console, "start pack").unwrap();
    wait_for("pack runs again", || names(addr).len() == 2);
    s.stop();
    s.join().unwrap();
}
