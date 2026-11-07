use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::limits::Budgets;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{CommandOrigin, HostConfig, ScriptHost, Side};

fn relaxed(b: &mut Budgets, s: u64) {
    (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(s));
    (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(2 * s));
    (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(2 * s), u64::MAX, Duration::from_secs(2 * s));
}

fn files_package(
    name: &str,
    side: Side,
    files: &[(&str, &str)],
    settings: toml::Table,
) -> Result<PackageScripts, String> {
    let scripts = Scripts {
        client: vec!["client/**/*.luau".into()],
        server: vec!["server/**/*.luau".into()],
        shared: vec!["shared/**/*.luau".into()],
    };
    let files_cfg = Files { include: vec!["lib/**".into()], server_only: vec!["secret/**".into()], ..Files::default() };
    let contents: Vec<(String, String)> = files.iter().map(|(p, s)| (p.to_string(), s.to_string())).collect();
    PackageScripts::from_memory(name, side, &scripts, &files_cfg, settings, &contents, &HostConfig::new(side).limits)
}

fn host(side: Side) -> ScriptHost {
    let mut c = HostConfig::new(side);
    relaxed(&mut c.limits.budgets, 2);
    ScriptHost::new(c).unwrap()
}

fn start(h: &ScriptHost, name: &str, src: &str) -> Result<(), String> {
    let path = if h.side() == Side::Server { "server/main.luau" } else { "client/main.luau" };
    h.start(files_package(name, h.side(), &[(path, src)], toml::Table::new())?)
}

#[test]
fn budgets_that_cannot_hold_are_refused() {
    let ms = Duration::from_millis;
    type Change = fn(&mut Budgets);
    let bad: [(&str, Change); 9] = [
        ("chain = call", |b| b.chain = b.call),
        ("chain backstop = call backstop", |b| b.chain_backstop = b.call_backstop),
        ("call 0", |b| b.call = 0),
        ("call backstop 0", |b| b.call_backstop = Duration::ZERO),
        ("call beyond the most", |b| (b.call, b.chain) = (Limits::MAX_CHECKPOINTS + 1, u64::MAX)),
        ("tick 0", |b| b.tick = Duration::ZERO),
        ("tick checkpoints 0", |b| b.tick_checkpoints = 0),
        ("tick backstop 0", |b| b.tick_backstop = Duration::ZERO),
        ("over_tick 0", |b| b.over_tick = 0),
    ];
    for (what, f) in bad {
        let mut c = HostConfig::new(Side::Client);
        f(&mut c.limits.budgets);
        assert!(ScriptHost::new(c).is_err(), "{what}");
    }
    let mut c = HostConfig::new(Side::Client);
    (c.limits.budgets.call_backstop, c.limits.budgets.chain_backstop) = (ms(1), ms(2));
    assert!(ScriptHost::new(c).is_ok(), "the client's checkpoint budgets with backstops of 1 and 2 ms hold");
}

fn out(h: &ScriptHost, name: &str) -> Vec<String> {
    h.take_output(name).map(|o| o.lines).unwrap_or_default()
}

#[test]
fn scripts_load_shared_first_then_the_sides_in_path_order() {
    let h = host(Side::Server);
    let files = [
        ("server/b.luau", "print('server b')"),
        ("server/a.luau", "print('server a')"),
        ("shared/z.luau", "print('shared z')"),
        ("client/c.luau", "print('client')"),
        ("lib/m.luau", "return 1"),
    ];
    h.start(files_package("order", Side::Server, &files, toml::Table::new()).unwrap()).unwrap();
    assert_eq!(out(&h, "order"), ["shared z", "server a", "server b"]);
    let client = files_package("order", Side::Client, &files, toml::Table::new()).unwrap();
    assert_eq!(client.scripts.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["shared/z.luau", "client/c.luau"]);
    assert!(!client.modules.contains_key("server/a.luau"), "a client never receives server scripts");
    let hidden = [("client/c.luau", "return require('secret/s')"), ("secret/s.luau", "return 1")];
    let c = files_package("hidden", Side::Client, &hidden, toml::Table::new()).unwrap();
    assert!(!c.modules.contains_key("secret/s.luau"), "server_only files stay on the server");
    for bad in ["con", "nul.x", "Upper", "../up", "a/b", ""] {
        assert!(
            files_package(bad, Side::Server, &files, toml::Table::new()).is_err(),
            "{bad:?} names a storage folder"
        );
    }
    let twice = [("server/a.luau", ""), ("server/A.luau", "")];
    assert!(files_package("case", Side::Server, &twice, toml::Table::new()).unwrap_err().contains("two spellings"));
}

