use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avalanche_server::config::Config;
use avalanche_server::{ServerOptions, ServerThread};
use ssx_content::ContentHash;
use ssx_net_join::resources::fetch_manifest;
use ssx_net_proto::master::MasterUrl;

use crate::common::temp_dir;

fn package(root: &Path, name: &str, file: &[u8]) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join("data")).unwrap();
    let manifest = format!("[resource]\nname = \"{name}\"\nversion = \"1.0.0\"\napi = 1\n");
    std::fs::write(dir.join("resource.toml"), manifest).unwrap();
    std::fs::write(dir.join("data/file.bin"), file).unwrap();
}

fn master(list: Arc<Mutex<Vec<ContentHash>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let list = Arc::clone(&list);
            std::thread::spawn(move || serve(stream, &list));
        }
    });
    format!("http://{addr}")
}

fn serve(mut s: TcpStream, list: &Mutex<Vec<ContentHash>>) {
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
        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
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
    let mut config = Config::defaults();
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
fn a_resource_a_master_names_does_not_start_on_a_first_start() {
    let folder = temp_dir("blocklist-resources");
    let file = b"a file a master blocks".to_vec();
    package(&folder, "pack", &file);
    let url = master(Arc::new(Mutex::new(vec![ContentHash::of(&file)])));
    let mut c = config(&folder, &["pack"]);
    c.discovery.masters = vec![MasterUrl::parse(&url).unwrap()];
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
    c.discovery.masters = vec![MasterUrl::parse(&url).unwrap()];
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
