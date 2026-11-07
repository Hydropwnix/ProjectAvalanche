use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ssx_net_portmap::fake::{self, IgdMode, PmpMode};
use ssx_net_portmap::{Config, Level, Mapper, Method, Port, Protocol, SsdpTarget, Status, Targets};

fn ports() -> Vec<Port> {
    vec![
        Port { protocol: Protocol::Udp, port: 27450 },
        Port { protocol: Protocol::Tcp, port: 27450 },
        Port { protocol: Protocol::Udp, port: 27451 },
    ]
}

type Lines = Arc<Mutex<Vec<String>>>;

fn start(methods: Vec<Method>, gateway: Ipv4Addr, targets: Targets, lifetime: u32) -> (Mapper, Lines) {
    assert!(targets.loopback_only(gateway), "a test's targets must all be on loopback");
    let lines: Lines = Arc::default();
    let l2 = Arc::clone(&lines);
    let cfg = Config {
        methods,
        gateway: Some(gateway),
        lifetime,
        local: Some(Ipv4Addr::LOCALHOST),
        ports: ports(),
        description: "Project Avalanche: test".into(),
    };
    let log = Arc::new(move |level: Level, line: &str| l2.lock().unwrap().push(format!("{level:?} {line}")));
    (Mapper::start(cfg, targets, log).expect("started"), lines)
}

fn targets(pmp_port: u16, ssdp: Option<std::net::SocketAddrV4>) -> Targets {
    let ssdp = ssdp.unwrap_or_else(|| "127.0.0.1:9".parse().unwrap());
    Targets { ssdp: SsdpTarget::Unicast(ssdp), pmp_port }
}

fn wait_for(status: &Mutex<Status>, within: Duration, f: impl Fn(&Status) -> bool) -> Status {
    let until = Instant::now() + within;
    loop {
        let s = status.lock().unwrap().clone();
        if f(&s) || Instant::now() > until {
            return s;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn natpmp_maps_renews_and_removes() {
    let gw = Ipv4Addr::new(127, 0, 41, 1);
    let g = fake::pmp(gw, PmpMode::NatPmp, 2);
    let (m, lines) = start(vec![Method::NatPmp], gw, targets(g.addr.port(), None), 2);
    let status = m.status();
    let s = wait_for(&status, Duration::from_secs(5), |s| s.renewals >= 2);
    assert!(s.mapped && s.renewals >= 2, "{s:?}");
    assert_eq!(s.external, Some("203.0.113.5".parse().unwrap()));
    m.stop();
    let seen = g.seen.lock().unwrap().clone();
    let removed: Vec<_> = seen.iter().filter(|(_, _, life)| *life == 0).collect();
    assert_eq!(removed.len(), 3, "every port removed at the stop: {seen:?}");
    assert!(seen.iter().filter(|(_, _, l)| *l == 2).count() >= 9, "made and renewed twice: {seen:?}");
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("removed UDP 27450, TCP 27450, UDP 27451")));
}

#[test]
fn pcp_maps_and_a_natpmp_gateway_is_told_apart() {
    let gw = Ipv4Addr::new(127, 0, 41, 2);
    let g = fake::pmp(gw, PmpMode::Pcp, 4);
    let (m, _) = start(vec![Method::Pcp], gw, targets(g.addr.port(), None), 4);
    let s = wait_for(&m.status(), Duration::from_secs(5), |s| s.mapped);
    assert_eq!((s.mapped, s.method, s.lifetime), (true, Some(Method::Pcp), 4), "{s:?}");
    m.stop();
    assert_eq!(g.seen.lock().unwrap().iter().filter(|(_, _, l)| *l == 0).count(), 3, "removed with their nonces");
    let gw = Ipv4Addr::new(127, 0, 41, 3);
    let g = fake::pmp(gw, PmpMode::NatPmp, 3600);
    let started = Instant::now();
    let (m, _) = start(vec![Method::Pcp, Method::NatPmp], gw, targets(g.addr.port(), None), 3600);
    let s = wait_for(&m.status(), Duration::from_secs(5), |s| s.mapped);
    assert_eq!(s.method, Some(Method::NatPmp), "{s:?}");
    assert!(started.elapsed() < Duration::from_secs(3), "no PCP retransmission to a NAT-PMP gateway");
    m.stop();
}

#[test]
fn upnp_maps_renews_and_removes_and_takes_permanent_leases() {
    let gw = Ipv4Addr::new(127, 0, 42, 1);
    let g = fake::igd(gw, IgdMode::Normal, Some(Ipv4Addr::new(127, 0, 42, 9)));
    let (m, lines) = start(vec![Method::Upnp], gw, targets(9, Some(g.ssdp)), 2);
    let s = wait_for(&m.status(), Duration::from_secs(8), |s| s.renewals >= 1);
    assert!(s.mapped && s.renewals >= 1, "{s:?}");
    assert_eq!((s.method, s.external), (Some(Method::Upnp), Some("203.0.113.5".parse().unwrap())));
    m.stop();
    let seen = g.seen.lock().unwrap().clone();
    assert!(seen.iter().filter(|(a, _, l)| a.starts_with("add") && *l == 2).count() >= 6, "{seen:?}");
    assert_eq!(seen.iter().filter(|(a, _, _)| a.starts_with("delete")).count(), 3, "{seen:?}");
    assert!(lines.lock().unwrap().iter().any(|l| l.contains("UPnP IGD at 127.0.42.1 (WANIPConnection")));
    assert!(
        g.contacted.lock().unwrap().iter().all(|r| r.starts_with("GET /rootDesc.xml") || r.starts_with("POST /ctl"))
    );
    let gw = Ipv4Addr::new(127, 0, 42, 2);
    let g = fake::igd(gw, IgdMode::PermanentOnly, None);
    let (m, _) = start(vec![Method::Upnp], gw, targets(9, Some(g.ssdp)), 3600);
    let s = wait_for(&m.status(), Duration::from_secs(8), |s| s.mapped);
    assert_eq!((s.mapped, s.lifetime), (true, 0), "{s:?}");
    m.stop();
    let seen = g.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().filter(|(a, _, l)| a.starts_with("add") && *l == 0).count(), 3, "{seen:?}");
    assert_eq!(seen.iter().filter(|(a, _, _)| a.starts_with("delete")).count(), 3);
}