#[test]
fn a_failed_start_rolls_everything_back_and_a_restart_is_clean() {
    let h = host(Side::Server);
    let bad = [
        (
            "server/a.luau",
            "createElement('thing') addEvent('x') addEventHandler('x', root, print) setTimer(print, 100, 0)",
        ),
        ("server/b.luau", "error('second script fails')"),
    ];
    let e = h.start(files_package("roll", Side::Server, &bad, toml::Table::new()).unwrap()).unwrap_err();
    assert!(e.contains("second script fails"), "{e}");
    let (held, vm) = h.holdings("roll");
    assert_eq!((held, vm), (Default::default(), false), "nothing left after the rollback");
    assert_eq!(h.elements(), 1, "only the root");
    start(&h, "roll", "print('clean start')").unwrap();
    assert_eq!(out(&h, "roll"), ["clean start"]);
    assert!(start(&h, "roll", "").unwrap_err().contains("already running"));
    let mut future = files_package("future", Side::Server, &[("server/a.luau", "")], toml::Table::new()).unwrap();
    future.api = 2;
    assert!(h.start(future).unwrap_err().contains("API level 2"));
    let cancel = "addEventHandler('onResourceStart', resourceRoot, function() cancelEvent() end)";
    assert!(start(&h, "cancelled", cancel).unwrap_err().contains("cancelled"));
    assert!(h.running().contains(&"roll".to_string()) && !h.running().contains(&"cancelled".to_string()));
}

#[test]
fn elements_belong_to_their_package_and_go_with_it() {
    let h = host(Side::Server);
    start(&h, "a", "box = createElement('box', 'b1') inner = createElement('box') setElementParent(inner, box)")
        .unwrap();
    start(
        &h,
        "b",
        "local box = getElementByID('b1')\n\
         print(getElementType(box), getElementID(box), getElementParent(box) == getResourceRootElement() and 'own' or 'foreign')\n\
         print(pcall(destroyElement, box))\n\
         print(pcall(setElementParent, createElement('x'), box))\n\
         print(pcall(createElement, 'player'))\n\
         print(#getElementsByType('box'), #getElementChildren(box))",
    )
    .unwrap();
    let lines = out(&h, "b");
    assert_eq!(lines[0], "box\tb1\tforeign");
    assert!(lines[1].starts_with("false\t") && lines[1].contains("not this package's"), "{}", lines[1]);
    assert!(lines[2].contains("neither this package's root"), "{}", lines[2]);
    assert!(lines[3].contains("the host or the game makes"), "{}", lines[3]);
    assert_eq!(lines[4], "2\t1");
    h.stop("a", "test");
    let after = h.elements();
    start(&h, "c", "print(getElementByID('b1') == nil, #getElementsByType('box'))").unwrap();
    assert_eq!(out(&h, "c"), ["true\t0"], "a's elements went with it (tree had {after})");
}

#[test]
fn element_handles_compare_and_key_tables_and_die_with_their_element() {
    let h = host(Side::Server);
    start(
        &h,
        "a",
        "local e = createElement('x') local t = {[e] = 1}\n\
         print(getElementsByType('x')[1] == e, t[getElementsByType('x')[1]], tostring(e):match('^element: #') ~= nil)\n\
         destroyElement(e) print(isElement(e), pcall(getElementType, e))\n\
         print(pcall(function() return getmetatable(e) end))",
    )
    .unwrap();
    let lines = out(&h, "a");
    assert_eq!(lines[0], "true\t1\ttrue");
    assert!(lines[1].starts_with("false\tfalse\t") && lines[1].contains("does not exist"), "{}", lines[1]);
    assert_eq!(lines[2], "true\tfalse", "the userdata's metatable is locked");
}

