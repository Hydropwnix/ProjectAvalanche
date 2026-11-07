use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpStream};
use std::time::{Duration, Instant};

use avalanche_server::http::{CONNECTION_TIMEOUT, HEAD_TIMEOUT, MAX_CONNECTIONS, MAX_CONNECTIONS_PER_ADDRESS};
use ssx_net_proto::info::ServerInfo;
use ssx_net_proto::join::{CHALLENGE_SIZE, MAX_JOIN_BODY};

use crate::common::*;

fn loopback(a: u8, b: u8, c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, a, b, c))
}

#[test]
fn endpoints_and_their_errors() {
    let server = server_with(
        |c| {
            c.name = "Test Peak".into();
            c.description = "Freeride on Vorassay".into();
            c.instances[0].spec.rewinds = ssx_net_proto::Rewinds::Limit(3);
            c.instances[0].spec.ghost_style = ssx_net_proto::GhostStyle::Hologram;
        },
        None,
    );
    let addr = server.started.addr;
    let (status, body) = get(addr, LOCAL, "/info");
    assert_eq!(status, 200);
    let info = ServerInfo::from_json(&body).expect("the info reads strictly");
    assert_eq!((info.name.as_str(), info.players, info.max_players, info.password), ("Test Peak", 0, 16, false));
    assert_eq!(
        (info.description.as_str(), info.rewinds.as_str(), info.ghosts, info.ghost_style.as_str()),
        ("Freeride on Vorassay", "3", false, "hologram")
    );
    assert_eq!(info.key().expect("key"), server.started.identity_key);
    let (status, body) = get(addr, loopback(0, 0, 2), "/challenge");
    assert_eq!((status, body.len()), (200, CHALLENGE_SIZE));
    assert_eq!(post(addr, loopback(0, 0, 3), "/info", b"").0, 405);
    assert_eq!(get(addr, loopback(0, 0, 3), "/join").0, 405);
    assert_eq!(get(addr, loopback(0, 0, 3), "/players").0, 200);
    assert_eq!(post(addr, loopback(0, 0, 3), "/players", b"").0, 405);
    assert_eq!(get(addr, loopback(0, 0, 3), "/metrics").0, 404);
    let garbage = raw(
        addr,
        loopback(0, 0, 4),
        b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03garbage\r\n\r\n",
        Duration::from_secs(3),
    );
    assert_eq!(garbage.0, 400, "not HTTP");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn head_and_body_limits() {
    let server = server();
    let addr = server.started.addr;
    let started = Instant::now();
    let big_header = format!("GET /info HTTP/1.1\r\nHost: x\r\nX-Pad: {}\r\n\r\n", "a".repeat(9000));
    let (status, _) = raw(addr, loopback(0, 1, 1), big_header.as_bytes(), Duration::from_secs(3));
    assert_eq!(status, 431, "a head above 8 KiB");
    let many: String = (0..40).map(|i| format!("X-H{i}: v\r\n")).collect();
    let (status, _) = raw(
        addr,
        loopback(0, 1, 2),
        format!("GET /info HTTP/1.1\r\nHost: x\r\n{many}\r\n").as_bytes(),
        Duration::from_secs(3),
    );
    assert_eq!(status, 431, "more than 32 headers");
    let declared = format!("POST /join HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", 100_000);
    let (status, _) = raw(addr, loopback(0, 1, 3), declared.as_bytes(), Duration::from_secs(3));
    assert_eq!(status, 413, "a declared body above the limit, refused before reading");
    let mut chunked = b"POST /join HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    let chunk = vec![b'z'; MAX_JOIN_BODY + 100];
    chunked.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
    chunked.extend_from_slice(&chunk);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    let (status, _) = raw(addr, loopback(0, 1, 4), &chunked, Duration::from_secs(3));
    assert_eq!(status, 413, "a chunked body above the limit");
    assert!(started.elapsed() < Duration::from_secs(3), "every refusal at once: {:?}", started.elapsed());
    server.stop();
    server.join().expect("stops");
}

fn held(addr: std::net::SocketAddr, local: IpAddr, bytes: &'static [u8]) -> std::thread::JoinHandle<(Duration, u16)> {
    std::thread::spawn(move || {
        let started = Instant::now();
        let (status, _) = raw(addr, local, bytes, Duration::from_secs(30));
        (started.elapsed(), status)
    })
}

#[test]
fn slow_clients_are_cut() {
    let server = server();
    let addr = server.started.addr;
    let head = held(addr, loopback(0, 2, 1), b"GET /info HTTP/1.1\r\nHost: x\r\n");
    let body =
        held(addr, loopback(0, 2, 2), b"POST /join HTTP/1.1\r\nHost: x\r\nContent-Length: 400\r\n\r\n0123456789");
    let (head_time, head_status) = head.join().expect("thread");
    let (body_time, _) = body.join().expect("thread");
    assert!(
        head_time >= HEAD_TIMEOUT && head_time < HEAD_TIMEOUT + Duration::from_secs(2),
        "{head_time:?} {head_status}"
    );
    assert!(
        body_time >= CONNECTION_TIMEOUT - Duration::from_secs(1)
            && body_time < CONNECTION_TIMEOUT + Duration::from_secs(2),
        "{body_time:?}"
    );
    assert_eq!(get(addr, loopback(0, 2, 3), "/info").0, 200);
    server.stop();
    server.join().expect("stops");
}

fn open_idle(addr: std::net::SocketAddr, local: IpAddr) -> TcpStream {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    runtime.block_on(async {
        let socket = tokio::net::TcpSocket::new_v4().expect("socket");
        socket.bind(std::net::SocketAddr::new(local, 0)).expect("bind");
        let stream = socket.connect(addr).await.expect("connect").into_std().expect("std");
        stream.set_nonblocking(false).expect("blocking");
        stream
    })
}

fn closed_at_once(mut stream: TcpStream) -> bool {
    stream.set_read_timeout(Some(Duration::from_secs(1))).expect("timeout");
    let _ = stream.write_all(b"GET /info HTTP/1.1\r\nHost: x\r\n\r\n");
    let mut buf = [0u8; 64];
    match stream.read(&mut buf) {
        Ok(n) => n == 0,
        Err(e) => !matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut),
    }
}