fn refused(methods: Vec<Method>, gw: Ipv4Addr, t: Targets, within: Duration) -> (String, Duration) {
    let started = Instant::now();
    let (m, _) = start(methods, gw, t, 3600);
    let s = wait_for(&m.status(), within, |s| s.failures > 0 || s.mapped);
    let took = started.elapsed();
    assert!(!s.mapped, "{s:?}");
    m.stop();
    (s.last_error.unwrap_or_default(), took)
}

#[test]
fn hostile_igds_are_refused_with_their_reason() {
    let cases = [
        (IgdMode::Huge, "larger than 65536", 8),
        (IgdMode::Never, "no answer in time", 9),
        (IgdMode::Entities, "DOCTYPE", 6),
        (IgdMode::Redirect, "a redirect is never followed", 6),
        (IgdMode::OtherHost, "not on the gateway", 6),
        (IgdMode::Conflict, "718 ConflictInMappingEntry", 6),
        (IgdMode::Slow, "no answer in time", 9),
    ];
    std::thread::scope(|scope| {
        for (i, (mode, why, secs)) in cases.into_iter().enumerate() {
            scope.spawn(move || {
                let gw = Ipv4Addr::new(127, 0, 43, 1 + i as u8);
                let g = fake::igd(gw, mode, None);
                let (reason, took) =
                    refused(vec![Method::Upnp], gw, targets(9, Some(g.ssdp)), Duration::from_secs(secs));
                assert!(reason.contains(why), "{mode:?}: {reason}");
                assert!(took < Duration::from_secs(secs), "{mode:?} took {took:?}");
                assert!(!reason.chars().any(char::is_control) && reason.chars().count() <= 400, "{mode:?}: bounded");
            });
        }
    });
}

#[test]
fn hostile_pmp_gateways_are_refused_with_their_reason() {
    let cases = [
        (PmpMode::Garbage, Method::NatPmp, "no usable answer", 6),
        (PmpMode::OtherPort, Method::NatPmp, "the port must be the same outside", 6),
        (PmpMode::Refuse, Method::NatPmp, "not authorized or refused", 6),
        (PmpMode::WrongNonce, Method::Pcp, "nonce is not this request's", 12),
    ];
    std::thread::scope(|scope| {
        for (i, (mode, method, why, secs)) in cases.into_iter().enumerate() {
            scope.spawn(move || {
                let gw = Ipv4Addr::new(127, 0, 44, 1 + i as u8);
                let g = fake::pmp(gw, mode, 3600);
                let (reason, _) = refused(vec![method], gw, targets(g.addr.port(), None), Duration::from_secs(secs));
                assert!(reason.contains(why), "{mode:?}: {reason}");
                if mode == PmpMode::OtherPort {
                    let seen = g.seen.lock().unwrap().clone();
                    assert!(seen.iter().any(|(_, _, l)| *l == 0), "{seen:?}");
                }
            });
        }
    });
}

#[test]
fn a_silent_gateway_ends_every_method_in_its_bound() {
    let gw = Ipv4Addr::new(127, 0, 45, 1);
    let (reason, took) = refused(Method::ALL.to_vec(), gw, targets(9, None), Duration::from_secs(20));
    assert!(
        reason.contains("UPnP IGD: SSDP: no gateway answered") && reason.contains("PCP") && reason.contains("NAT-PMP"),
        "{reason}"
    );
    assert!(took < Duration::from_secs(16), "{took:?}");
}