#[test]
fn events_reach_handlers_on_the_source_its_ancestors_and_below_and_cancel() {
    let h = host(Side::Server);
    start(&h, "decl", "addEvent('ping')").unwrap();
    start(
        &h,
        "a",
        "local parent = createElement('p') local child = createElement('c') setElementParent(child, parent)\n\
         addEventHandler('ping', root, function(n) print('root', n, source == child, this == root, eventName) end)\n\
         addEventHandler('ping', parent, function() print('parent') end, false)\n\
         addEventHandler('ping', child, function(n) print('child', n) cancelEvent() end)\n\
         print(triggerEvent('ping', child, 5), wasEventCancelled())\n\
         print(triggerEvent('ping', parent, 6))\n\
         print(pcall(triggerEvent, 'onResourceStart', root))\n\
         print(pcall(triggerEvent, 'undeclared', root))\n\
         print(pcall(triggerEvent, 'ping', root, print))",
    )
    .unwrap();
    let lines = out(&h, "a");
    assert_eq!(
        lines[..3],
        ["root\t5\ttrue\ttrue\tping", "child\t5", "false\ttrue"],
        "parent's handler does not propagate"
    );
    assert_eq!(lines[3..7], ["root\t6\tfalse\ttrue\tping", "parent", "child\t6", "false"], "below the source too");
    assert!(lines[7].contains("host's own event"), "{}", lines[7]);
    assert!(lines[8].contains("not declared"), "{}", lines[8]);
    assert!(lines[9].contains("cannot be copied"), "{}", lines[9]);
}

#[test]
fn events_nest_at_most_sixteen_deep() {
    let h = host(Side::Server);
    start(
        &h,
        "a",
        "addEvent('again') local depth = 0\n\
         addEventHandler('again', root, function() depth += 1 local ok, e = pcall(triggerEvent, 'again', root) if not ok then print(depth, e) end end)\n\
         triggerEvent('again', root)",
    )
    .unwrap();
    let lines = out(&h, "a");
    assert!(lines[0].starts_with("16\t") && lines[0].contains("nested deeper than 16"), "{lines:?}");
}

#[test]
fn exports_copy_values_across_packages_and_bound_their_nesting() {
    let mut c = HostConfig::new(Side::Server);
    (c.limits.budgets.call, c.limits.budgets.chain) = (1_000_000, 2_000_000);
    let h = ScriptHost::new(c).unwrap();
    start(
        &h,
        "lib",
        "exports('fill', function(t) t.x = 1 return t, #t end)\n\
         exports('bounce', function(n) return exports.user:bounce(n + 1) end)\n\
         exports('spin', function() while true do end end)",
    )
    .unwrap();
    start(
        &h,
        "user",
        "local t = {1, 2} local r, n = exports.lib:fill(t) print(t.x, r.x, n, r == t)\n\
         print(pcall(exports.lib.fill, t))\n\
         print(pcall(function() return exports.lib:fill(print) end))\n\
         print(pcall(function() return exports.lib:nothing() end))\n\
         exports('bounce', function(n) return exports.lib:bounce(n + 1) end)\n\
         print(pcall(function() return exports.lib:bounce(0) end))\n\
         print(pcall(function() return exports.lib:spin() end))\n\
         print('user goes on')",
    )
    .unwrap();
    let lines = out(&h, "user");
    assert_eq!(lines[0], "nil\t1\t2\tfalse", "the callee changed its copy");
    assert!(lines[1].contains("call it with ':'"), "{}", lines[1]);
    assert!(lines[2].contains("cannot be copied") || lines[2].contains("arguments"), "{}", lines[2]);
    assert!(lines[3].contains("exports no nothing"), "{}", lines[3]);
    assert!(lines[4].contains("nested deeper than 8"), "{}", lines[4]);
    assert!(lines[5].contains("lib is stopped"), "{}", lines[5]);
    assert_eq!(lines[6], "user goes on", "the caller is not charged for the callee's time");
    assert!(!h.running().contains(&"lib".to_string()), "the callee broke its budget and was stopped");
    let stop = h.monitor().stops.into_iter().find(|s| s.package == "lib").unwrap();
    assert!(stop.reason.to_string().contains("exports.lib:spin"), "{}", stop.reason);
}

