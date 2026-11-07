use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Overrun;
use ssx_script::host::monitor::{Monitor, StopReason};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{HostConfig, ScriptHost, Side};

const MARGIN: u64 = 64;

const SPIN: &str = "local function spin() local x = 0 for i = 1, 8000 do x += i end return x end\n";

fn one_at_a_time() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn host() -> ScriptHost {
    let h = ScriptHost::new(HostConfig::new(Side::Client)).unwrap();
    let b = h.limits().budgets;
    assert_eq!((b.call, b.chain, b.tick_checkpoints), (250_000, 500_000, 50_000), "the client's budgets");
    let backstops = (b.call_backstop, b.chain_backstop, b.tick_backstop);
    let ms = Duration::from_millis;
    assert_eq!(backstops, (ms(80), ms(160), ms(16)), "the client's backstops");
    h
}

fn start(h: &ScriptHost, name: &str, src: &str) {
    let scripts = Scripts { client: vec!["client/**/*.luau".into()], server: vec![], shared: vec![] };
    let contents = vec![("client/main.luau".to_string(), src.to_string())];
    let pkg = PackageScripts::from_memory(
        name,
        Side::Client,
        &scripts,
        &Files::default(),
        toml::Table::new(),
        &contents,
        h.limits(),
    )
    .unwrap();
    h.start(pkg).unwrap();
}

fn drive(h: &ScriptHost, ticks: u32) -> Monitor {
    start(h, "canary", "setTimer(function() print('canary') end, 50, 0)");
    let mut now = Duration::ZERO;
    for _ in 0..=ticks {
        now += Duration::from_millis(100);
        h.tick(now);
    }
    h.take_output("canary");
    for _ in 0..2 {
        now += Duration::from_millis(100);
        h.tick(now);
    }
    let canary = h.take_output("canary").map(|o| o.lines).unwrap_or_default();
    assert_eq!(canary, ["canary", "canary"], "the canary goes on after the cut\n{}", h.monitor().render());
    h.monitor()
}

fn running(h: &ScriptHost, name: &str) -> bool {
    h.running().iter().any(|n| n == name)
}

fn the_stop(m: &Monitor) -> (String, String) {
    assert_eq!(m.stops.len(), 1, "one package is stopped:\n{}", m.render());
    (m.stops[0].package.clone(), m.stops[0].reason.to_string())
}

fn cut_once_within_the_bound(m: &Monitor, what: &str) {
    let (bound, backstop) = (m.budgets.chain, m.budgets.chain_backstop);
    let n = m.chain_peak_checkpoints;
    eprintln!(
        "{what}: the longest outermost call {n} checkpoints (bound {bound}), {:.3} ms of CPU time, {:.3} ms of wall time",
        m.chain_peak.as_secs_f64() * 1e3,
        m.chain_peak_wall.as_secs_f64() * 1e3
    );
    assert_eq!(m.chain_cuts, 1, "{what}: the chain was cut once\n{}", m.render());
    let cut = m.stops.iter().find_map(|s| match s.reason {
        StopReason::ChainBudget { bound, checkpoints, cpu, .. } => Some((bound, checkpoints, cpu)),
        _ => None,
    });
    match cut {
        Some((Overrun::Checkpoints(b), at, _)) => {
            assert!(b == bound && at == bound + 1, "{what}: cut at {at} checkpoints");
            assert!(n <= bound + MARGIN, "{what}: the host's call passed {n} checkpoints");
        }
        Some((Overrun::Backstop(b), at, cpu)) => {
            eprintln!("{what}: the backstop came first ({at} checkpoints at {cpu:?})");
            assert!(b == backstop && cpu >= backstop && at <= bound, "{what}: cut at {at} checkpoints, {cpu:?}");
        }
        None => panic!("{what}: no chain stop\n{}", m.render()),
    }
}

#[test]
fn a_caller_that_loops_on_a_callees_export_is_stopped_at_the_chain_bound() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "b", &format!("{SPIN}exports('spin', function() spin() return 1 end)"));
    start(&h, "a", "setTimer(function() local n = 0 while true do n += exports.b:spin() end end, 50, 1)");
    let m = drive(&h, 1);
    cut_once_within_the_bound(&m, "a loops on exports.b:spin");
    let (who, why) = the_stop(&m);
    assert_eq!(who, "a", "the caller that issued the calls, not the callee each call of which was short");
    assert!(why.contains("past its chain bound") && why.contains("timer"), "{why}");
    assert!(running(&h, "b"), "the callee goes on");
}

