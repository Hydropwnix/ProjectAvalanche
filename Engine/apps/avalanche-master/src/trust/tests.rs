use std::sync::mpsc;

use super::*;
use crate::route::{RouteError, StartLine, gateway_lines, start_line};

fn ip(s: &str) -> IpAddr {
    s.parse().expect("an address")
}

fn name(s: &str) -> HostName {
    HostName::parse(s).expect("a name")
}

fn trusted(addrs: &[&str], until: Instant) -> NameState {
    NameState::Trusted { addrs: addrs.iter().map(|a| ip(a)).collect(), until, failing: None }
}

#[test]
fn an_answer_is_trusted_for_its_bound_and_a_failure_keeps_nothing_beyond_it() {
    let t0 = Instant::now();
    let first = next_state(&NameState::Pending, Ok(vec![ip("10.0.1.5")]), t0);
    assert_eq!(first, trusted(&["10.0.1.5"], t0 + STALE_AFTER));
    let t1 = t0 + REFRESH;
    let kept = next_state(&first, Err("timed out".into()), t1);
    assert_eq!(
        kept,
        NameState::Trusted { addrs: vec![ip("10.0.1.5")], until: t0 + STALE_AFTER, failing: Some("timed out".into()) }
    );
    let at_bound = next_state(&kept, Err("timed out".into()), t0 + STALE_AFTER);
    assert_eq!(at_bound, NameState::Unresolved("timed out".into()));
    let moved = next_state(&first, Ok(vec![ip("10.0.1.9")]), t1);
    assert_eq!(moved, trusted(&["10.0.1.9"], t1 + STALE_AFTER));
    let refused = next_state(&first, Ok(vec![ip("10.0.1.5"), ip("8.8.8.8")]), t1);
    assert_eq!(refused, NameState::Refused("8.8.8.8 is an address the Internet routes to".into()));
    assert_eq!(next_state(&refused, Err("x".into()), t1), NameState::Unresolved("x".into()));
    let NameState::Unresolved(why) = next_state(&NameState::Pending, Err(format!("\u{1b}[2J{}", "e".repeat(999))), t0)
    else {
        panic!("unresolved");
    };
    assert!(why.len() < 200 && !why.contains('\u{1b}'), "{why}");
}

#[test]
fn the_set_trusts_a_names_addresses_only_within_their_bound() {
    let t0 = Instant::now();
    let net = Network::parse("10.0.9.0/24").expect("a network");
    let mut set = TrustSet::new(vec![net], vec![name("coolify-proxy"), name("other")]);
    assert_eq!(set.holder_at(ip("10.0.9.7"), t0), Some(Holder::Network(&net)));
    assert_eq!(set.holder_at(ip("10.0.1.5"), t0), None, "pending: nothing of the name");
    set.names[0].1 = trusted(&["10.0.1.5", "fd00::5"], t0 + STALE_AFTER);
    set.names[1].1 = NameState::Refused("x".into());
    let proxy = name("coolify-proxy");
    for peer in ["10.0.1.5", "::ffff:10.0.1.5", "fd00::5"] {
        assert_eq!(set.holder_at(ip(peer), t0), Some(Holder::Name(&proxy)), "{peer}");
    }
    assert_eq!(set.holder_at(ip("10.0.1.6"), t0), None);
    assert_eq!(set.holder_at(ip("10.0.1.5"), t0 + STALE_AFTER), None, "the bound is checked where it is used");
    assert_eq!(
        set.describe(t0),
        "10.0.9.0/24, coolify-proxy (10.0.1.5, fd00::5), other (refused)",
        "the start's, check's and status's list"
    );
    assert!(set.describe(t0 + STALE_AFTER).contains("coolify-proxy (its addresses expired)"));
    let moment = Moment { set: Arc::new(set), at: t0 };
    assert!(moment.trusts(ip("10.0.1.5")) && !moment.trusts(ip("10.0.1.7")));
    let lines = gateway_lines(&Ok(vec![ip("10.0.1.5"), ip("10.0.1.1")]), &moment);
    assert!(
        lines[0].starts_with("warning: the default gateway 10.0.1.5 is an address of the trusted name coolify-proxy")
    );
    assert_eq!(lines[1], "default gateway 10.0.1.1: not a trusted proxy's address");
    let Some(StartLine::Warning(start)) = start_line(&Ok(vec![ip("10.0.1.5")]), &moment) else { panic!("warning") };
    assert!(
        start.starts_with("the default gateway 10.0.1.5 (an address of coolify-proxy) is a trusted proxy's address")
    );
    assert_eq!(start_line(&Ok(vec![ip("10.0.1.1")]), &moment), None);
    assert!(matches!(start_line(&Err(RouteError::Header), &moment), Some(StartLine::NotRead { .. })));
}