#[test]
fn timers_fire_on_the_host_clock_without_catching_up() {
    let h = host(Side::Client);
    start(
        &h,
        "t",
        "n = 0 local t t = setTimer(function(a, b) n += 1 print('tick', n, a, b) if n == 3 then killTimer(t) end end, 100, 0, 'x', 7)\n\
         setTimer(function() print('once') end, 250, 1)\n\
         print(pcall(setTimer, print, 10, 1))\n\
         print(pcall(setTimer, print, 100, -1))",
    )
    .unwrap();
    let first = out(&h, "t");
    assert!(first[0].contains("from 50 to"), "{}", first[0]);
    assert!(first[1].contains("bad argument #3"), "{}", first[1]);
    for ms in [100u64, 200, 300, 1300, 1400] {
        h.tick(Duration::from_millis(ms));
    }
    assert_eq!(
        out(&h, "t"),
        ["tick\t1\tx\t7", "tick\t2\tx\t7", "once", "tick\t3\tx\t7"],
        "one firing per tick, then killed"
    );
    assert_eq!(h.holdings("t").0.timers, 0);
}

#[test]
fn commands_reach_every_handler_with_the_player_on_the_server() {
    let h = host(Side::Server);
    start(&h, "a", "addCommandHandler('say', function(p, cmd, ...) print(p == nil, cmd, ...) end)").unwrap();
    start(
        &h,
        "b",
        "addCommandHandler('say', function(p, cmd, a) print('b', a) end) print(pcall(addCommandHandler, 'Say', print))",
    )
    .unwrap();
    assert_eq!(h.execute_command(CommandOrigin::Console, "say hello  world").unwrap(), 2);
    assert_eq!(out(&h, "a"), ["true\tsay\thello\tworld"]);
    let b = out(&h, "b");
    assert!(b[0].contains("lower case") && b[1] == "b\thello", "{b:?}");
    assert!(h.execute_command(CommandOrigin::Console, "nothing").is_err());
    assert!(
        h.execute_command(CommandOrigin::Console, &format!("say {}", "x ".repeat(40)))
            .unwrap_err()
            .contains("arguments")
    );
    let c = host(Side::Client);
    start(&c, "a", "addCommandHandler('hud', function(cmd, x) print(cmd, x) end)").unwrap();
    assert_eq!(c.execute_command(CommandOrigin::Console, "hud on").unwrap(), 1);
    assert_eq!(out(&c, "a"), ["hud\ton"]);
    let other = ssx_script::host::ClientId(3);
    assert!(c.execute_command(CommandOrigin::Client(other), "hud on").unwrap_err().contains("own console"));
}

#[test]
fn settings_come_from_the_manifest_and_the_operator() {
    let mut cfg = HostConfig::new(Side::Server);
    let mut over = toml::Table::new();
    over.insert("countdown_s".into(), toml::Value::Integer(9));
    cfg.settings.insert("s".into(), over);
    let h = ScriptHost::new(cfg).unwrap();
    let settings: toml::Table =
        toml::from_str("countdown_s = 5\nname = 'x'\nlist = [1, 2]\n[nested]\nk = true").unwrap();
    let files =
        [("server/a.luau", "print(get('countdown_s'), get('name'), get('list')[2], get('nested').k, get('missing'))")];
    h.start(files_package("s", Side::Server, &files, settings).unwrap()).unwrap();
    assert_eq!(out(&h, "s"), ["9\tx\t2\ttrue\tnil"]);
    let big: toml::Table = toml::from_str("n = 9007199254740993").unwrap();
    let e = h.start(files_package("big", Side::Server, &[("server/a.luau", "")], big).unwrap()).unwrap_err();
    assert!(e.contains("2^53"), "{e}");
}

#[test]
fn json_and_print_and_the_monitor() {
    let h = host(Side::Server);
    start(
        &h,
        "j",
        "local t = json.decode('{\"a\": [1, 2, {\"b\": null}], \"c\": \"d\"}') print(t.a[2], t.c, json.encode({1, 2}))\n\
         print(pcall(json.encode, {[{}] = 1}))\n\
         print(pcall(json.decode, '[' .. string.rep('[', 20) .. ']'))",
    )
    .unwrap();
    let lines = out(&h, "j");
    assert_eq!(lines[0], "2\td\t[1,2]");
    assert!(lines[1].contains("cannot be copied"), "{}", lines[1]);
    assert!(lines[2].contains("deeper"), "{}", lines[2]);
    h.tick(Duration::from_millis(16));
    let m = h.monitor();
    let p = m.packages.iter().find(|p| p.name == "j").unwrap();
    assert!(p.heap > 0 && p.heap_peak >= p.heap);
    assert!(m.render().contains("j"));
}

