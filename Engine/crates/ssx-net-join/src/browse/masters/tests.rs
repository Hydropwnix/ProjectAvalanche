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

#[test]
fn a_missing_member_is_the_projects_master_and_an_empty_list_none() {
    let official = official().expect("the project's master reads by the masters' rule");
    assert_eq!(official.to_string(), ssx_core::OFFICIAL_MASTER);
    assert_eq!(from_settings(None), Ok(vec![official]));
    assert_eq!(from_settings(Some(&serde_json::json!([]))), Ok(Vec::new()));
}
