use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicU16;

use super::*;
use crate::browse::resolve::{COUNTER_TESTS, under_way};

static LISTENER_PORT: AtomicU16 = AtomicU16::new(0);

fn to_the_listener(name: &str, _: u16) -> std::io::Result<Option<SocketAddr>> {
    assert_eq!(name, "master.invalid", "only the master's own name is looked up");
    Ok(Some(SocketAddr::from(([127, 0, 0, 1], LISTENER_PORT.load(Ordering::Acquire)))))
}

fn slower_than_the_bound(_: &str, port: u16) -> std::io::Result<Option<SocketAddr>> {
    std::thread::sleep(RESOLVE_TIMEOUT + Duration::from_millis(500));
    Ok(Some(SocketAddr::from(([192, 0, 2, 1], port))))
}

#[test]
fn a_masters_name_is_looked_up_bounded_counted_and_cancellable() {
    let _counter = COUNTER_TESTS.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let url = MasterUrl::parse("http://master.invalid:27900").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    LISTENER_PORT.store(listener.local_addr().unwrap().port(), Ordering::Release);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
            head.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").unwrap();
        String::from_utf8_lossy(&head).to_ascii_lowercase()
    });
    let e = fetch_until(&url, None, &None, None, to_the_listener).unwrap_err();
    assert!(e.contains("HTTP 404"), "{e}");
    let head = server.join().unwrap();
    assert!(head.contains("host: master.invalid:27900"), "{head}");
    let t = Instant::now();
    let e = fetch_until(&url, None, &None, None, slower_than_the_bound).unwrap_err();
    assert!(e.contains("within 2 s"), "{e}");
    assert!(t.elapsed() < RESOLVE_TIMEOUT + Duration::from_millis(400), "{:?}", t.elapsed());
    assert!(under_way() >= 1, "the late lookup's thread is counted");
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancel);
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        flag.store(true, Ordering::Release);
    });
    let t = Instant::now();
    let e = fetch_until(&url, None, &None, Some(cancel), slower_than_the_bound).unwrap_err();
    stopper.join().unwrap();
    assert!(e.contains("cancelled"), "{e}");
    assert!(t.elapsed() < Duration::from_millis(500), "{:?}", t.elapsed());
    let refused = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = MasterUrl::parse(&format!("http://{}", refused.local_addr().unwrap())).unwrap();
    drop(refused);
    let e = fetch_until(&closed, None, &None, None, to_the_listener).unwrap_err();
    assert!(e.contains("nothing answers there"), "{e}");
    let end = Instant::now() + Duration::from_secs(5);
    while under_way() > 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(under_way(), 0, "the late lookups' threads have ended");
}

fn entry(address: &str, port: u16, key: &str, name: &str) -> ListedServer {
    ListedServer {
        address: address.into(),
        port,
        key: key.into(),
        name: name.into(),
        mode: "freeride".into(),
        drop: "exp_alps_vorassay_drop_3".into(),
        map: None,
        players: 0,
        max_players: 16,
        password: false,
        region: String::new(),
        tags: Vec::new(),
        build: BUILD_ID.into(),
        api_level: 0,
        info_version: ssx_net_proto::info::INFO_VERSION,
        info_hash: "ab".repeat(32),
        resources: ssx_net_proto::resources::ResourceSummary {
            content_id: String::new(),
            client_mods: "presentation".into(),
            client_scripts: false,
            count: 0,
            download: 0,
            manifest: "ab".repeat(32),
            manifest_bytes: 200,
        },
    }
}

#[test]
fn the_masters_list_takes_each_key_and_each_place_once() {
    let (k0, k1, k2) = ("01".repeat(32), "02".repeat(32), "03".repeat(32));
    let servers = vec![
        entry("203.0.113.1", 27450, &k0, "first"),
        entry("203.0.113.2", 27450, &k0, "the same key again"),
        entry("203.0.113.1", 27450, &k1, "the first's place again"),
        entry("203.0.113.1", 27451, &k1, "another port"),
        entry("not an address", 27450, &k2, "no address"),
        entry("203.0.113.3", 27450, "zz", "no key"),
        entry("2001:db8::7", 27450, &k2, "IPv6"),
    ];
    let list =
        ServerList { list_version: ssx_net_proto::master::LIST_VERSION, protocol_version: PROTOCOL_VERSION, servers };
    let (taken, left_out) = listed(&list);
    let names: Vec<&str> = taken.iter().map(|l| l.server.name.as_str()).collect();
    assert_eq!(names, ["first", "another port", "IPv6"]);
    assert_eq!(left_out, 4);
    assert_eq!(socket(&taken[1]), Some(SocketAddr::from(([203, 0, 113, 1], 27451))));
    let m = MasterList::new(MasterUrl::parse("http://127.0.0.1:9").unwrap(), None);
    assert_eq!(m.listed(), (Vec::new(), 0));
    assert!(!m.waiting() && m.status.is_none());
}
