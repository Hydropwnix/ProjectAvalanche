use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use avalanche_master::forwarded::{Network, Trusted};
use ssx_net_proto::master::{HeartbeatAnswer, MasterChallenge, ServerList};

use crate::common::*;

const PROXY: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 9, 1));

fn behind_proxy() -> avalanche_master::MasterThread {
    master_with(|c| c.trusted = Trusted::new(vec![Network::parse("127.0.9.0/24").expect("network")]))
}

fn via(addr: SocketAddr, client: &str, method: &str, path: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    let (s, _, b) = runtime().block_on(exchange(addr, PROXY, method, path, body, &[("x-forwarded-for", client)]));
    (s, b)
}

fn counters(m: &avalanche_master::MasterThread) -> (u64, u64, u64) {
    let s = &m.started.master.stats;
    (s.proxied.load(Ordering::Relaxed), s.forwarded_refused.load(Ordering::Relaxed), s.limited.load(Ordering::Relaxed))
}

fn list_through_proxy(addr: SocketAddr) -> ServerList {
    let path = format!("/v1/servers?protocol={}", ssx_net_proto::PROTOCOL_VERSION);
    let (status, body) = via(addr, "198.51.100.200", "GET", &path, vec![]);
    assert_eq!(status, 200);
    ServerList::from_json(&body, ssx_net_proto::PROTOCOL_VERSION).expect("the list reads")
}

#[test]
fn clients_behind_a_proxy_are_limited_one_by_one() {
    let m = behind_proxy();
    let addr = m.started.addr;
    let codes: Vec<u16> = (0..40).map(|_| via(addr, "203.0.113.5", "GET", "/v1/challenge", vec![]).0).collect();
    assert_eq!(codes.iter().filter(|&&c| c == 200).count(), 32, "{codes:?}");
    assert!(codes[32..].iter().all(|&c| c == 429), "{codes:?}");
    assert_eq!(via(addr, "203.0.113.6", "GET", "/v1/challenge", vec![]).0, 200, "another client passes");
    assert_eq!(via(addr, "203.0.113.6, 203.0.113.5", "GET", "/v1/challenge", vec![]).0, 429);
    assert_eq!(via(addr, "2001:db8::5", "GET", "/v1/challenge", vec![]).0, 200, "an IPv6 client");
    assert_eq!(counters(&m), (43, 0, 9));
    m.stop();
    m.join().expect("stops");
}

#[test]
fn a_heartbeat_through_the_proxy_is_listed_at_its_client_and_checked_there() {
    let m = behind_proxy();
    let addr = m.started.addr;
    let fake = Fake::start("Behind Coolify", Query::Honest, true);
    let client = fake.ip.to_string();
    let (status, body) = via(addr, &client, "GET", "/v1/challenge", vec![]);
    assert_eq!(status, 200);
    let c = MasterChallenge::from_bytes(&body).expect("a challenge");
    let sealed = fake.beat().seal(&fake.identity, &c).expect("sealed");
    let (status, said) = via(addr, "127.0.20.250", "POST", "/v1/heartbeat", sealed.clone());
    assert_eq!((status, String::from_utf8_lossy(&said).contains("not for this address")), (400, true));
    let before = fake.contacted.load(Ordering::Relaxed);
    let (status, body) = via(addr, &client, "POST", "/v1/heartbeat", sealed);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let answer = HeartbeatAnswer::open(&body, &fake.identity, &c).expect("the master's boxed answer");
    assert!(answer.listed, "{}", answer.reason);
    assert!(fake.contacted.load(Ordering::Relaxed) > before, "the check went to the client itself");
    let l = list_through_proxy(addr);
    assert_eq!(l.servers.len(), 1);
    assert_eq!((l.servers[0].address.as_str(), l.servers[0].port), (client.as_str(), fake.port));
    m.stop();
    m.join().expect("stops");
}

#[test]
fn forwarded_headers_from_any_other_peer_change_nothing() {
    for m in [behind_proxy(), master_with(|_| {})] {
        let addr = m.started.addr;
        let rt = runtime();
        let flood = next_ip();
        let codes: Vec<u16> = (0..34)
            .map(|i| {
                let named = format!("198.51.100.{i}");
                rt.block_on(exchange(addr, flood, "GET", "/v1/challenge", vec![], &[("x-forwarded-for", &named)])).0
            })
            .collect();
        assert_eq!(codes.iter().filter(|&&c| c == 200).count(), 32, "{codes:?}");
        let fake = Fake::start("Direct Forger", Query::Honest, true);
        let forged = [("x-forwarded-for", "198.51.100.77")];
        let (_, _, body) = rt.block_on(exchange(addr, fake.ip, "GET", "/v1/challenge", vec![], &forged));
        let c = MasterChallenge::from_bytes(&body).expect("a challenge");
        let sealed = fake.beat().seal(&fake.identity, &c).expect("sealed");
        let (status, _, body) = rt.block_on(exchange(addr, fake.ip, "POST", "/v1/heartbeat", sealed, &forged));
        assert_eq!(status, 200);
        assert!(HeartbeatAnswer::open(&body, &fake.identity, &c).expect("opens").listed);
        let path = format!("/v1/servers?protocol={}", ssx_net_proto::PROTOCOL_VERSION);
        let (_, _, body) = rt.block_on(exchange(addr, next_ip(), "GET", &path, vec![], &forged));
        let l = ServerList::from_json(&body, ssx_net_proto::PROTOCOL_VERSION).expect("reads");
        assert_eq!(l.servers.iter().map(|s| s.address.clone()).collect::<Vec<_>>(), [fake.ip.to_string()]);
        assert_eq!(counters(&m).0 + counters(&m).1, 0, "nothing proxied, nothing refused");
        m.stop();
        m.join().expect("stops");
    }
}