#[test]
fn a_caller_that_catches_every_failed_call_is_stopped_all_the_same() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "b", &format!("{SPIN}exports('spin', function() spin() return 1 end)"));
    start(&h, "a", "setTimer(function() while true do pcall(function() return exports.b:spin() end) end end, 50, 1)");
    let m = drive(&h, 1);
    cut_once_within_the_bound(&m, "a loops on pcall(exports.b:spin)");
    assert_eq!(the_stop(&m).0, "a");
    assert!(running(&h, "b"));
}

#[test]
fn a_package_that_loops_on_an_event_another_package_handles_is_stopped_at_the_chain_bound() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "b", &format!("{SPIN}addEvent('ping') addEventHandler('ping', root, function() spin() end)"));
    start(&h, "a", "setTimer(function() while true do triggerEvent('ping', root) end end, 50, 1)");
    let m = drive(&h, 1);
    cut_once_within_the_bound(&m, "a loops on triggerEvent with b's handler");
    let (who, why) = the_stop(&m);
    assert_eq!(who, "a", "{why}");
    assert!(running(&h, "b"), "the handler's package goes on");
}

fn links(h: &ScriptHost, looping: Option<usize>) {
    for i in 2..9 {
        let src = if looping == Some(i) {
            format!("exports('f', function() while true do exports.p{}:f() end end)", i + 1)
        } else {
            format!("exports('f', function() return exports.p{}:f() end)", i + 1)
        };
        start(h, &format!("p{i}"), &src);
    }
    start(
        h,
        "p9",
        &format!(
            "{SPIN}local told = false\n\
             exports('f', function() if not told then told = true print(pcall(function() return exports.p2:f() end)) end spin() return 1 end)"
        ),
    );
}

#[test]
fn a_chain_at_the_export_depth_cap_with_every_link_under_budget_is_cut_and_its_looping_head_stopped() {
    let _one = one_at_a_time();
    let h = host();
    links(&h, None);
    start(&h, "p1", "setTimer(function() while true do exports.p2:f() end end, 50, 1)");
    let m = drive(&h, 1);
    let told = h.take_output("p9").map(|o| o.lines).unwrap_or_default();
    assert!(told[0].contains("nested deeper than 8"), "the chain is at the depth cap: {told:?}");
    cut_once_within_the_bound(&m, "p1 loops on a chain of 8 exports");
    assert_eq!(the_stop(&m).0, "p1", "the head that issued the calls");
    assert!((2..10).all(|i| running(&h, &format!("p{i}"))), "every link goes on");
}

#[test]
fn a_link_in_the_middle_that_loops_is_stopped_and_the_links_around_it_go_on() {
    let _one = one_at_a_time();
    let h = host();
    links(&h, Some(5));
    start(
        &h,
        "p1",
        "setTimer(function() print(pcall(function() return exports.p2:f() end)) print('p1 goes on') end, 100, 2)",
    );
    let m = drive(&h, 2);
    cut_once_within_the_bound(&m, "p5 loops in the middle of the chain");
    let (who, why) = the_stop(&m);
    assert_eq!(who, "p5", "{why}");
    assert!(why.contains("exports.p5:f"), "{why}");
    assert!((1..10).filter(|i| *i != 5).all(|i| running(&h, &format!("p{i}"))), "the links around it go on");
    let p1 = h.take_output("p1").map(|o| o.lines).unwrap_or_default();
    assert!(p1[0].contains("call chain ran past its bound and was cut"), "the head's call unwound: {p1:?}");
    assert!(p1[1].starts_with("false\t") && p1[1].contains("p5 is not running"), "{p1:?}");
    assert_eq!(p1[2], "p1 goes on", "the head is not stopped");
}

