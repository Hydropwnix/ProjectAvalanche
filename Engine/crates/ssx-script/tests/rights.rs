use std::sync::Arc;
use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::limits::Budgets;
use ssx_script::host::loopback::Loopback;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::rights::{
    Decision, Holder, MAX_COMMANDS_PER_FRAME, OperationOrder, PRIVILEGED, Privileged, ResourceAction, Rights,
};
use ssx_script::host::server::{ServerEvent, ServerFrame};
use ssx_script::host::{CommandOrigin, HostConfig, ScriptHost, Side};

fn config(side: Side) -> HostConfig {
    let mut c = HostConfig::new(side);
    let b: &mut Budgets = &mut c.limits.budgets;
    (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(2));
    (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(4));
    (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(4), u64::MAX, Duration::from_secs(4));
    c
}

fn server(name: &str, src: &str) -> PackageScripts {
    let scripts = Scripts { client: vec![], server: vec!["server/*.luau".into()], shared: vec![] };
    let files = [("server/main.luau".to_string(), src.to_string())];
    let limits = config(Side::Server).limits;
    PackageScripts::from_memory(name, Side::Server, &scripts, &Files::default(), toml::Table::new(), &files, &limits)
        .unwrap()
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

struct Granted {
    packages: Vec<(&'static str, &'static str)>,
    players: Vec<(u32, &'static str)>,
}

impl Rights for Granted {
    fn check(&self, holder: Holder<'_>, right: &str) -> Result<Decision, String> {
        if !right.starts_with("function.") && !right.starts_with("command.") {
            return Err(format!("{right:?} is not a right"));
        }
        let default = right.strip_prefix("function.").is_some_and(|f| !PRIVILEGED.contains(&f));
        let allowed = match holder {
            Holder::Package(name) => default || self.packages.iter().any(|(p, r)| *p == name && *r == right),
            Holder::Player(c) => self.players.iter().any(|(p, r)| *p == c.0 && *r == right),
        };
        Ok(Decision { allowed, why: if allowed { "a group allows it".into() } else { "nothing grants it".into() } })
    }

    fn generation(&self) -> u64 {
        0
    }

    fn reason(&self, text: &str) -> Result<(), String> {
        if text.contains('<') { Err("the reason: no markup".into()) } else { Ok(()) }
    }
}

fn frame(now: u64) -> ServerFrame {
    ServerFrame { now: Duration::from_millis(33 * now), ..ServerFrame::default() }
}

#[test]
fn a_call_without_its_right_is_refused_with_the_reason() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    let src = "addCommandHandler('go', function()\n\
                 for _, p in ipairs(getPlayers()) do print(kickPlayer(p, 'bye')) end\n\
                 print(stopResource('other'))\n\
               end)";
    s.server.start(server("admin", src)).unwrap();
    let (a, _) = s.connect(ScriptHost::new(config(Side::Client)).unwrap(), "P1").unwrap();
    s.server.execute_command(CommandOrigin::Console, "go").unwrap();
    let lines = out(&s.server, "admin");
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].starts_with("kickPlayer: refused: admin does not hold function.kickPlayer: this host has no ACL"));
    assert!(lines[1].starts_with("false\tadmin does not hold function.kickPlayer"), "{lines:?}");
    assert!(lines[3].starts_with("false\tadmin does not hold function.stopResource"), "{lines:?}");
    s.server.set_rights(Arc::new(Granted { packages: vec![("admin", "function.banPlayer")], players: vec![] }));
    s.server.execute_command(CommandOrigin::Console, "go").unwrap();
    let lines = out(&s.server, "admin");
    assert_eq!(lines[1], "false\tadmin does not hold function.kickPlayer: nothing grants it");
    assert_eq!(s.server.server_frame(frame(1)).unwrap().operation, vec![], "nothing ordered");
    let counts = s.server.rights_counts();
    assert_eq!((counts.checks, counts.refused, counts.ordered), (4, 4, 0));
    s.release(a, "left");
}

