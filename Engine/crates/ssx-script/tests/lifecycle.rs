use std::sync::Arc;
use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::limits::Budgets;
use ssx_script::host::loopback::Loopback;
use ssx_script::host::outbox::{Body, Peer};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::lifecycle::{CANCEL_START_RIGHT, LifecycleCounts};
use ssx_script::host::server::rights::{Decision, Holder, PRIVILEGED, Rights};
use ssx_script::host::{ClientId, HostConfig, ScriptHost, Side};

fn config(side: Side) -> HostConfig {
    let mut c = HostConfig::new(side);
    let b: &mut Budgets = &mut c.limits.budgets;
    (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(2));
    (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(4));
    (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(4), u64::MAX, Duration::from_secs(4));
    c
}

fn pkg(name: &str, side: Side, files: &[(&str, &str)]) -> PackageScripts {
    let scripts =
        Scripts { client: vec!["client/*.luau".into()], server: vec!["server/*.luau".into()], shared: vec![] };
    let files: Vec<(String, String)> = files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect();
    let limits = config(side).limits;
    PackageScripts::from_memory(name, side, &scripts, &Files::default(), toml::Table::new(), &files, &limits).unwrap()
}

fn server(name: &str, src: &str) -> PackageScripts {
    pkg(name, Side::Server, &[("server/main.luau", src)])
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

struct Granted(Vec<(&'static str, &'static str)>);

impl Rights for Granted {
    fn check(&self, holder: Holder<'_>, right: &str) -> Result<Decision, String> {
        let default = right.strip_prefix("function.").is_some_and(|f| !PRIVILEGED.contains(&f));
        let allowed = matches!(holder, Holder::Package(name)
            if default || self.0.iter().any(|(p, r)| *p == name && *r == right));
        Ok(Decision { allowed, why: if allowed { "granted".into() } else { "not granted".into() } })
    }

    fn generation(&self) -> u64 {
        0
    }

    fn reason(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_start_is_cancelled_only_by_a_package_with_the_right() {
    let s = Loopback::new(config(Side::Server)).unwrap();
    let guard = "addEvent('inner') addEventHandler('inner', root, function() inner = cancelEvent() end)\n\
                 addEventHandler('onResourcePreStart', root, function(name)\n\
                   triggerEvent('inner', root)\n\
                   local c = name == 'blocked' and tostring(cancelEvent()) or '-'\n\
                   print('pre', name, inner, c) end)";
    s.server.start(server("guard", guard)).unwrap();
    s.server.start(server("open", "x = 1")).unwrap();
    assert_eq!(out(&s.server, "guard"), ["pre\topen\ttrue\t-"]);
    s.server.start(server("blocked", "print('started')")).unwrap();
    let lines = out(&s.server, "guard");
    assert!(lines[0].contains(CANCEL_START_RIGHT), "the refusal noted: {lines:?}");
    assert_eq!(lines[1], "pre\tblocked\ttrue\tfalse", "refused without the right: {lines:?}");
    assert_eq!(out(&s.server, "blocked"), ["started"]);
    s.server.stop("blocked", "again");
    s.server.set_rights(Arc::new(Granted(vec![("guard", CANCEL_START_RIGHT), ("open", "function.startResource")])));
    let e = s.server.start(server("blocked", "print('started')")).unwrap_err();
    assert_eq!(e, "guard cancelled it (onResourcePreStart)");
    assert!(!s.server.running().contains(&"blocked".to_string()));
    assert_eq!(out(&s.server, "guard"), ["pre\tblocked\ttrue\ttrue"]);
    let counts = s.server.lifecycle_counts();
    assert_eq!((counts.starts_cancelled, counts.cancels_refused), (1, 1));
}

#[test]
fn every_state_change_is_raised() {
    let s = Loopback::new(config(Side::Server)).unwrap();
    let watch = "addEventHandler('onResourceStateChange', root, function(n, a, b) print(n, a, b) end)";
    s.server.start(server("watch", watch)).unwrap();
    assert_eq!(out(&s.server, "watch"), ["watch\tstarting\trunning"], "its own running, after its handler is there");
    s.server.start(server("a", watch)).unwrap();
    s.server.stop("a", "test");
    assert!(s.server.start(server("bad", "error('no')")).is_err());
    let want = [
        "a\tloaded\tstarting",
        "a\tstarting\trunning",
        "a\trunning\tstopping",
        "a\tstopping\tstopped",
        "bad\tloaded\tstarting",
        "bad\tstarting\tstopping",
        "bad\tstopping\tstopped",
    ];
    assert_eq!(out(&s.server, "watch"), want);
}

#[test]
fn acknowledgements_are_taken_once_per_client_and_start() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    let hud = |src: &str| pkg("hud", Side::Server, &[("server/main.luau", src), ("client/main.luau", "x = 1")]);
    let src =
        "addEventHandler('onPlayerResourceStart', root, function(name) print('ack', name, getElementType(source)) end)";
    s.server.start(hud(src)).unwrap();
    s.server.start(server("plain", "x = 1")).unwrap();
    let (a, _) = s.join(config(Side::Client)).unwrap();
    let client = s.client(a).unwrap();
    assert!(client.acknowledge_start("hud").is_err(), "not running on the client");
    client.start(pkg("hud", Side::Client, &[("client/main.luau", "x = 1")])).unwrap();
    client.acknowledge_start("hud").unwrap();
    s.step(Duration::from_millis(50));
    assert_eq!(out(&s.server, "hud"), ["ack\thud\tplayer"]);
    let started = |p: &str| s.server.deliver(Peer::Client(a), Some(p), Body::Started);
    assert!(started("hud").unwrap_err().contains("repeated"));
    assert!(started("nope").unwrap_err().contains("does not run"));
    assert!(started("plain").unwrap_err().contains("no client scripts"));
    assert!(s.server.deliver(Peer::Client(ClientId(99)), Some("hud"), Body::Started).is_err(), "no player");
    s.server.stop("hud", "restart");
    s.server.start(hud(src)).unwrap();
    s.server.deliver(Peer::Client(a), Some("hud"), Body::Started).unwrap();
    assert_eq!(out(&s.server, "hud"), ["ack\thud\tplayer"]);
    let want = LifecycleCounts { acknowledgements: 2, acknowledgements_refused: 4, ..Default::default() };
    assert_eq!(s.server.lifecycle_counts(), want);
}