#[test]
fn the_monitor_shows_the_budgets_in_checkpoints() {
    let h = ScriptHost::new(HostConfig::new(Side::Client)).unwrap();
    start(&h, "spin", "setTimer(function() local x = 0 for i = 1, 1000 do x += i end end, 50, 0)").unwrap();
    for ms in [50, 100, 150] {
        h.tick(Duration::from_millis(ms));
    }
    let m = h.monitor();
    let text = m.render();
    let budgets = "[client budgets] call 250000 ckpt (backstop 80.000 ms CPU) | tick 50000 ckpt (backstop 16.000 ms \
                   CPU) x30 in a row | chain 500000 ckpt (backstop 160.000 ms CPU) | queued work waits past 2.000 ms CPU \
                   a tick\n";
    assert!(text.starts_with(budgets), "{text}");
    let p = m.packages.iter().find(|p| p.name == "spin").unwrap();
    assert!((1000..1010).contains(&p.call_peak), "{}", p.call_peak);
    let line = text.lines().find(|l| l.trim_start().starts_with("spin ")).unwrap();
    assert!(line.contains(&format!("call peak {:>7}", p.call_peak)), "{line}");
}

#[test]
fn a_stopped_package_leaves_nothing_and_others_hear_of_it() {
    let h = host(Side::Server);
    start(
        &h,
        "watch",
        "addEventHandler('onResourceStop', root, function() print('stopped', getElementID(source)) end)",
    )
    .unwrap();
    start(
        &h,
        "busy",
        "addEvent('e', true) addEventHandler('e', root, print) local el = createElement('x')\n\
         setElementData(el, 'k', {1, 2}) setElementData(root, 'mine', 'v') setTimer(print, 100, 0)\n\
         addCommandHandler('busy', print) exports('f', print)",
    )
    .unwrap();
    let (held, _) = h.holdings("busy");
    assert!(held.elements == 1 && held.data_keys == 2 && held.handlers == 1 && held.timers == 1 && held.exports == 1);
    h.stop("busy", "the test");
    assert_eq!(out(&h, "watch"), ["stopped\tbusy"]);
    assert_eq!(h.holdings("busy"), (Default::default(), false));
    start(&h, "check", "print(getElementData(root, 'mine'))").unwrap();
    assert_eq!(out(&h, "check"), ["nil"], "its data on the root went too");
}

#[test]
fn a_value_set_in_element_data_is_a_copy() {
    let h = host(Side::Server);
    start(
        &h,
        "d",
        "local t = {1} setElementData(root, 'k', t) t[1] = 2 print(getElementData(root, 'k')[1])\n\
         print(pcall(setElementData, root, 'k', string.rep('x', 70000)))\n\
         print(pcall(setElementData, root, 'bad key!', 1))\n\
         print(setElementData(root, 'k', nil), hasElementData(root, 'k'))",
    )
    .unwrap();
    let lines = out(&h, "d");
    assert_eq!(lines[0], "1");
    assert!(lines[1].contains("65536"), "{}", lines[1]);
    assert!(lines[2].contains("bad argument #2"), "{}", lines[2]);
    assert_eq!(lines[3], "true\tfalse");
}

#[test]
fn nested_calls_fit_a_one_mib_stack() {
    const DEEP: &str = "local function deep(then_) local t = setmetatable({}, {__index = function(s, k) \
                        if k > 0 then return s[k - 1] end return then_() end}) return t[180] end";
    for n in 1..=6u32 {
        let run = move || {
            let mut c = HostConfig::new(Side::Server);
            relaxed(&mut c.limits.budgets, 9);
            let h = ScriptHost::new(c).unwrap();
            for k in (1..=n + 1).rev() {
                let src = if k <= n {
                    format!(
                        "{DEEP} exports('f', function() return deep(function() return exports['p{}']:f() end) end)",
                        k + 1
                    )
                } else {
                    "local function g(s) return (string.gsub(s, '.', g)) end exports('f', function() return pcall(g, 'ab') end)".into()
                };
                start(&h, &format!("p{k}"), &src).unwrap();
            }
            start(&h, "top", "print(pcall(function() return exports.p1:f() end))").unwrap();
            out(&h, "top")
        };
        let lines = std::thread::Builder::new().stack_size(1 << 20).spawn(run).unwrap().join().unwrap();
        assert!(
            lines[0].starts_with("true") || lines[0].contains("nested deeper than the host's stack allows"),
            "{n}: {lines:?}"
        );
    }
}