#[test]
fn a_held_right_orders_the_call_once_per_player_and_tick() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    let granted =
        vec![("admin", "function.kickPlayer"), ("admin", "function.banPlayer"), ("admin", "function.mutePlayer")];
    s.server.set_rights(Arc::new(Granted { packages: granted, players: vec![] }));
    let src = "addCommandHandler('go', function()\n\
                 local p = getPlayers()[1]\n\
                 local k1, r1 = kickPlayer(p, 'read the rules')\n\
                 local k2, r2 = kickPlayer(p)\n\
                 local b1, q1 = banPlayer(p, nil, '<b>x</b>')\n\
                 print(k1, r1, k2, r2, b1, q1)\n\
                 local b2, q2 = banPlayer(p, 3600, 'griefing')\n\
                 local m1, n1 = mutePlayer(p)\n\
                 print(b2, q2, m1, n1)\n\
               end)";
    s.server.start(server("admin", src)).unwrap();
    let (a, _) = s.connect(ScriptHost::new(config(Side::Client)).unwrap(), "P1").unwrap();
    s.server.execute_command(CommandOrigin::Console, "go").unwrap();
    let lines: Vec<String> = out(&s.server, "admin").into_iter().filter(|l| !l.contains(": refused: ")).collect();
    assert_eq!(
        lines,
        [
            "true\tnil\tfalse\tthe player has that order already this tick\tfalse\tthe reason: no markup",
            "true\tnil\ttrue\tnil",
        ]
    );
    let orders = s.server.server_frame(frame(1)).unwrap().operation;
    let p = |right: &str, order| Privileged { package: "admin".into(), right: right.into(), order };
    assert_eq!(
        orders,
        vec![
            p("function.kickPlayer", OperationOrder::Kick { client: a, reason: "read the rules".into() }),
            p("function.banPlayer", OperationOrder::Ban { client: a, seconds: Some(3600), reason: "griefing".into() }),
            p("function.mutePlayer", OperationOrder::Mute { client: a, seconds: None, reason: String::new() }),
        ]
    );
    s.server.execute_command(CommandOrigin::Console, "go").unwrap();
    assert_eq!(s.server.server_frame(frame(2)).unwrap().operation.len(), 3, "a new tick takes new orders");
}

#[test]
fn commands_need_both_rights_and_are_bounded_per_tick() {
    let s = Loopback::new(config(Side::Server)).unwrap();
    let granted = vec![("ops", "function.executeCommand"), ("ops", "command.say"), ("ops", "function.restartResource")];
    s.server.set_rights(Arc::new(Granted { packages: granted, players: vec![] }));
    let src = "addCommandHandler('go', function()\n\
                 print(executeCommand('stop'))\n\
                 print(executeCommand('   '))\n\
                 print(restartResource('board'))\n\
                 local n = 0\n\
                 for i = 1, 20 do if executeCommand('say hello ' .. i) then n = n + 1 end end\n\
                 print(n)\n\
               end)";
    s.server.start(server("ops", src)).unwrap();
    s.server.execute_command(CommandOrigin::Console, "go").unwrap();
    let lines: Vec<String> = out(&s.server, "ops").into_iter().filter(|l| !l.contains(": refused: ")).collect();
    assert_eq!(lines[0], "false\tops does not hold command.stop: nothing grants it");
    assert_eq!(lines[1], "false\tan empty command line");
    assert_eq!(lines[2], "true\tnil");
    assert_eq!(lines[3], (MAX_COMMANDS_PER_FRAME - 1).to_string(), "the restart counts against the bound");
    let orders = s.server.server_frame(frame(1)).unwrap().operation;
    assert_eq!(orders.len(), MAX_COMMANDS_PER_FRAME);
    let restart = OperationOrder::Resource { action: ResourceAction::Restart, name: "board".into() };
    assert_eq!((orders[0].right.as_str(), &orders[0].order), ("function.restartResource", &restart));
    let say = OperationOrder::Command { line: "say hello 1".into() };
    assert_eq!((orders[1].right.as_str(), &orders[1].order), ("function.executeCommand", &say));
}

