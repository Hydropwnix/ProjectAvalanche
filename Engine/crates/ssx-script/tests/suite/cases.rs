use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::Limits;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{ClientId, HostConfig, HostLimits, ScriptHost, Side};

#[path = "budget.rs"]
mod budget;

pub enum Expect {
    Stopped(&'static str),
    StartFails(&'static str),
    Prints(&'static str),
}

pub struct Case {
    pub name: &'static str,
    pub premise: &'static str,
    pub side: Side,
    pub files: fn() -> Vec<(&'static str, String)>,
    pub ticks: u32,
    pub limits: Lim,
    pub client: bool,
    pub others: Option<fn() -> Vec<Other>>,
    pub expect: Expect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lim {
    Side,
    Relaxed,
    TickOnly,
}

pub type Other = (&'static str, Vec<(&'static str, String)>);

pub struct Report {
    pub outcome: &'static str,
    pub reason: String,
    pub canary_ran: bool,
    pub host_answers: bool,
    pub leftovers: bool,
    pub others_running: bool,
    pub longest_checkpoints: u64,
    pub longest_call: Duration,
    pub call_budget: u64,
    pub call_backstop: Duration,
    pub chain_bound: u64,
    pub chain_backstop: Duration,
}

const CANARY: &str = "n = 0 setTimer(function() n += 1 print('canary', n) end, 50, 0)";

fn package(
    name: &str,
    side: Side,
    files: &[(&'static str, String)],
    limits: &HostLimits,
) -> Result<PackageScripts, String> {
    let scripts =
        Scripts { client: vec!["client/**/*.luau".into()], server: vec!["server/**/*.luau".into()], shared: vec![] };
    let files_cfg = Files { include: vec!["lib/**".into()], ..Files::default() };
    let contents: Vec<(String, String)> = files.iter().map(|(p, s)| (p.to_string(), s.clone())).collect();
    PackageScripts::from_memory(name, side, &scripts, &files_cfg, toml::Table::new(), &contents, limits)
}

fn main_file(side: Side) -> &'static str {
    match side {
        Side::Server => "server/main.luau",
        Side::Client => "client/main.luau",
    }
}

pub fn run(case: &Case) -> Report {
    let mut config = HostConfig::new(case.side);
    let b = &mut config.limits.budgets;
    if case.limits != Lim::Side {
        (b.call, b.call_backstop) = (Limits::MAX_CHECKPOINTS, Duration::from_secs(9));
        (b.chain, b.chain_backstop) = (u64::MAX, Duration::from_secs(18));
    }
    if case.limits == Lim::Relaxed {
        (b.tick, b.tick_checkpoints, b.tick_backstop) = (Duration::from_secs(9), u64::MAX, Duration::from_secs(9));
    }
    let limits = config.limits;
    let host = ScriptHost::new(config).expect("a host");
    if case.client {
        host.connect_client(ClientId(1)).expect("a client");
    }
    let canary = package("canary", case.side, &[(main_file(case.side), CANARY.to_string())], &limits);
    host.start(canary.expect("the canary loads")).expect("the canary starts");
    let others = case.others.map(|f| f()).unwrap_or_default();
    for (name, files) in &others {
        host.start(package(name, case.side, files, &limits).expect("another package loads"))
            .expect("another package starts");
    }
    let started = package("hostile", case.side, &(case.files)(), &limits).and_then(|p| host.start(p));
    let mut now = Duration::ZERO;
    for _ in 0..case.ticks {
        now += Duration::from_millis(100);
        host.tick(now);
    }
    host.take_output("canary");
    for _ in 0..3 {
        now += Duration::from_millis(100);
        host.tick(now);
    }
    let canary_lines = host.take_output("canary").map(|o| o.lines).unwrap_or_default();
    let canary_ran = host.running().iter().any(|n| n == "canary") && canary_lines.len() >= 2;
    let monitor = host.monitor();
    let host_answers = monitor.packages.iter().any(|p| p.name == "canary");
    let running = host.running().iter().any(|n| n == "hostile");
    let others_running = others.iter().all(|(o, _)| host.running().iter().any(|n| n == o));
    let (held, vm) = host.holdings("hostile");
    let leftovers = !running && (held != Default::default() || vm);
    let stop = monitor.stops.iter().rev().find(|s| s.package == "hostile").map(|s| s.reason.to_string());
    let (outcome, reason) = match (&started, stop, running) {
        (Err(e), _, _) => ("refused at its start", e.clone()),
        (Ok(()), Some(r), false) => ("stopped", r),
        (Ok(()), _, _) => {
            let lines = host.take_output("hostile").map(|o| o.lines).unwrap_or_default();
            ("still running; it printed", lines.join(" | "))
        }
    };
    Report {
        outcome,
        reason,
        canary_ran,
        host_answers,
        leftovers,
        others_running,
        longest_checkpoints: monitor.chain_peak_checkpoints,
        longest_call: monitor.chain_peak,
        call_budget: monitor.budgets.call,
        call_backstop: monitor.budgets.call_backstop,
        chain_bound: monitor.budgets.chain,
        chain_backstop: monitor.budgets.chain_backstop,
    }
}

pub fn check(case: &Case, r: &Report) -> Result<(), String> {
    let ok = match &case.expect {
        Expect::Stopped(why) => r.outcome == "stopped" && r.reason.contains(why),
        Expect::StartFails(why) => r.outcome == "refused at its start" && r.reason.contains(why),
        Expect::Prints(what) => r.outcome == "still running; it printed" && r.reason.contains(what),
    };
    if !ok {
        return Err(format!("{}: {} — {}", case.name, r.outcome, r.reason));
    }
    if !r.canary_ran || !r.host_answers {
        return Err(format!("{}: the host or the canary did not go on", case.name));
    }
    if r.leftovers {
        return Err(format!("{}: the stopped package left something behind", case.name));
    }
    if !r.others_running {
        return Err(format!("{}: another package of the case was stopped", case.name));
    }
    let early = (r.reason.contains(COUNT) && r.longest_checkpoints <= r.call_budget)
        || (r.reason.contains(BACKSTOP) && r.longest_call < r.call_backstop)
        || (r.reason.contains(CHAIN_COUNT) && r.longest_checkpoints <= r.chain_bound)
        || (r.reason.contains(CHAIN_BACKSTOP) && r.longest_call < r.chain_backstop);
    if early {
        let (n, cpu) = (r.longest_checkpoints, r.longest_call);
        return Err(format!("{}: a stop before its bound ({n} checkpoints, {cpu:?}): {}", case.name, r.reason));
    }
    Ok(())
}

fn client(src: &'static str) -> Vec<(&'static str, String)> {
    vec![("client/main.luau", src.to_string())]
}

fn server(src: &'static str) -> Vec<(&'static str, String)> {
    vec![("server/main.luau", src.to_string())]
}

fn later(side: Side, src: &'static str) -> Vec<(&'static str, String)> {
    vec![(main_file(side), format!("setTimer(function() {src} end, 50, 1)"))]
}

const fn case(
    name: &'static str,
    premise: &'static str,
    side: Side,
    files: fn() -> Vec<(&'static str, String)>,
    expect: Expect,
) -> Case {
    Case { name, premise, side, files, ticks: 0, limits: Lim::Relaxed, client: false, others: None, expect }
}

const fn budget_case(
    name: &'static str,
    premise: &'static str,
    side: Side,
    files: fn() -> Vec<(&'static str, String)>,
    expect: Expect,
) -> Case {
    Case { limits: Lim::Side, ..case(name, premise, side, files, expect) }
}

const fn budget_running(
    name: &'static str,
    premise: &'static str,
    side: Side,
    files: fn() -> Vec<(&'static str, String)>,
    expect: Expect,
) -> Case {
    Case { limits: Lim::Side, ..running(name, premise, side, files, expect) }
}

const fn running(
    name: &'static str,
    premise: &'static str,
    side: Side,
    files: fn() -> Vec<(&'static str, String)>,
    expect: Expect,
) -> Case {
    Case { name, premise, side, files, ticks: 1, limits: Lim::Relaxed, client: false, others: None, expect }
}

const C: Side = Side::Client;
const S: Side = Side::Server;
const BUDGET: &str = "ran past its call budget";
const COUNT: &str = "ran past its call budget of";
const BACKSTOP: &str = "ran past its call budget's backstop of";
const MEMORY: &str = "went over its memory limit";
const CHAIN: &str = "past its chain bound";
const CHAIN_COUNT: &str = "past its chain bound of";
const CHAIN_BACKSTOP: &str = "past its chain bound's backstop of";
pub fn cases() -> Vec<Case> {
    let mut v = budget::cases();
    v.extend([
        running(
            "table_growth",
            "table growth is stopped at the heap cap",
            C,
            || later(C, "local t = {} while true do t[#t + 1] = {} end"),
            Expect::Stopped(MEMORY),
        ),
        running(
            "concat_bomb",
            "string doubling is stopped at the heap cap",
            C,
            || later(C, "local s = 'x' while true do s = s .. s end"),
            Expect::Stopped(MEMORY),
        ),
        running(
            "buffer_bomb",
            "buffer.create beyond the heap is refused by the allocator",
            C,
            || later(C, "local b = buffer.create(1e9)"),
            Expect::Stopped(MEMORY),
        ),
        running(
            "rep_bomb",
            "string.rep with a huge count is refused by the allocator",
            C,
            || later(C, "local s = string.rep('x', 2^30)"),
            Expect::Stopped(MEMORY),
        ),
        running(
            "closure_bomb",
            "a table of closures is stopped at the heap cap",
            C,
            || later(C, "local fs = {} while true do local x = {} fs[#fs + 1] = function() return x end end"),
            Expect::Stopped(MEMORY),
        ),
        running(
            "server_heap",
            "the server's VMs stop at their own cap (256 MiB)",
            S,
            || later(S, "local t = {} local i = 0 while true do i += 1 t[i] = string.rep('x', 65536) .. i end"),
            Expect::Stopped("256 MiB"),
        ),
        case(
            "no_debug",
            "debug has nothing beyond traceback",
            C,
            || client("print('debug', debug.getinfo, debug.sethook, debug.getlocal, debug.traceback ~= nil)"),
            Expect::Prints("debug\tnil\tnil\tnil\ttrue"),
        ),
        case(
            "no_io_os",
            "no io, no os.execute / getenv / exit / remove",
            C,
            || client("print('io', io, os.execute, os.getenv, os.exit, os.remove, os.clock ~= nil)"),
            Expect::Prints("io\tnil\tnil\tnil\tnil\tnil\ttrue"),
        ),
        case(
            "no_fenv",
            "getfenv and setfenv are gone",
            C,
            || client("print('fenv', getfenv, setfenv)"),
            Expect::Prints("fenv\tnil\tnil"),
        ),
        case(
            "loadstring_stays_inside",
            "loadstring writes only into its caller's environment",
            C,
            || {
                vec![
                    ("client/a.luau", "loadstring('leaked = 1')()".to_string()),
                    ("client/b.luau", "print('leak', leaked)".to_string()),
                ]
            },
            Expect::Prints("leak\tnil"),
        ),
        case(
            "no_bytecode",
            "bytecode never loads",
            C,
            || client("print('bytecode', loadstring(string.char(6, 3, 0, 0)))"),
            Expect::Prints("binary chunk"),
        ),
        case(
            "require_up",
            "require of ../ is refused",
            C,
            || client("print(pcall(require, '../x'))"),
            Expect::Prints("only the package's own modules"),
        ),
        case(
            "require_absolute",
            "require of an absolute path or a drive is refused",
            C,
            || client("print(pcall(require, '/etc/passwd')) print(pcall(require, 'C:/x'))"),
            Expect::Prints("only the package's own modules"),
        ),
        case(
            "require_case",
            "require matches names exactly (no case folding)",
            C,
            || {
                vec![
                    ("client/main.luau", "print(pcall(require, 'lib/Mod'))".to_string()),
                    ("lib/mod.luau", "return 1".to_string()),
                ]
            },
            Expect::Prints("no such module"),
        ),
        case(
            "case_collision",
            "two spellings of one path refuse the package",
            C,
            || vec![("client/main.luau", String::new()), ("lib/a.luau", String::new()), ("lib/A.luau", String::new())],
            Expect::StartFails("two spellings"),
        ),
        case(
            "readonly_globals",
            "the libraries and globals are read-only",
            C,
            || {
                client(
                    "print('ro', pcall(function() string.upper = nil end), pcall(function() _G.x = 1 end), pcall(setmetatable, _G, {}))",
                )
            },
            Expect::Prints("ro\tfalse"),
        ),
        case(
            "api_tables_readonly",
            "the API's tables (exports, json, storage) are frozen and exports' metatable locked",
            C,
            || {
                client(
                    "local writes = 0 for _, f in ipairs({function() exports.x = 1 end, function() rawset(exports, 'x', 1) end, \
                     function() json.encode = print end, function() storage.set = print end, function() setmetatable(exports, {}) end}) do \
                     if pcall(f) then writes += 1 end end print('api writes', writes, getmetatable(exports))",
                )
            },
            Expect::Prints("api writes\t0\tlocked"),
        ),
        case(
            "event_flood",
            "event handlers stop at their cap (1 024)",
            C,
            || {
                client(
                    "addEvent('flood') for i = 1, 2000 do local ok, e = pcall(addEventHandler, 'flood', root, function() return i end) if not ok then print(i, e) break end end",
                )
            },
            Expect::Prints("1025\t"),
        ),
        case(
            "timer_flood",
            "timers stop at their cap (1 024)",
            C,
            || {
                client(
                    "for i = 1, 2000 do local ok, e = pcall(setTimer, print, 1000, 1) if not ok then print(i, e) break end end",
                )
            },
            Expect::Prints("1024 timers"),
        ),
        case(
            "data_key_flood",
            "data keys stop at their cap (16 384)",
            S,
            || {
                server(
                    "for i = 1, 20000 do local ok, e = pcall(setElementData, resourceRoot, 'k' .. i, i) if not ok then print(i, e) break end end",
                )
            },
            Expect::Prints("16384 data keys"),
        ),
        case(
            "data_byte_flood",
            "data bytes stop at their cap (4 MiB)",
            S,
            || {
                server(
                    "local v = string.rep('x', 60000) for i = 1, 100 do local ok, e = pcall(setElementData, resourceRoot, 'k' .. i, v) if not ok then print(i, e) break end end",
                )
            },
            Expect::Prints("4194304"),
        ),
        case(
            "element_flood",
            "elements stop at their cap (4 096)",
            C,
            || {
                client(
                    "for i = 1, 5000 do local ok, e = pcall(createElement, 'thing') if not ok then print(i, e) break end end",
                )
            },
            Expect::Prints("4096 elements"),
        ),
        Case {
            client: true,
            ..case(
                "outbox_flood",
                "a package's messages stop at their per-tick bound (1 024)",
                S,
                || {
                    server(
                        "for i = 1, 2000 do local ok, e = pcall(triggerClientEvent, root, 'x', resourceRoot, i) if not ok then print(i, e) break end end",
                    )
                },
                Expect::Prints("1025\t"),
            )
        },
        Case {
            client: true,
            ..case(
                "latent_flood",
                "latent transfers stop at their cap (16)",
                S,
                || {
                    server(
                        "for i = 1, 20 do local ok, e = pcall(triggerLatentClientEvent, root, 'x', 50000, resourceRoot, i) if not ok then print(i, e) break end end",
                    )
                },
                Expect::Prints("17\t"),
            )
        },
        case(
            "value_depth",
            "a value 17 deep is not copied",
            S,
            || server("local t = 1 for i = 1, 17 do t = {t} end print(pcall(setElementData, resourceRoot, 'k', t))"),
            Expect::Prints("deeper than 16"),
        ),
        case(
            "value_long_string",
            "a 65 KiB string is not copied",
            S,
            || server("print(pcall(setElementData, resourceRoot, 'k', string.rep('x', 66560)))"),
            Expect::Prints("at most 65536"),
        ),
        case(
            "value_cycle",
            "a cycle is not copied",
            S,
            || server("local t = {} t.t = t print(pcall(setElementData, resourceRoot, 'k', t))"),
            Expect::Prints("contains itself"),
        ),
        case(
            "value_shared_subtable",
            "a subtable shared twice counts twice",
            S,
            || {
                server(
                    "local t = {string.rep('x', 40000)} print(pcall(setElementData, resourceRoot, 'k', t)) print(pcall(setElementData, resourceRoot, 'k', {t, t}))",
                )
            },
            Expect::Prints("over 65536 bytes"),
        ),
        case(
            "value_metatable",
            "a table with a metatable is not copied",
            S,
            || server("print(pcall(setElementData, resourceRoot, 'k', setmetatable({}, {})))"),
            Expect::Prints("metatable"),
        ),
        case(
            "value_function",
            "a function is not copied",
            S,
            || server("print(pcall(setElementData, resourceRoot, 'k', {f = print}))"),
            Expect::Prints("a function cannot be copied"),
        ),
        case(
            "deep_nesting",
            "1 000 nested parentheses are refused by the parser, not a stack overflow",
            C,
            || vec![("client/main.luau", format!("return {}1{}", "(".repeat(1000), ")".repeat(1000)))],
            Expect::StartFails("Exceeded allowed recursion depth"),
        ),
        case(
            "million_terms",
            "a 10^6-term expression is refused by the source cap before the compiler sees it",
            C,
            || vec![("client/main.luau", format!("return 1{}", "+1".repeat(999_999)))],
            Expect::StartFails("over 1048576 bytes"),
        ),
        case(
            "long_chain",
            "a 200 000-term expression under the cap is refused by the parser",
            C,
            || vec![("client/main.luau", format!("return 1{}", "+1".repeat(200_000)))],
            Expect::StartFails("Exceeded allowed recursion depth"),
        ),
        case(
            "huge_literal",
            "a string literal over the cap is refused before the compiler",
            C,
            || vec![("client/main.luau", format!("return '{}'", "x".repeat(1 << 20)))],
            Expect::StartFails("over 1048576 bytes"),
        ),
        case(
            "deep_method_chain",
            "the stack-hungriest accepted source (998 method calls) compiles off the caller's stack",
            C,
            || {
                vec![(
                    "client/main.luau",
                    format!(
                        "local x = setmetatable({{}}, {{__index = function(t) return function() return t end end}}) local y = x{} print('chain', y == x)",
                        ":m()".repeat(998)
                    ),
                )]
            },
            Expect::Prints("chain\ttrue"),
        ),
        case(
            "loadstring_deep",
            "loadstring of a 1 000-deep source is refused with the parser's reason",
            C,
            || client("print(loadstring('return ' .. string.rep('(', 1000) .. '1' .. string.rep(')', 1000)))"),
            Expect::Prints("Exceeded allowed recursion depth"),
        ),
        Case {
            client: true,
            ticks: 2,
            ..budget_case(
                "leftovers",
                "a stopped package leaves no element, data, handler, timer, export, message or transfer",
                S,
                || {
                    server(
                        "addEvent('e', true) addEventHandler('e', root, print) createElement('thing') setElementData(root, 'k', 1)\n\
                      exports('f', print) addCommandHandler('c', print) triggerClientEvent(root, 'x', resourceRoot, 1)\n\
                      triggerLatentClientEvent(root, 'y', 500, resourceRoot, string.rep('z', 60000))\n\
                      setTimer(function() while true do end end, 100, 1)",
                    )
                },
                Expect::Stopped(BUDGET),
            )
        },
    ]);
    v.sort_by_key(|c| c.name);
    v
}
