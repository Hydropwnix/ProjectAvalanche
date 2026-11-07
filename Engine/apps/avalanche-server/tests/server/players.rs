use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use ssx_net_proto::players::PlayerList;

use crate::common::*;

fn names(server: std::net::SocketAddr, from: IpAddr) -> Option<Vec<String>> {
    let (status, body) = get(server, from, "/players");
    (status == 200).then(|| PlayerList::from_json(&body).expect("the list reads strictly").players)
}

#[test]
fn the_list_follows_the_riders_and_carries_names_only() {
    let server = server();
    let addr = server.started.addr;
    let watcher = IpAddr::V4(Ipv4Addr::new(127, 0, 9, 1));
    assert_eq!(names(addr, watcher), Some(vec![]));
    let (mut alex, mut mac) = (Player::new(), Player::new());
    let a = alex.join(addr, "Alex", "").expect("Alex joins");
    let (mut app_a, ea) = udp_client(&a.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut app_a], Duration::from_secs(5), |x| connected(x[0], ea)));
    let m = mac.join(addr, "  Mac  Fraser ", "").expect("Mac joins");
    let (mut app_m, em) = udp_client(&m.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut app_a, &mut app_m], Duration::from_secs(5), |x| connected(x[1], em)));
    let both = step_until(&mut [&mut app_a, &mut app_m], Duration::from_secs(3), |_| {
        names(addr, watcher).is_some_and(|n| n == ["Alex", "Mac Fraser"])
    });
    assert!(both, "both riders, in the order their sessions began, as the name rule leaves them");
    let (_, body) = get(addr, watcher, "/players");
    let text = String::from_utf8_lossy(&body);
    let key = server.started.identity_key.to_hex();
    assert!(!text.contains(&key) && !text.contains("127.0.0.1"), "no key and no address: {text}");
    drop(app_a);
    let left = step_until(&mut [&mut app_m], Duration::from_secs(15), |_| {
        names(addr, watcher).is_some_and(|n| n == ["Mac Fraser"])
    });
    assert!(left, "a rider whose connection ended is no longer listed");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn the_rate_is_infos_in_a_bucket_of_its_own() {
    let server = server();
    let addr = server.started.addr;
    let a = IpAddr::V4(Ipv4Addr::new(127, 0, 9, 2));
    let lists: Vec<u16> = (0..9).map(|_| get(addr, a, "/players").0).collect();
    assert_eq!(lists[..8], [200; 8], "the burst of 8");
    assert_eq!(lists[8], 429, "the ninth at once");
    assert_eq!(get(addr, a, "/info").0, 200, "/info keeps its own budget");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn hostile_requests_get_the_limits_every_endpoint_has() {
    use avalanche_server::http::HEAD_TIMEOUT;
    let server = server();
    let addr = server.started.addr;
    let b = IpAddr::V4(Ipv4Addr::new(127, 0, 9, 3));
    let big = format!("GET /players HTTP/1.1\r\nHost: x\r\nX-Pad: {}\r\n\r\n", "a".repeat(9000));
    assert_eq!(raw(addr, b, big.as_bytes(), Duration::from_secs(3)).0, 431, "a head above 8 KiB");
    let query = raw(addr, b, b"GET /players?all=1&keys=1 HTTP/1.1\r\nHost: x\r\n\r\n", Duration::from_secs(3));
    assert_eq!(query.0, 200, "a query string changes nothing");
    assert_eq!(PlayerList::from_json(&query.1).map(|l| l.players.len()), Ok(0));
    let started = std::time::Instant::now();
    let _ = raw(
        addr,
        IpAddr::V4(Ipv4Addr::new(127, 0, 9, 4)),
        b"GET /players HTTP/1.1\r\nHost: x\r\n",
        Duration::from_secs(30),
    );
    let cut = started.elapsed();
    assert!(cut >= HEAD_TIMEOUT && cut < HEAD_TIMEOUT + Duration::from_secs(2), "a head never finished: {cut:?}");
    server.stop();
    server.join().expect("stops");
}
