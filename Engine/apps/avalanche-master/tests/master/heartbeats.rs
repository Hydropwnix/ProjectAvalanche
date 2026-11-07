use ssx_net_proto::keys::{ServerSecret, to_hex};
use ssx_net_proto::master::{ServerList, seal_heartbeat_json};
use ssx_net_proto::query::info_hash;

use crate::common::*;

fn list(master: std::net::SocketAddr) -> ServerList {
    let rt = runtime();
    let path = format!("/v1/servers?protocol={}", ssx_net_proto::PROTOCOL_VERSION);
    let (status, _, body) = rt.block_on(exchange(master, next_ip(), "GET", &path, vec![], &[]));
    assert_eq!(status, 200);
    ServerList::from_json(&body, ssx_net_proto::PROTOCOL_VERSION).expect("the list reads")
}

fn listed(got: &Got) -> bool {
    matches!(got, Got::Answer(a) if a.listed)
}

fn reason(got: &Got) -> String {
    match got {
        Got::Answer(a) => a.reason.clone(),
        Got::Plain(s, t) => format!("{s} {t}"),
    }
}

#[test]
fn an_honest_server_is_listed_and_leaves() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let fake = Fake::start("Alps Night Riders", Query::Honest, true);
    let rt = runtime();
    let got = rt.block_on(heartbeat(addr, &fake, |_| {}));
    assert!(listed(&got), "{got:?}");
    let l = list(addr);
    assert_eq!(l.servers.len(), 1);
    let s = &l.servers[0];
    assert_eq!((s.address.as_str(), s.port), (fake.ip.to_string().as_str(), fake.port));
    assert_eq!(s.key, fake.identity.public().to_hex(), "the key the heartbeat was boxed with");
    assert_eq!(s.info_hash, to_hex(&info_hash(&fake.info.lock().unwrap())));
    assert_eq!((s.region.as_str(), s.tags.as_slice()), ("eu-central", &["race".to_string()][..]));
    let got = rt.block_on(heartbeat(addr, &fake, |b| b.players = 7));
    assert!(listed(&got));
    std::thread::sleep(1100 * SECOND / 1000);
    assert_eq!(list(addr).servers[0].players, 7);
    let got = rt.block_on(heartbeat(addr, &fake, |b| b.leaving = true));
    assert!(!listed(&got) && reason(&got).contains("stops"), "{got:?}");
    std::thread::sleep(1100 * SECOND / 1000);
    assert!(list(addr).servers.is_empty());
    m.stop();
    m.join().expect("stops");
}

#[test]
fn forged_and_replayed_heartbeats_are_refused() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let victim = Fake::start("Victim", Query::Honest, true);
    let rt = runtime();
    let c = rt.block_on(challenge(addr, victim.ip));
    let forger = ServerSecret::generate();
    let mut body = victim.beat().seal(&forger, &c).expect("sealed");
    body[5..37].copy_from_slice(victim.identity.public().as_bytes());
    let got = rt.block_on(post(addr, victim.ip, body, &victim.identity, &c));
    assert!(matches!(&got, Got::Plain(400, t) if t.contains("does not open")), "{got:?}");
    let c = rt.block_on(challenge(addr, victim.ip));
    let body = victim.beat().seal(&victim.identity, &c).expect("sealed");
    assert!(listed(&rt.block_on(post(addr, victim.ip, body.clone(), &victim.identity, &c))));
    let got = rt.block_on(post(addr, victim.ip, body.clone(), &victim.identity, &c));
    assert!(matches!(&got, Got::Plain(400, t) if t.contains("used")), "a replay: {got:?}");
    let elsewhere = next_ip();
    let got = rt.block_on(post(addr, elsewhere, body.clone(), &victim.identity, &c));
    assert!(
        matches!(&got, Got::Plain(400, t) if t.contains("not for this address") || t.contains("not this master")),
        "{got:?}"
    );
    let c = rt.block_on(challenge(addr, victim.ip));
    let mut changed = victim.beat().seal(&victim.identity, &c).expect("sealed");
    let last = changed.len() - 30;
    changed[last] ^= 0x20;
    let got = rt.block_on(post(addr, victim.ip, changed, &victim.identity, &c));
    assert!(matches!(&got, Got::Plain(400, _)), "{got:?}");
    let other = master_with(|_| {});
    let c = rt.block_on(challenge(other.started.addr, victim.ip));
    let body = victim.beat().seal(&victim.identity, &c).expect("sealed");
    let got = rt.block_on(post(addr, victim.ip, body, &victim.identity, &c));
    assert!(matches!(&got, Got::Plain(400, _)), "{got:?}");
    let c = rt.block_on(challenge(addr, victim.ip));
    let json = br#"{"heartbeat_version":1,"port":0}"#;
    let body = seal_heartbeat_json(json, &victim.identity, &c).expect("sealed");
    let got = rt.block_on(post(addr, victim.ip, body, &victim.identity, &c));
    assert!(matches!(&got, Got::Answer(a) if !a.listed && a.reason.contains("heartbeat")), "{got:?}");
    assert_eq!(list(addr).servers.len(), 1, "the victim stays listed, nothing else");
    for t in [m, other] {
        t.stop();
        t.join().expect("stops");
    }
}

