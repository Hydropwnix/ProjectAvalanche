use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use avalanche_master::MasterOptions;
use avalanche_server::config::Config;
use avalanche_server::discovery::Extras;
use avalanche_server::{ServerOptions, ServerThread};
use ssx_net_join::browse::ping::Pinger;

use crate::common::*;

const V6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

fn server_on(bind: IpAddr, edit: impl FnOnce(&mut Config)) -> Option<ServerThread> {
    std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).ok()?;
    let mut config = crate::common::defaults();
    config.bind = bind;
    config.port = 0;
    edit(&mut config);
    let options = ServerOptions {
        config,
        data_dir: temp_dir("data6"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
        extend: Some(Box::new(|app| {
            app.insert_resource(Extras { interval: Some(Duration::from_secs(1)), ..Extras::default() });
        })),
    };
    Some(avalanche_server::spawn(options).expect("the server starts"))
}

fn get6(server: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect_timeout(&server, Duration::from_secs(2)).expect("connect over IPv6");
    s.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
    let _ = s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes());
    let mut answer = Vec::new();
    let _ = s.read_to_end(&mut answer);
    parse(&answer)
}

#[test]
fn a_join_over_ipv6_connects_and_is_counted() {
    let Some(server) = server_on(V6, |_| {}) else { return };
    let addr = server.started.addr;
    assert_eq!(addr.ip(), V6);
    let (status, body) = get6(addr, "/info");
    assert_eq!(status, 200);
    let info = ssx_net_proto::info::ServerInfo::from_json(&body).expect("/info over IPv6");
    assert_eq!(info.identity_key, server.started.identity_key.to_hex());
    let mut player = Player::new();
    let granted = player.join(addr, "Psymon", "").expect("joined over ::1");
    let (mut app, entity) = udp_client(&granted.token, addr, V6, None);
    assert!(
        step_until(&mut [&mut app], Duration::from_secs(5), |a| connected(a[0], entity)),
        "the UDP connect over ::1"
    );
    let counted = step_until(&mut [&mut app], Duration::from_secs(2), |_| {
        ssx_net_proto::info::ServerInfo::from_json(&get6(addr, "/info").1).is_ok_and(|i| i.players == 1)
    });
    assert!(counted, "the rider is counted");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn ipv4_peers_of_an_ipv6_socket_join_and_connect() {
    let mapped = IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped());
    let Some(server) = server_on(mapped, |_| {}) else { return };
    let port = server.started.addr.port();
    let v4 = SocketAddr::new(LOCAL, port);
    let mut player = Player::new();
    let granted = player.join(v4, "Moby", "").expect("joined over IPv4 to the IPv6 socket");
    let (mut app, entity) = udp_client(&granted.token, v4, LOCAL, None);
    assert!(
        step_until(&mut [&mut app], Duration::from_secs(5), |a| connected(a[0], entity)),
        "the ticket's address check"
    );
    let counted = step_until(&mut [&mut app], Duration::from_secs(2), |_| {
        ssx_net_proto::info::ServerInfo::from_json(&get(v4, LOCAL, "/info").1).is_ok_and(|i| i.players == 1)
    });
    assert!(counted, "the rider is counted");
    server.stop();
    server.join().expect("stops");
}

#[test]
fn the_query_the_ping_and_a_master_over_ipv6() {
    let mut config = avalanche_master::config::Config::defaults();
    config.bind = V6;
    config.port = 0;
    config.allow_private = true;
    if std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).is_err() {
        return;
    }
    let master = avalanche_master::spawn(MasterOptions {
        config,
        data_dir: std::env::temp_dir(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    })
    .expect("a master on ::1");
    let url = format!("http://[::1]:{}", master.started.addr.port());
    let Some(server) = server_on(V6, |c| c.discovery = listed_on(&url)) else {
        return;
    };
    let game = server.started.addr;
    let key = server.started.identity_key.to_hex();
    let end = Instant::now() + Duration::from_secs(15);
    let listed = loop {
        let r = master.started.master.registry.lock().expect("registry");
        let found = r.iter().find(|l| ssx_net_proto::keys::to_hex(&l.key) == key).map(|l| (l.ip, l.beat.port));
        drop(r);
        if found.is_some() || Instant::now() > end {
            break found;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    assert_eq!(listed, Some((V6, game.port())), "the master checked /info and the query over ::1 and listed it");
    let mut pinger = Pinger::start().expect("the browser's pinger");
    assert!(pinger.ask(game, Duration::from_secs(10), Instant::now()));
    let end = Instant::now() + Duration::from_secs(8);
    let ping = loop {
        if let Some(p) = pinger.take().into_iter().next() {
            break Some(p);
        }
        if Instant::now() > end {
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let ping = ping.expect("a ping result");
    let (answer, rtt) = ping.result.expect("the query answered over ::1");
    assert!(answer.max_players > 0 && rtt < Duration::from_secs(1), "{rtt:?}");
    server.stop();
    server.join().expect("stops");
    master.stop();
    master.join().expect("stops");
}