#[test]
fn the_monitors_counters_match_a_walk_of_the_host() {
    let h = host(Side::Server);
    start(&h, "other", "exports('n', function() return 1 end)").unwrap();
    start(
        &h,
        "p",
        "local a = createElement('x') local b = createElement('x') setElementParent(b, a) local c = createElement('x')\n\
         setElementData(a, 'k', 1) setElementData(b, 'k', string.rep('y', 100)) setElementData(c, 'k', 2) setElementData(root, 'r', {1, 2})\n\
         setElementData(a, 'k', 'longer value') setElementData(c, 'k', nil) destroyElement(a)\n\
         local t = setTimer(print, 100, 0) killTimer(t) setTimer(print, 100, 1) setTimer(print, 100, 0)",
    )
    .unwrap();
    for ms in [100u64, 200, 300] {
        h.tick(Duration::from_millis(ms));
    }
    let walked = h.holdings("p").0;
    let kept = h.monitor().packages.into_iter().find(|p| p.name == "p").unwrap().counts;
    assert_eq!(walked, kept);
    assert_eq!((kept.elements, kept.data_keys, kept.timers), (1, 1, 1), "c, root's r, the endless timer");
}

#[test]
fn storage_survives_a_restart() {
    let root = std::env::temp_dir().join(format!("ssx-script-host-storage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut c = HostConfig::new(Side::Server);
    c.storage_root = Some(root.clone());
    let h = ScriptHost::new(c).unwrap();
    start(&h, "keep", "storage.set('best', {lap = 61.5, who = 'elise'}) print(storage.get('best').who)").unwrap();
    assert_eq!(out(&h, "keep"), ["elise"]);
    h.stop("keep", "restart");
    start(&h, "keep", "local b = storage.get('best') print(b.lap, b.who, storage.get('none'))").unwrap();
    assert_eq!(out(&h, "keep"), ["61.5\telise\tnil"]);
    assert!(root.join("keep/storage.bin").is_file());
    let none = host(Side::Server);
    start(&none, "n", "print(pcall(storage.get, 'k'))").unwrap();
    assert!(out(&none, "n")[0].contains("no storage"));
    drop(h);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_servers_package_keeps_its_storage_in_the_servers_folder() {
    let root = std::env::temp_dir().join(format!("ssx-script-host-server-storage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (local, server) = (root.join("local"), root.join("0123abcd"));
    std::fs::create_dir_all(&local).unwrap();
    std::fs::create_dir_all(&server).unwrap();
    let mut c = HostConfig::new(Side::Client);
    c.storage_root = Some(local.clone());
    let h = ScriptHost::new(c).unwrap();
    let pkg = |src: &str| files_package("hud", Side::Client, &[("client/main.luau", src)], toml::Table::new()).unwrap();
    h.start_with_storage(pkg("storage.set('laps', 3) print(storage.get('laps'))"), &server).unwrap();
    assert_eq!(out(&h, "hud"), ["3"]);
    h.stop("hud", "the session ended");
    h.start(pkg("print(storage.get('laps'))")).unwrap();
    assert_eq!(out(&h, "hud"), ["nil"]);
    h.stop("hud", "a session");
    h.start_with_storage(pkg("print(storage.get('laps'))"), &server).unwrap();
    assert_eq!(out(&h, "hud"), ["3"]);
    assert!(server.join("hud/storage.bin").is_file());
    drop(h);
    let none = host(Side::Client);
    none.start_with_storage(pkg("print(pcall(storage.get, 'k'))"), &server).unwrap();
    assert!(out(&none, "hud")[0].contains("no storage"));
    let _ = std::fs::remove_dir_all(&root);
}