#[test]
fn permissions_are_asked_of_players_and_resources_and_shutdown_is_raised() {
    let mut s = Loopback::new(config(Side::Server)).unwrap();
    let src = "addEventHandler('onShutdown', root, function(secs) print('shutdown', secs) end)\n\
               addCommandHandler('ask', function()\n\
                 local p = getPlayers()[1]\n\
                 print(hasObjectPermissionTo(p, 'command.kick'), hasObjectPermissionTo(p, 'command.ban'),\n\
                       hasObjectPermissionTo(resourceRoot, 'function.kickPlayer'),\n\
                       pcall(hasObjectPermissionTo, root, 'command.kick'))\n\
               end)";
    s.server.start(server("panel", src)).unwrap();
    let (a, _) = s.connect(ScriptHost::new(config(Side::Client)).unwrap(), "P1").unwrap();
    s.server.execute_command(CommandOrigin::Console, "ask").unwrap();
    assert!(out(&s.server, "panel")[0].starts_with("false\tfalse\tfalse\tfalse\t"), "no ACL: nothing held");
    let rights = Granted { packages: vec![("panel", "function.kickPlayer")], players: vec![(a.0, "command.kick")] };
    s.server.set_rights(Arc::new(rights));
    s.server.execute_command(CommandOrigin::Console, "ask").unwrap();
    let line = &out(&s.server, "panel")[0];
    assert!(line.starts_with("true\tfalse\ttrue\tfalse\t"), "{line}");
    assert!(line.contains("neither a connected player nor a running resource"), "{line}");
    assert_eq!(s.server.rights_counts().checks, 0, "not a privileged call");
    let mut f = frame(1);
    f.events = vec![ServerEvent::Shutdown { round_end_secs: 60 }];
    s.server.server_frame(f).unwrap();
    assert_eq!(out(&s.server, "panel"), ["shutdown\t60"]);
}

#[test]
fn script_time_is_taken_once_per_package() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    assert!(s.take_spent().is_empty());
    let busy = "local function work() local x = 0 for i = 1, 200000 do x = x + i end end\n\
                addEventHandler('onResourceStop', root, work)\n\
                work()";
    s.start(server("busy", busy)).unwrap();
    s.start(server("idle", "x = 1")).unwrap();
    s.server_frame(frame(1)).unwrap();
    let spent: std::collections::BTreeMap<String, Duration> = s.take_spent().into_iter().collect();
    assert_eq!(spent.keys().collect::<Vec<_>>(), ["busy", "idle"]);
    assert!(spent["busy"] > spent["idle"], "{spent:?}");
    assert!(s.take_spent().is_empty(), "taken once");
    s.stop("busy", "done");
    let after = s.take_spent();
    assert_eq!(after.len(), 1, "only the stopped package's last time: {after:?}");
    assert!(after[0].0 == "busy" && after[0].1 > Duration::ZERO, "{after:?}");
}

struct Denied {
    denied: std::sync::Mutex<Vec<(&'static str, &'static str)>>,
    generation: std::sync::atomic::AtomicU64,
}

impl Denied {
    fn set(&self, denied: Vec<(&'static str, &'static str)>, moved: bool) {
        *self.denied.lock().unwrap() = denied;
        if moved {
            self.generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }
}

impl Rights for Denied {
    fn check(&self, holder: Holder<'_>, right: &str) -> Result<Decision, String> {
        let Holder::Package(name) = holder else { return Ok(Decision { allowed: false, why: "a player".into() }) };
        if self.denied.lock().unwrap().iter().any(|(p, r)| *p == name && *r == right) {
            return Ok(Decision { allowed: false, why: format!("group Muted denies {right}") });
        }
        let allowed = right.strip_prefix("function.").is_some_and(|f| !PRIVILEGED.contains(&f));
        Ok(Decision { allowed, why: if allowed { "the defaults".into() } else { "nothing grants it".into() } })
    }

    fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    fn reason(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn every_call_is_checked_against_the_package_set() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    let rights = Arc::new(Denied { denied: Default::default(), generation: Default::default() });
    s.set_rights(rights.clone());
    let src = "addCommandHandler('go', function()\n\
                 local ok, err = pcall(playSoundFor)\n\
                 print(ok or tostring(err):find('access denied', 1, true) and tostring(err) or 'other: ' .. tostring(err))\n\
                 print(hasObjectPermissionTo(resourceRoot, 'function.playSoundFor'), executeCommand('say hi'))\n\
               end)";
    s.start(server("board", src)).unwrap();
    s.start(server("other", &src.replace("'go'", "'go2'"))).unwrap();
    assert_eq!(s.rights_counts().resolved, 2, "each set resolved at its start");
    let go = |cmd: &str, name: &str| {
        s.execute_command(CommandOrigin::Console, cmd).unwrap();
        out(&s, name).into_iter().filter(|l| !l.contains(": refused: ")).collect::<Vec<_>>()
    };
    let allowed = go("go", "board");
    assert!(allowed[0].starts_with("other: "), "the defaults allow it: only its arguments refused: {allowed:?}");
    assert_eq!(allowed[1], "true\tfalse\tboard does not hold function.executeCommand: nothing grants it");
    rights.set(vec![("board", "function.playSoundFor")], true);
    let denied = go("go", "board");
    assert!(
        denied[0].starts_with(
            "runtime error: playSoundFor: access denied: board does not hold function.playSoundFor: group Muted \
             denies function.playSoundFor\n"
        ),
        "{denied:?}"
    );
    assert!(denied[1].starts_with("false\tfalse\t"), "{denied:?}");
    assert!(go("go2", "other")[0].starts_with("other: "), "another package's same call is not touched");
    let counts = s.rights_counts();
    assert_eq!((counts.calls_refused, counts.resolved), (1, 4), "refused once; both sets resolved once more");
    rights.set(vec![], false);
    s.tick(Duration::from_millis(33));
    assert!(go("go", "board")[0].starts_with("runtime error: playSoundFor: access denied"), "the set stands");
    assert_eq!(s.rights_counts().resolved, 4);
    rights.set(vec![], true);
    s.tick(Duration::from_millis(66));
    assert_eq!(s.rights_counts().resolved, 6, "resolved at the tick");
    assert!(go("go", "board")[0].starts_with("other: "), "allowed again");
    rights.set(vec![("board", "function.playSoundFor")], false);
    s.stop("board", "restart");
    s.start(server("board", src)).unwrap();
    assert_eq!(s.rights_counts().resolved, 7);
    assert!(go("go", "board")[0].starts_with("runtime error: playSoundFor: access denied"), "the restart's set");
    assert_eq!(s.rights_counts().calls_refused, 3);
}

#[test]
fn a_server_command_name_is_one_the_acl_can_name() {
    let s = ScriptHost::new(config(Side::Server)).unwrap();
    let src = "print(pcall(addCommandHandler, 'top-scores', function() end))\n\
               print(addCommandHandler('top_scores', function() end))";
    s.start(server("board", src)).unwrap();
    let lines = out(&s, "board");
    assert!(
        lines[0].starts_with("false\t") && lines[0].contains("a server command name is a-z, 0-9 and _"),
        "{lines:?}"
    );
    assert_eq!(lines[1], "true");
    let c = ScriptHost::new(config(Side::Client)).unwrap();
    let scripts = Scripts { client: vec!["client/*.luau".into()], server: vec![], shared: vec![] };
    let files =
        [("client/main.luau".to_string(), "print(addCommandHandler('top-scores', function() end))".to_string())];
    let limits = config(Side::Client).limits;
    let p = PackageScripts::from_memory(
        "board",
        Side::Client,
        &scripts,
        &Files::default(),
        toml::Table::new(),
        &files,
        &limits,
    )
    .unwrap();
    c.start(p).unwrap();
    assert_eq!(out(&c, "board"), ["true"]);
}