#[test]
fn a_callees_own_runaway_is_still_stopped_by_its_own_budget_before_the_chain_bound() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "b", "exports('spin', function() while true do end end)");
    start(
        &h,
        "a",
        "setTimer(function() print(pcall(function() return exports.b:spin() end)) print('a goes on') end, 50, 1)",
    );
    let m = drive(&h, 1);
    assert_eq!(m.chain_cuts, 0, "the callee's own budget came first\n{}", m.render());
    let (who, why) = the_stop(&m);
    assert_eq!(who, "b");
    assert!(why.contains("ran past its call budget") && why.contains("in exports.b:spin"), "{why}");
    let a = h.take_output("a").map(|o| o.lines).unwrap_or_default();
    assert!(a[0].contains("b is stopped") && a[1] == "a goes on", "{a:?}");
    assert!(m.chain_peak_checkpoints < m.budgets.chain, "{}", m.chain_peak_checkpoints);
}

#[test]
fn a_caller_that_keeps_a_callee_busy_every_tick_under_the_chain_bound_is_the_one_the_tick_budget_stops() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "b", &format!("{SPIN}exports('spin', function() spin() return 1 end)"));
    start(&h, "a", "setTimer(function() for i = 1, 10 do exports.b:spin() end end, 50, 0)");
    let m = drive(&h, 34);
    assert_eq!(m.chain_cuts, 0, "no call reached the chain bound\n{}", m.render());
    let (who, why) = the_stop(&m);
    assert_eq!(who, "a", "the caller pays for the calls it made: {why}");
    assert!(
        why.contains("its calls used more than the tick budget of 50000 checkpoints in 30 ticks in a row"),
        "{why}"
    );
    assert!(running(&h, "b"), "the callee, each call of which was short, goes on");
}

#[test]
fn a_callee_stopped_for_its_own_runaway_is_not_counted_against_its_caller() {
    let _one = one_at_a_time();
    let h = host();
    start(&h, "z", "exports('run', function() while true do end end)");
    start(&h, "v", &format!("{SPIN}exports('spin', function() spin() return 1 end)"));
    start(&h, "w", "exports('f', function() while true do exports.v:spin() end end)");
    start(&h, "y", "exports('f', function() pcall(function() return exports.z:run() end) return exports.w:f() end)");
    start(&h, "x", "setTimer(function() pcall(function() return exports.y:f() end) end, 50, 1)");
    let m = drive(&h, 1);
    cut_once_within_the_bound(&m, "y called a runaway z, then w looped on v");
    let stops: Vec<(String, String)> = m.stops.iter().map(|s| (s.package.clone(), s.reason.to_string())).collect();
    assert_eq!(stops.len(), 2, "{}", m.render());
    assert!(stops[0].0 == "z" && stops[0].1.contains("ran past its call budget"), "{stops:?}");
    assert!(stops[1].0 == "w" && stops[1].1.contains("past its chain bound"), "{stops:?}");
    assert!(["x", "y", "v"].iter().all(|p| running(&h, p)), "{}", m.render());
}

#[test]
fn a_large_scripts_compile_at_its_start_is_outside_the_chain() {
    let _one = one_at_a_time();
    let mut c = HostConfig::new(Side::Client);
    (c.limits.budgets.call_backstop, c.limits.budgets.chain_backstop) =
        (Duration::from_millis(10), Duration::from_millis(20));
    let h = ScriptHost::new(c).unwrap();
    let src = format!("local function never()\n{}end\na = 1\n", "a = 1\n".repeat(150_000));
    let began = std::time::Instant::now();
    start(&h, "large", &src);
    let took = began.elapsed();
    let m = h.monitor();
    eprintln!(
        "the start took {:.1} ms, its longest host call {} checkpoints, {:.3} ms of CPU time",
        took.as_secs_f64() * 1e3,
        m.chain_peak_checkpoints,
        m.chain_peak.as_secs_f64() * 1e3
    );
    let backstop = m.budgets.chain_backstop;
    assert!(took > backstop, "the premise: the start took longer than the chain's backstop ({took:?})");
    assert!(m.chain_cuts == 0 && m.stops.is_empty() && running(&h, "large"), "{}", m.render());
    assert!(m.chain_peak < m.budgets.call_backstop, "the top level's call ran {:?}", m.chain_peak);
}
