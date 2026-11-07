use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use ssx_net_join::browse::lan::{LanSearch, Targets};
use ssx_net_join::browse::masters::{Fetched, Masters, STEP, fetch};
use ssx_net_join::browse::ping::{Pinger, TIMEOUT};
use ssx_net_proto::keys::{ServerSecret, to_hex};
use ssx_net_proto::master::{LIST_VERSION, ListedServer, MasterUrl, ServerList};
use ssx_net_proto::query::{Challenge, LAN_PORT, LanAnswer, LanProbe, QueryAnswer, QueryRequest};

static NEXT: AtomicU32 = AtomicU32::new(0);

fn next_ip() -> IpAddr {
    let n = NEXT.fetch_add(1, Ordering::Relaxed) + 1;
    IpAddr::V4(Ipv4Addr::new(127, 0, 60 + (n / 250) as u8, (n % 250) as u8 + 1))
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

fn fake_master(serve: impl Fn(&str) -> (String, String, Vec<u8>) + Send + Sync + 'static) -> MasterUrl {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
    let addr = listener.local_addr().expect("addr");
    let serve = std::sync::Arc::new(serve);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let serve = std::sync::Arc::clone(&serve);
            std::thread::spawn(move || {
                let mut s = stream;
                let head = read_head(&mut s);
                let (status, extra, body) = serve(&head);
                let h =
                    format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len());
                let _ = s.write_all(h.as_bytes());
                let _ = s.write_all(&body);
            });
        }
    });
    MasterUrl::parse(&format!("http://{addr}")).expect("url")
}