#[test]
fn every_change_of_a_name_is_logged_once() {
    let t0 = Instant::now();
    let p = name("coolify-proxy");
    let a = trusted(&["10.0.1.5"], t0 + STALE_AFTER);
    let line = |old: &NameState, new: &NameState| transition(&p, old, new);
    assert_eq!(
        line(&NameState::Pending, &a),
        Some((false, "the trusted name coolify-proxy resolves to 10.0.1.5".into()))
    );
    assert_eq!(line(&a, &trusted(&["10.0.1.5"], t0 + STALE_AFTER + REFRESH)), None, "a refresh with the same answer");
    let b = trusted(&["10.0.1.9"], t0 + STALE_AFTER);
    assert_eq!(
        line(&a, &b),
        Some((false, "the trusted name coolify-proxy now resolves to 10.0.1.9 (was 10.0.1.5)".into()))
    );
    let failing = next_state(&a, Err("e".into()), t0);
    assert!(matches!(line(&a, &failing), Some((true, l)) if l.contains("did not resolve (e)") && l.contains("10 s")));
    assert_eq!(line(&failing, &next_state(&failing, Err("e".into()), t0)), None, "warned once while it fails");
    assert!(matches!(line(&failing, &a), Some((false, l)) if l.ends_with("resolves again: 10.0.1.5")));
    let gone = NameState::Unresolved("e".into());
    assert!(matches!(line(&failing, &gone), Some((true, l)) if l.contains("are no longer trusted")));
    assert_eq!(line(&gone, &gone.clone()), None);
    assert!(matches!(line(&NameState::Pending, &gone), Some((true, l)) if l.contains("until it does")));
    let refused = NameState::Refused("8.8.8.8 is an address the Internet routes to".into());
    assert!(matches!(line(&a, &refused), Some((true, l)) if l.contains("none of its addresses is trusted")));
    assert_eq!(line(&refused, &refused.clone()), None);
}

struct Scripted(Mutex<mpsc::Receiver<Result<Vec<IpAddr>, String>>>);

impl Resolve for Scripted {
    fn resolve(&self, _: &HostName) -> Result<Vec<IpAddr>, String> {
        let next = self.0.lock().expect("one lookup at a time").recv();
        next.unwrap_or_else(|_| Err("the test ended".into()))
    }
}

#[test]
fn a_hanging_lookup_blocks_no_request_and_its_late_answer_is_dropped() {
    let (answers, script) = mpsc::channel();
    let mut lookups = Lookups::with_timeout(Arc::new(Scripted(Mutex::new(script))), Duration::from_millis(50));
    let runtime = tokio::runtime::Builder::new_current_thread().enable_time().build().expect("a runtime");
    let p = name("coolify-proxy");
    answers.send(Ok(vec![ip("10.0.1.5")])).expect("sent");
    assert_eq!(runtime.block_on(lookups.lookup(&p)), Ok(vec![ip("10.0.1.5")]));
    let started = Instant::now();
    assert_eq!(runtime.block_on(lookups.lookup(&p)), Err("no answer within 50 ms".into()));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        runtime.block_on(lookups.lookup(&p)),
        Err("an earlier lookup has not ended (no answer within 50 ms)".into())
    );
    answers.send(Ok(vec![ip("10.0.1.6")])).expect("sent");
    std::thread::sleep(Duration::from_millis(50));
    answers.send(Ok(vec![ip("10.0.1.7")])).expect("sent");
    assert_eq!(runtime.block_on(lookups.lookup(&p)), Ok(vec![ip("10.0.1.7")]));
    let trust = Trust::new(TrustSet::new(vec![], vec![p.clone()]));
    answers.send(Ok(vec![ip("10.0.1.8")])).expect("sent");
    runtime.block_on(refresh(&trust, &mut lookups));
    assert!(trust.now().trusts(ip("10.0.1.8")) && !trust.now().trusts(ip("10.0.1.7")));
}
