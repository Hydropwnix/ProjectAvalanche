use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use ssx_net_join::browse::query::{InfoAnswer, QUERY_TIMEOUT, ask_info, ask_players};
use ssx_net_proto::PROTOCOL_VERSION;
use ssx_net_proto::info::INFO_VERSION;

fn hostile(serve: fn(TcpStream)) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || serve(stream));
        }
    });
    addr
}

fn read_head(s: &mut TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
}

fn answer(mut s: TcpStream, status: &str, extra: &str, body: &[u8]) {
    read_head(&mut s);
    let head = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn info_json(name: &str, info_version: u32, protocol: u32) -> String {
    let resources = if info_version >= 4 {
        format!(
            r#","resources":{{"content_id":"","client_mods":"presentation","client_scripts":true,"count":0,"download":0,"manifest":"{}","manifest_bytes":120}}"#,
            "2".repeat(64)
        )
    } else {
        String::new()
    };
    let operation = if info_version >= 5 { r#","region":"","tags":[],"icon":"""#.to_string() } else { String::new() };
    let map = if info_version >= 6 { r#","map":null"# } else { "" };
    format!(
        r#"{{"info_version":{info_version},"name":"{name}","description":"d","protocol_version":{protocol},"api_level":0,"build":"b","players":1,"max_players":8,"password":false,"mode":"freeride","drop":"exp_alps_vorassay_drop_3"{map},"rewinds":"on","ghosts":false,"ghost_style":"character","identity_key":"{}"{resources}{operation}}}"#,
        "1".repeat(64)
    )
}

fn refused(addr: SocketAddr, why: &str) {
    let started = Instant::now();
    let e = ask_info(addr).expect_err(why);
    assert!(e.contains(why), "{why}: {e}");
    assert!(started.elapsed() < QUERY_TIMEOUT + Duration::from_secs(2), "{why}: {:?}", started.elapsed());
}

#[test]
fn oversized_malformed_and_rude_answers() {
    refused(hostile(|s| answer(s, "200 OK", "", &vec![b' '; 5000])), "larger than 4096");
    refused(hostile(|s| answer(s, "200 OK", "", b"{\"info_version\":3,")), "does not read");
    refused(hostile(|s| answer(s, "302 Found", "Location: http://10.0.0.1/\r\n", b"")), "HTTP 302");
    refused(hostile(|s| answer(s, "429 Too Many", "Retry-After: 99\r\n", b"")), "HTTP 429");
    refused(
        hostile(|s| answer(s, "200 OK", &format!("X-Pad: {}\r\n", "a".repeat(100_000)), b"{}")),
        "cannot be reached",
    );
    refused(
        hostile(|s| answer(s, "200 OK", "", info_json("Peak\\u0007", INFO_VERSION, PROTOCOL_VERSION).as_bytes())),
        "the server name",
    );
    refused(
        hostile(|s| answer(s, "200 OK", "", info_json(&"N".repeat(10_000), INFO_VERSION, PROTOCOL_VERSION).as_bytes())),
        "larger than",
    );
    refused(
        hostile(|s| answer(s, "200 OK", "", info_json(&"N".repeat(41), INFO_VERSION, PROTOCOL_VERSION).as_bytes())),
        "at most 40",
    );
    refused(
        hostile(|mut s| {
            read_head(&mut s);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            while s.write_all(b"400\r\n").is_ok() && s.write_all(&[b' '; 1024]).is_ok() && s.write_all(b"\r\n").is_ok()
            {
            }
        }),
        "larger than 4096",
    );
}

#[test]
fn another_version_is_a_greyed_row_with_its_reason() {
    let old = hostile(|s| answer(s, "200 OK", "", info_json("Old Peak", 2, PROTOCOL_VERSION).as_bytes()));
    match ask_info(old).expect("the head reads") {
        (InfoAnswer::Incompatible { head, why }, _) => {
            assert_eq!(head.name.as_deref(), Some("Old Peak"));
            assert!(why.contains("version 2"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    let newer = hostile(|s| {
        let body = format!(r#"{{"info_version":9,"protocol_version":{},"name":"Next","x":[1]}}"#, PROTOCOL_VERSION + 1);
        answer(s, "200 OK", "", body.as_bytes())
    });
    match ask_info(newer).expect("the head reads") {
        (InfoAnswer::Incompatible { why, .. }, _) => {
            assert!(why.contains(&format!("protocol {}", PROTOCOL_VERSION + 1)), "{why}")
        }
        other => panic!("{other:?}"),
    }
    let good = hostile(|s| answer(s, "200 OK", "", info_json("Good Peak", INFO_VERSION, PROTOCOL_VERSION).as_bytes()));
    assert!(matches!(ask_info(good), Ok((InfoAnswer::Info(i), _)) if i.name == "Good Peak"));
}

#[test]
fn slow_and_silent_servers_end_within_the_browsers_time() {
    let silent = hostile(|s| {
        std::thread::sleep(Duration::from_secs(30));
        drop(s);
    });
    refused(silent, "no answer within");
    let drip = hostile(|mut s| {
        read_head(&mut s);
        for b in b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789" {
            if s.write_all(&[*b]).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    refused(drip, "no answer within");
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    refused(closed, "nothing answers there");
}

#[test]
fn hostile_player_lists() {
    let v = INFO_VERSION;
    let older = format!("version {}", v - 1);
    for (body, why) in [
        (format!(r#"{{"info_version":{v},"players":["Elise","Mac\u0007"]}}"#), "a player name"),
        (format!(r#"{{"info_version":{v},"players":["{}"]}}"#, "x".repeat(10_000)), "larger than 8192"),
        (format!(r#"{{"info_version":{v},"players":["Elise"],"keys":["00"]}}"#), "does not read"),
        (format!(r#"{{"info_version":{},"players":[]}}"#, v - 1), older.as_str()),
        (format!(r#"{{"info_version":{v},"players":[{}]}}"#, vec!["\"p\""; 65].join(",")), "at most 64"),
    ] {
        let addr = serve_once(body);
        let e = ask_players(addr).expect_err(why);
        assert!(e.contains(why), "{why}: {e}");
    }
    let ok = serve_once(format!(r#"{{"info_version":{v},"players":["Elise","Zoë Payne"]}}"#));
    assert_eq!(ask_players(ok).map(|l| l.players), Ok(vec!["Elise".to_string(), "Zoë Payne".to_string()]));
}

fn serve_once(body: String) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let addr = listener.local_addr().expect("its address");
    std::thread::spawn(move || {
        if let Ok((s, _)) = listener.accept() {
            answer(s, "200 OK", "", body.as_bytes());
        }
    });
    addr
}

#[test]
fn the_refresher_keeps_its_rate_and_leaves_nothing_behind() {
    use ssx_net_join::browse::refresh::{Ask, MIN_INTERVAL, Refresher, Reply, WORKERS};
    use ssx_net_join::browse::{Host, ServerAddress};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let at = |a: SocketAddr| ServerAddress { host: Host::Ip(a.ip()), port: a.port() };
    let good =
        at(hostile(|s| answer(s, "200 OK", "", info_json("Good Peak", INFO_VERSION, PROTOCOL_VERSION).as_bytes())));
    let mut r = Refresher::start(None).expect("threads");
    let now = Instant::now();
    assert!(r.ask(&good, Ask::Info, Duration::ZERO, now));
    assert!(!r.ask(&good, Ask::Info, Duration::ZERO, now), "one question waits per server");
    let mut answers = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while answers.is_empty() && Instant::now() < end {
        answers = r.take();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(&answers[..], [a] if matches!(a.reply, Reply::Info(Ok(_)))), "{answers:?}");
    assert!(!r.ask(&good, Ask::Info, Duration::ZERO, now + MIN_INTERVAL / 2), "the rate per server");
    assert!(r.ask(&good, Ask::Info, Duration::ZERO, now + MIN_INTERVAL), "asked again after the interval");
    let accepted = Arc::new(AtomicUsize::new(0));
    let mut silent = Vec::new();
    for _ in 0..30 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        silent.push(at(listener.local_addr().unwrap()));
        let count = Arc::clone(&accepted);
        std::thread::spawn(move || {
            for s in listener.incoming().flatten() {
                count.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(20));
                    drop(s);
                });
            }
        });
    }
    let mut r = Refresher::start(None).expect("threads");
    for address in &silent {
        assert!(r.ask(address, Ask::Info, Duration::ZERO, Instant::now()));
    }
    std::thread::sleep(Duration::from_millis(500));
    drop(r);
    let at_drop = accepted.load(Ordering::SeqCst);
    std::thread::sleep(QUERY_TIMEOUT + Duration::from_secs(2));
    let later = accepted.load(Ordering::SeqCst);
    assert!(at_drop <= WORKERS && later == at_drop, "connections at the drop {at_drop}, after {later}");
}