#[test]
fn connection_caps() {
    let server = server();
    let addr = server.started.addr;
    let per = MAX_CONNECTIONS_PER_ADDRESS as usize;
    let mut idle: Vec<TcpStream> = (0..per).map(|_| open_idle(addr, loopback(0, 3, 1))).collect();
    std::thread::sleep(Duration::from_millis(200));
    assert!(closed_at_once(open_idle(addr, loopback(0, 3, 1))), "one more from the same address is closed");
    assert_eq!(get(addr, loopback(0, 3, 2), "/info").0, 200, "another address is served");
    let mut c = 0u8;
    while idle.len() < MAX_CONNECTIONS {
        c += 1;
        for _ in 0..per.min(MAX_CONNECTIONS - idle.len()) {
            idle.push(open_idle(addr, loopback(1, 0, c)));
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(closed_at_once(open_idle(addr, loopback(0, 3, 3))), "above the overall cap");
    drop(idle);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(get(addr, loopback(0, 3, 4), "/info").0, 200, "served again once they closed");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn rate_limits_per_address_and_endpoint() {
    let server = server();
    let addr = server.started.addr;
    let (a, b) = (loopback(0, 4, 1), loopback(0, 4, 2));
    let infos: Vec<u16> = (0..9).map(|_| get(addr, a, "/info").0).collect();
    assert_eq!(infos[..8], [200; 8], "the burst of 8");
    assert_eq!(infos[8], 429, "the ninth at once");
    assert_eq!(get(addr, b, "/info").0, 200, "another address has its own budget");
    let challenges: Vec<u16> = (0..5).map(|_| get(addr, a, "/challenge").0).collect();
    assert_eq!(challenges, [200, 200, 200, 200, 429], "the challenge's burst of 4, its own budget");
    let joins: Vec<u16> = (0..5).map(|_| post(addr, a, "/join", b"x").0).collect();
    assert_eq!(joins, [400, 400, 400, 400, 429], "garbage joins count too");
    let limited = raw(addr, a, b"GET /info HTTP/1.1\r\nHost: x\r\n\r\n", Duration::from_secs(3));
    assert_eq!(limited.0, 429);
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(get(addr, a, "/info").0, 200, "refilled");
    server.stop();
    server.join().expect("stops");
}