type Case<'a> = (&'a str, &'a [(&'a str, &'a str)], u16);

#[test]
fn a_proxys_hostile_headers_are_refused_and_counted() {
    let m = behind_proxy();
    let addr = m.started.addr;
    let rt = runtime();
    let get = |headers: &[(&str, &str)]| rt.block_on(exchange(addr, PROXY, "GET", "/v1/challenge", vec![], headers)).0;
    let hundred = vec!["203.0.113.7"; 100].join(", ");
    let big = "7".repeat(64 * 1024);
    let cases: [Case; 10] = [
        ("missing", &[], 400),
        ("empty", &[("x-forwarded-for", "")], 400),
        ("malformed", &[("x-forwarded-for", "unknown")], 400),
        ("IPv6", &[("x-forwarded-for", "2001:db8::7")], 200),
        ("bracketed", &[("x-forwarded-for", "[2001:db8::7]")], 400),
        ("port-suffixed", &[("x-forwarded-for", "203.0.113.7:4711")], 400),
        ("100 entries", &[("x-forwarded-for", &hundred)], 400),
        ("64 KiB", &[("x-forwarded-for", &big)], 431),
        ("RFC 7239 only", &[("forwarded", "for=203.0.113.7")], 400),
        ("X-Real-IP only", &[("x-real-ip", "203.0.113.7")], 400),
    ];
    let mut refused = 0;
    for (what, headers, want) in cases {
        assert_eq!(get(headers), want, "{what}");
        refused += u64::from(want == 400);
        assert_eq!(counters(&m).1, refused, "{what}: counted");
    }
    let lines: Vec<(&str, &str)> = (0..20).map(|_| ("x-forwarded-for", "203.0.113.8")).collect();
    assert_eq!(get(&lines), 400);
    let lines: Vec<(&str, &str)> = (0..40).map(|_| ("x-forwarded-for", "203.0.113.8")).collect();
    assert_eq!(get(&lines), 431);
    assert_eq!(counters(&m).1, refused + 1);
    let lines: Vec<(&str, &str)> = (0..3).map(|_| ("x-forwarded-for", "203.0.113.8")).collect();
    assert_eq!(get(&lines), 200, "three lines of one proxy chain");
    m.stop();
    m.join().expect("stops");
}

#[test]
fn a_clients_requests_at_once_are_bounded_behind_the_proxy() {
    let m = behind_proxy();
    let addr = m.started.addr;
    let rt = runtime();
    let held: Vec<std::net::TcpStream> = (0..16)
        .map(|_| {
            let mut s = rt.block_on(async {
                let s = tokio::net::TcpSocket::new_v4().expect("socket");
                s.bind(SocketAddr::new(PROXY, 0)).expect("bind");
                s.connect(addr).await.expect("connect").into_std().expect("std")
            });
            s.set_nonblocking(false).expect("blocking");
            s.write_all(
                b"POST /v1/heartbeat HTTP/1.1\r\nHost: m\r\nX-Forwarded-For: 203.0.113.9\r\nContent-Length: 2000\r\n\r\n",
            )
            .expect("head");
            s
        })
        .collect();
    let t0 = Instant::now();
    while counters(&m).0 < 16 && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(via(addr, "203.0.113.9", "GET", "/v1/challenge", vec![]).0, 429, "the 17th at once");
    assert_eq!(via(addr, "203.0.113.10", "GET", "/v1/challenge", vec![]).0, 200, "another client passes");
    drop(held);
    let t0 = Instant::now();
    let mut status = 0;
    while status != 200 && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
        status = via(addr, "203.0.113.9", "GET", "/v1/challenge", vec![]).0;
    }
    assert_eq!(status, 200, "its slots are back once the held requests ended");
    m.stop();
    m.join().expect("stops");
}

#[test]
fn the_health_check() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let (status, _, body) = rt.block_on(exchange(addr, next_ip(), "GET", "/v1/health", vec![], &[]));
    assert_eq!((status, body.as_slice()), (200, &b"ok\n"[..]));
    assert_eq!(rt.block_on(exchange(addr, next_ip(), "POST", "/v1/health", vec![], &[])).0, 405);
    let mut config = m.started.master.config.clone();
    config.port = addr.port();
    assert_eq!(avalanche_master::health::check(&config), Ok(addr));
    m.stop();
    m.join().expect("stops");
    let down = avalanche_master::health::check(&config).expect_err("a stopped master");
    assert!(down.contains("Connection refused") || down.contains("refused"), "{down}");
    let m = behind_proxy();
    let addr = m.started.addr;
    let own = |method, path| rt.block_on(exchange(addr, PROXY, method, path, vec![], &[])).0;
    assert_eq!(own("GET", "/v1/health"), 200);
    assert_eq!((own("POST", "/v1/health"), own("GET", "/v1/challenge")), (400, 400));
    assert_eq!(counters(&m), (0, 2, 0));
    m.stop();
    m.join().expect("stops");
    let m = master_with(|c| c.trusted = Trusted::new(vec![Network::parse("127.0.0.0/8").expect("network")]));
    let mut config = m.started.master.config.clone();
    config.port = m.started.addr.port();
    assert_eq!(avalanche_master::health::check(&config), Ok(m.started.addr));
    m.stop();
    m.join().expect("stops");
}