#[test]
fn unreachable_servers_are_not_listed_and_told_why() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let closed = Fake::start("No TCP", Query::Honest, false);
    let got = rt.block_on(heartbeat(addr, &closed, |_| {}));
    assert!(!listed(&got) && reason(&got).contains("TCP") && reason(&got).contains("forwarded"), "{got:?}");
    let silent = Fake::start("No UDP", Query::Silent, true);
    let t0 = std::time::Instant::now();
    let got = rt.block_on(heartbeat(addr, &silent, |_| {}));
    assert!(!listed(&got) && reason(&got).contains("UDP") && reason(&got).contains("query = true"), "{got:?}");
    assert!(t0.elapsed() < 10 * SECOND, "the check is bounded: {:?}", t0.elapsed());
    assert!(list(addr).servers.is_empty());
    let went = Fake::start("Goes Silent", Query::Honest, true);
    assert!(listed(&rt.block_on(heartbeat(addr, &went, |_| {}))));
    *went.query.lock().unwrap() = Query::Silent;
    went.info.lock().unwrap().tags = vec!["trick".into()];
    let got = rt.block_on(heartbeat(addr, &went, |b| b.tags = vec!["trick".into()]));
    assert!(!listed(&got) && reason(&got).contains("UDP"), "{got:?}");
    std::thread::sleep(1100 * SECOND / 1000);
    assert!(list(addr).servers.is_empty(), "taken off");
    m.stop();
    m.join().expect("stops");
}

#[test]
fn servers_lying_in_their_info_or_query_are_refused() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let liar = Fake::start("Honest Name", Query::Honest, true);
    let got = rt.block_on(heartbeat(addr, &liar, |b| b.name = "Another Name".into()));
    assert!(reason(&got).contains("name"), "{got:?}");
    let got = rt.block_on(heartbeat(addr, &liar, |b| b.max_players = 32));
    assert!(reason(&got).contains("max players"), "{got:?}");
    liar.info.lock().unwrap().identity_key = ServerSecret::generate().public().to_hex();
    let got = rt.block_on(heartbeat(addr, &liar, |_| {}));
    assert!(reason(&got).contains("identity key"), "{got:?}");
    let hash = Fake::start("Wrong Hash", Query::WrongHash, true);
    assert!(reason(&rt.block_on(heartbeat(addr, &hash, |_| {}))).contains("info hash"));
    let max = Fake::start("Wrong Max", Query::WrongMax, true);
    assert!(reason(&rt.block_on(heartbeat(addr, &max, |_| {}))).contains("player count"));
    let big = Fake::start("Big Info", Query::Honest, true);
    *big.info_body.lock().unwrap() = Some(vec![b'{'; 100_000]);
    assert!(reason(&rt.block_on(heartbeat(addr, &big, |_| {}))).contains("larger than"));
    *big.info_body.lock().unwrap() = Some(b"not json".to_vec());
    assert!(reason(&rt.block_on(heartbeat(addr, &big, |_| {}))).contains("/info"));
    assert!(list(addr).servers.is_empty());
    m.stop();
    m.join().expect("stops");
}

#[test]
fn private_addresses_blocked_keys_and_the_cap_per_address() {
    let rt = runtime();
    let public = master_with(|c| c.allow_private = false);
    let fake = Fake::start("Loopback", Query::Honest, true);
    let got = rt.block_on(heartbeat(public.started.addr, &fake, |_| {}));
    assert!(matches!(&got, Got::Plain(403, t) if t.contains("loopback")), "{got:?}");
    public.stop();
    public.join().expect("stops");

    let dir = std::env::temp_dir().join(format!("avalanche-master-test-blocked-{}", random_u32()));
    std::fs::create_dir_all(&dir).expect("dir");
    let blocked = Fake::start("Blocked", Query::Honest, true);
    let file = dir.join("blocked.txt");
    std::fs::write(
        &file,
        format!("Project Avalanche blocked servers v1\n{} spam\n", blocked.identity.public().to_hex()),
    )
    .expect("write");
    let m = master_with(|c| {
        c.blocked_servers = Some(file.clone());
        c.max_per_address = 1;
    });
    let got = rt.block_on(heartbeat(m.started.addr, &blocked, |_| {}));
    assert!(!listed(&got) && reason(&got).contains("blocked") && reason(&got).contains("spam"), "{got:?}");
    let first = Fake::start("First", Query::Honest, true);
    assert!(listed(&rt.block_on(heartbeat(m.started.addr, &first, |_| {}))));
    let second = Fake::start("Second", Query::Honest, true);
    let c = rt.block_on(challenge(m.started.addr, first.ip));
    let mut beat = second.beat();
    beat.port = first.port + 10;
    let body = beat.seal(&second.identity, &c).expect("sealed");
    let got = rt.block_on(post(m.started.addr, first.ip, body, &second.identity, &c));
    assert!(reason(&got).contains("at most 1"), "{got:?}");
    let _ = std::fs::remove_dir_all(&dir);
    m.stop();
    m.join().expect("stops");
}

#[test]
fn the_check_goes_to_the_heartbeats_source_only() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let elsewhere = Fake::start("Elsewhere", Query::Honest, true);
    let sender = next_ip();
    let c = rt.block_on(challenge(addr, sender));
    let body = elsewhere.beat().seal(&elsewhere.identity, &c).expect("sealed");
    let got = rt.block_on(post(addr, sender, body, &elsewhere.identity, &c));
    let said = reason(&got);
    assert!(!listed(&got), "{got:?}");
    assert!(said.contains(&format!("{sender}:{}", elsewhere.port)), "checked at the sender: {said}");
    assert!(!said.contains(&elsewhere.ip.to_string()), "never at the other address: {said}");
    assert_eq!(elsewhere.contacted.load(std::sync::atomic::Ordering::Relaxed), 0, "nothing reached the other server");
    assert!(list(addr).servers.is_empty());
    m.stop();
    m.join().expect("stops");
}