fn server(i: u32, key: &str) -> ListedServer {
    ListedServer {
        address: format!("203.0.113.{}", i % 250 + 1),
        port: 27450,
        key: key.into(),
        name: format!("Server {i}"),
        mode: "freeride".into(),
        drop: "exp_alps_vorassay_drop_3".into(),
        map: None,
        players: i % 16,
        max_players: 16,
        password: false,
        region: "eu-central".into(),
        tags: vec![],
        build: ssx_net_proto::BUILD_ID.into(),
        api_level: 0,
        info_version: ssx_net_proto::info::INFO_VERSION,
        info_hash: to_hex(&[0xAB; 32]),
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

fn list_of(servers: Vec<ListedServer>) -> Vec<u8> {
    ServerList { list_version: LIST_VERSION, protocol_version: ssx_net_proto::PROTOCOL_VERSION, servers }.to_json()
}

#[test]
fn two_masters_at_once_a_third_after_the_step_merged_by_key() {
    let keys: Vec<String> = (0..4).map(|_| ServerSecret::generate().public().to_hex()).collect();
    let silent = fake_master(|_| {
        std::thread::sleep(Duration::from_secs(20));
        ("200 OK".into(), String::new(), Vec::new())
    });
    let (k0, k1, k2, k3) = (keys[0].clone(), keys[1].clone(), keys[2].clone(), keys[3].clone());
    let k1b = k1.clone();
    let second = fake_master(move |_| {
        ("200 OK".into(), "ETag: \"a\"\r\n".into(), list_of(vec![server(1, &k0), server(2, &k1)]))
    });
    let third = fake_master(move |_| {
        ("200 OK".into(), String::new(), list_of(vec![server(1, &k2), server(3, &k1b), server(4, &k3)]))
    });
    let mut m = Masters::new(vec![silent.clone(), second.clone(), third.clone()], None);
    let t0 = Instant::now();
    let mut third_seen_at = None;
    while t0.elapsed() < Duration::from_secs(10) {
        m.poll(Instant::now());
        if third_seen_at.is_none() && m.status.iter().any(|(u, _)| *u == third) {
            third_seen_at = Some(t0.elapsed());
        }
        if m.kept.len() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let at = third_seen_at.expect("the third master asked and answered");
    assert!(at >= STEP, "the third is asked after the step, not at once: {at:?}");
    let (merged, left_out) = m.merged();
    let names: Vec<&str> = merged.iter().map(|l| l.server.name.as_str()).collect();
    assert_eq!(names, ["Server 1", "Server 2", "Server 4"], "{merged:?}");
    assert_eq!(left_out, 1);
    let k1 = merged.iter().find(|l| to_hex(&l.key) == keys[1]).expect("k1");
    assert_eq!(k1.masters, [second.clone(), third.clone()]);
    assert!(m.status.iter().all(|(u, _)| *u != silent), "the silent master has not answered");
}

#[test]
fn hostile_masters_are_refused_within_their_time() {
    let check = |url: &MasterUrl, want: &str, within: Duration| {
        let t0 = Instant::now();
        let e = fetch(url, None, &None).expect_err(want);
        assert!(e.contains(want), "{want}: {e}");
        assert!(t0.elapsed() < within, "{want}: {:?}", t0.elapsed());
    };
    let huge =
        fake_master(|_| ("200 OK".into(), String::new(), vec![b' '; ssx_net_proto::master::MAX_LIST_BYTES + 300_000]));
    check(&huge, "larger than", Duration::from_secs(10));
    let ten_mb_junk = fake_master(|_| ("200 OK".into(), String::new(), vec![b'['; 10_000_000]));
    check(&ten_mb_junk, "does not read", Duration::from_secs(10));
    check(
        &fake_master(|_| ("200 OK".into(), String::new(), b"{\"list_version\":1,".to_vec())),
        "does not read",
        Duration::from_secs(5),
    );
    check(
        &fake_master(|_| ("302 Found".into(), "Location: http://10.0.0.1/\r\n".into(), Vec::new())),
        "HTTP 302",
        Duration::from_secs(5),
    );
    check(&fake_master(|_| ("429 Too Many".into(), String::new(), Vec::new())), "429", Duration::from_secs(5));
    let other = ServerList { list_version: LIST_VERSION, protocol_version: 999, servers: vec![] }.to_json();
    check(
        &fake_master(move |_| ("200 OK".into(), String::new(), other.clone())),
        "protocol 999",
        Duration::from_secs(5),
    );
    let silent = fake_master(|_| {
        std::thread::sleep(Duration::from_secs(60));
        ("200 OK".into(), String::new(), Vec::new())
    });
    check(&silent, "no answer within 15 s", Duration::from_secs(17));
    let not_modified = fake_master(|head| {
        if head.to_ascii_lowercase().contains("if-none-match: \"x\"") {
            ("304 Not Modified".into(), String::new(), Vec::new())
        } else {
            ("200 OK".into(), "ETag: \"x\"\r\n".into(), list_of(vec![]))
        }
    });
    let first = fetch(&not_modified, None, &None).expect("the list");
    assert!(matches!(&first, Fetched::List { etag: Some(t), .. } if t == "\"x\""));
    assert_eq!(fetch(&not_modified, Some("\"x\""), &None), Ok(Fetched::NotModified));
}

#[test]
fn an_https_master_with_its_root_and_without() {
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
    let dir = std::env::temp_dir().join(format!("ssx-net-join-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut ca = CertificateParams::new(Vec::<String>::new()).expect("params");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca, KeyPair::generate().expect("key")).expect("CA");
    let key = KeyPair::generate().expect("key");
    let leaf =
        CertificateParams::new(vec!["127.0.0.1".to_string()]).expect("params").signed_by(&key, &ca).expect("leaf");
    std::fs::write(dir.join("chain.pem"), leaf.pem()).expect("write");
    std::fs::write(dir.join("key.pem"), key.serialize_pem()).expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("key.pem"), std::fs::Permissions::from_mode(0o600)).expect("mode");
    }
    let mut config = avalanche_master::config::Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.tls = Some(avalanche_master::config::Tls { certificate: dir.join("chain.pem"), key: dir.join("key.pem") });
    let m = avalanche_master::spawn(avalanche_master::MasterOptions {
        config,
        data_dir: dir.clone(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    })
    .expect("an HTTPS master");
    let url = MasterUrl::parse(&format!("https://127.0.0.1:{}", m.started.addr.port())).expect("url");
    let roots = Some(std::sync::Arc::new(vec![ureq::tls::Certificate::from_der(ca.der()).to_owned()]));
    assert!(matches!(fetch(&url, None, &roots), Ok(Fetched::List { .. })), "with the test root");
    let e = fetch(&url, None, &None).expect_err("the system's roots do not know the test CA");
    assert!(e.contains("cannot be reached"), "{e}");
    let plain = MasterUrl::parse(&format!("http://127.0.0.1:{}", m.started.addr.port())).expect("url");
    assert!(fetch(&plain, None, &None).is_err(), "plain HTTP to an HTTPS master");
    m.stop();
    m.join().expect("stops");
    let _ = std::fs::remove_dir_all(&dir);
}

fn responder(delay: Duration, chaff: bool) -> SocketAddr {
    let ip = next_ip();
    let game_port = 27450;
    let s = UdpSocket::bind(SocketAddr::new(ip, game_port + 1)).expect("udp");
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
            std::thread::sleep(delay);
            match QueryRequest::decode(&buf[..n]) {
                Ok(QueryRequest::Challenge) => {
                    let _ = s.send_to(&Challenge { token: [3; 16] }.encode(), from);
                }
                Ok(QueryRequest::Query { sequence, .. }) => {
                    let mut a = QueryAnswer {
                        sequence,
                        protocol_version: 4,
                        players: 3,
                        max_players: 16,
                        mode: 1,
                        password: false,
                        info_hash: [0; 32],
                    };
                    if chaff {
                        let _ = s.send_to(
                            &(QueryAnswer { sequence: sequence ^ 1, players: 15, ..a }).encode().expect("valid"),
                            from,
                        );
                        let _ = s.send_to(b"junk", from);
                        a.players = 4;
                    }
                    let _ = s.send_to(&a.encode().expect("valid"), from);
                }
                Err(_) => {}
            }
        }
    });
    SocketAddr::new(ip, game_port)
}

#[test]
fn the_ping_is_the_querys_round_trip() {
    let fast = responder(Duration::ZERO, false);
    let slow = responder(Duration::from_millis(60), true);
    let silent = SocketAddr::new(next_ip(), 27450);
    let mut p = Pinger::start().expect("a pinger");
    let now = Instant::now();
    for a in [fast, slow, silent] {
        assert!(p.ask(a, Duration::ZERO, now));
    }
    let mut got = std::collections::HashMap::new();
    let t0 = Instant::now();
    while got.len() < 3 && t0.elapsed() < TIMEOUT + Duration::from_secs(2) {
        for r in p.take() {
            got.insert(r.game, (r.result, t0.elapsed()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (fast_r, _) = &got[&fast];
    let (answer, rtt) = fast_r.as_ref().expect("an answer");
    assert!(*rtt < Duration::from_millis(20), "loopback: {rtt:?}");
    assert_eq!((answer.players, answer.max_players), (3, 16));
    let (slow_r, _) = &got[&slow];
    let (answer, rtt) = slow_r.as_ref().expect("an answer");
    assert_eq!(answer.players, 4, "the answer with the asked sequence, not the chaff");
    assert!(*rtt >= Duration::from_millis(60) && *rtt < Duration::from_millis(66), "{rtt:?}");
    let (silent_r, when) = &got[&silent];
    assert!(silent_r.as_ref().is_err_and(|e| e.contains("within 8 s")), "{silent_r:?}");
    assert!(*when >= TIMEOUT && *when < TIMEOUT + Duration::from_secs(1), "given up after 8 s: {when:?}");
}

#[test]
fn the_lan_search_finds_servers_on_its_targets() {
    let mut targets = Vec::new();
    for port in [27450u16, 27460] {
        let ip = next_ip();
        let s = UdpSocket::bind(SocketAddr::new(ip, LAN_PORT)).expect("udp");
        targets.push(SocketAddr::new(ip, LAN_PORT));
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            loop {
                let Ok((n, from)) = s.recv_from(&mut buf) else { continue };
                if LanProbe::decode(&buf[..n]).is_ok() {
                    let _ = s.send_to(&LanAnswer { protocol: 4, port }.encode(), from);
                    let _ = s.send_to(b"AVALANCHE-SERVER 4 0\n", from);
                }
            }
        });
    }
    let search = LanSearch::start(Targets::Given(targets.clone())).expect("the search");
    let t0 = Instant::now();
    let mut found = Vec::new();
    while found.len() < 2 && t0.elapsed() < Duration::from_secs(3) {
        found.extend(search.take());
        std::thread::sleep(Duration::from_millis(20));
    }
    found.sort_by_key(|f| f.game);
    let want: Vec<SocketAddr> = vec![SocketAddr::new(targets[0].ip(), 27450), SocketAddr::new(targets[1].ip(), 27460)];
    assert_eq!(found.iter().map(|f| f.game).collect::<Vec<_>>(), want);
    std::thread::sleep(Duration::from_millis(2500));
    assert!(search.take().is_empty(), "each server once");
}
