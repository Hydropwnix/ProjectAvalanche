use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avalanche_master::config::Config;
use avalanche_master::names::{HostName, Resolve};
use avalanche_master::trust::{REFRESH, STALE_AFTER};
use avalanche_master::{MasterOptions, MasterThread};

use crate::common::*;

const OLD: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 10, 1));
const NEW: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 10, 2));

struct Dns {
    answer: Mutex<Result<Vec<IpAddr>, String>>,
    asked: AtomicUsize,
}

impl Dns {
    fn new(answer: Result<Vec<IpAddr>, String>) -> Arc<Self> {
        Arc::new(Self { answer: Mutex::new(answer), asked: AtomicUsize::new(0) })
    }

    fn set(&self, answer: Result<Vec<IpAddr>, String>) {
        *self.answer.lock().expect("the answer") = answer;
    }
}

impl Resolve for Dns {
    fn resolve(&self, name: &HostName) -> Result<Vec<IpAddr>, String> {
        assert_eq!(name.as_str(), "coolify-proxy", "only the configured name is looked up");
        self.asked.fetch_add(1, Ordering::Relaxed);
        self.answer.lock().expect("the answer").clone()
    }
}

fn master_named(dns: &Arc<Dns>) -> MasterThread {
    let mut config = Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.allow_private = true;
    config.trusted_hosts = vec![HostName::parse("coolify-proxy").expect("a name")];
    let options = MasterOptions {
        config,
        data_dir: std::env::temp_dir(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    };
    avalanche_master::spawn_with_resolver(options, Arc::clone(dns) as Arc<dyn Resolve>).expect("the master starts")
}

fn trusted(addr: SocketAddr, from: IpAddr) -> bool {
    let headers = [("x-forwarded-for", "unknown")];
    match runtime().block_on(exchange(addr, from, "GET", "/v1/challenge", vec![], &headers)).0 {
        400 => true,
        200 => false,
        other => panic!("{from}: {other}"),
    }
}

fn until(bound: Duration, mut want: impl FnMut() -> bool) -> Option<Duration> {
    let start = Instant::now();
    while start.elapsed() < bound {
        if want() {
            return Some(start.elapsed());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    None
}

fn stop(m: MasterThread) {
    m.stop();
    m.join().expect("stops");
}

#[test]
fn a_proxy_trusted_by_name_names_its_clients_from_the_first_connection() {
    let dns = Dns::new(Ok(vec![OLD]));
    let m = master_named(&dns);
    let addr = m.started.addr;
    assert!(dns.asked.load(Ordering::Relaxed) >= 1, "looked up before the master took a connection");
    let (status, _, _) =
        runtime().block_on(exchange(addr, OLD, "GET", "/v1/challenge", vec![], &[("x-forwarded-for", "203.0.113.5")]));
    assert_eq!(status, 200);
    assert_eq!(m.started.master.stats.proxied.load(Ordering::Relaxed), 1, "the client named by the proxy");
    assert!(trusted(addr, OLD), "the name's address");
    assert!(!trusted(addr, NEW), "any other peer is its own client");
    stop(m);
}

#[test]
fn the_proxy_moved_is_trusted_at_its_new_address_within_a_refresh_and_no_longer_at_its_old() {
    let dns = Dns::new(Ok(vec![OLD]));
    let m = master_named(&dns);
    let addr = m.started.addr;
    assert!(trusted(addr, OLD) && !trusted(addr, NEW));
    dns.set(Ok(vec![NEW]));
    let took = until(REFRESH * 3, || trusted(addr, NEW)).expect("the new address trusted");
    assert!(took <= REFRESH + Duration::from_millis(800), "{took:?}");
    assert!(!trusted(addr, OLD), "the old address no longer");
    stop(m);
}

#[test]
fn a_name_answering_an_internet_address_trusts_nothing() {
    let dns = Dns::new(Ok(vec![OLD, "8.8.8.8".parse().expect("an address")]));
    let m = master_named(&dns);
    let addr = m.started.addr;
    assert!(!trusted(addr, OLD), "the whole answer refused");
    dns.set(Ok(vec![OLD]));
    until(REFRESH * 3, || trusted(addr, OLD)).expect("trusted once the answer is private");
    dns.set(Ok(vec![OLD, "2a00:1450:4001::200e".parse().expect("an address")]));
    let took = until(REFRESH * 3, || !trusted(addr, OLD)).expect("refused at once");
    assert!(took <= REFRESH + Duration::from_millis(800), "{took:?}");
    stop(m);
}

#[test]
fn a_failing_lookup_keeps_the_trust_no_longer_than_its_bound() {
    let dns = Dns::new(Ok(vec![OLD]));
    let m = master_named(&dns);
    let addr = m.started.addr;
    assert!(trusted(addr, OLD));
    dns.set(Err("Temporary failure in name resolution".into()));
    let failing_since = Instant::now();
    let took = until(STALE_AFTER + REFRESH * 2, || !trusted(addr, OLD)).expect("the trust ends");
    assert!(took + REFRESH + Duration::from_millis(200) >= STALE_AFTER, "ended early: {took:?}");
    assert!(failing_since.elapsed() <= STALE_AFTER + Duration::from_millis(800), "kept too long: {took:?}");
    dns.set(Ok(vec![OLD]));
    until(REFRESH * 3, || trusted(addr, OLD)).expect("trusted again");
    stop(m);
}
