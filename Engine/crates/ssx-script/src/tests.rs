use std::time::Duration;

use mlua::Value;

use super::*;

mod hostile;

const MARGIN: Duration = Duration::from_millis(250);

const OUT_OF_REACH: u64 = 100_000_000;

fn cpu_now() -> Duration {
    Duration::from_nanos(crate::cpu::charged())
}

fn limits(checkpoints: u64, memory: usize) -> Limits {
    Limits { call: checkpoints, call_backstop: Limits::MAX_BUDGET, memory }
}

fn backstop(ms: u64) -> Limits {
    Limits { call: Limits::MAX_CHECKPOINTS, call_backstop: Duration::from_millis(ms), memory: 64 << 20 }
}

fn vm() -> ScriptVm {
    ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, 32 << 20)).unwrap()
}

fn files(list: &[(&str, &str)]) -> FileMap {
    list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn stopped_by_budget(n: u64, src: &str) {
    let vm = ScriptVm::new(FileMap::new(), limits(n, 32 << 20)).unwrap();
    let r = vm.run_source("budget", src);
    assert!(matches!(r, Err(ScriptError::Budget(Overrun::Checkpoints(c))) if c == n), "{src}: {:?}", r.err());
    let passed = vm.interrupts();
    assert!((n + 1..n + 16).contains(&passed), "{src}: {passed} checkpoints for a budget of {n}");
    assert!(vm.stopped().is_some());
    assert!(matches!(vm.eval::<i32>("return 1"), Err(ScriptError::Stopped(_))), "a stopped VM runs nothing");
}

fn stopped_by_backstop(ms: u64, src: &str) {
    let vm = ScriptVm::new(FileMap::new(), backstop(ms)).unwrap();
    let t = cpu_now();
    let r = vm.run_source("backstop", src);
    let took = cpu_now() - t;
    let want = Duration::from_millis(ms);
    assert!(matches!(r, Err(ScriptError::Budget(Overrun::Backstop(b))) if b == want), "{src}: {:?}", r.err());
    assert!(took >= want && took < want + MARGIN, "{src}: {took:?}");
    assert!(vm.stopped().is_some());
}

fn stopped_by_memory(memory: usize, src: &str) {
    let vm = ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, memory)).unwrap();
    let r = vm.run_source("memory", src);
    assert!(matches!(r, Err(ScriptError::Memory(_))), "{src}: {:?}", r.err());
    assert!(vm.used_memory() <= memory, "{src}: {} > {memory}", vm.used_memory());
    assert!(vm.stopped().is_some());
}

#[test]
fn an_endless_loop_is_stopped_at_its_budgets_checkpoint() {
    stopped_by_budget(100_000, "while true do end");
    stopped_by_budget(100_000, "repeat until false");
    stopped_by_budget(100_000, "for i = 1, math.huge do end");
    stopped_by_budget(1, "while true do end");
    let counts: Vec<u64> = (0..3)
        .map(|_| {
            let vm = ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, 32 << 20)).unwrap();
            vm.run_source("n", "local t = {} for i = 1, 5000 do t[#t + 1] = tostring(i) end").unwrap();
            vm.interrupts()
        })
        .collect();
    assert!(counts.windows(2).all(|w| w[0] == w[1]) && counts[0] >= 5000, "{counts:?}");
}

#[test]
fn the_backstop_stops_work_between_checkpoints() {
    stopped_by_backstop(50, &format!("local x = 0 while true do {} end", "x = x + 1 ".repeat(20_000)));
    stopped_by_backstop(
        50,
        "local t = {} for i = 1, 100000 do t[i] = (i * 7919) % 100003 end while true do table.sort(t) end",
    );
    stopped_by_backstop(
        50,
        "local p = {} for i = 1, 50000 do p[i] = tostring(i) end local s = table.concat(p, ',') p = nil \
         while true do local parts = string.split(s, ',') end",
    );
    stopped_by_backstop(50, "while true do end");
}

#[test]
fn pcall_and_error_handlers_cannot_swallow_the_budget() {
    stopped_by_budget(100_000, "while true do pcall(function() while true do end end) end");
    stopped_by_budget(100_000, "while true do xpcall(function() while true do end end, function() return 'x' end) end");
    stopped_by_budget(100_000, "xpcall(function() error('x') end, function() while true do end end) while true do end");
    stopped_by_budget(100_000, "error(setmetatable({}, {__tostring = function() while true do end end}))");
    stopped_by_budget(100_000, "local t = setmetatable({}, {__index = function() while true do end end}) return t.x");
}

#[test]
fn coroutines_cannot_evade_the_budget() {
    stopped_by_budget(
        100_000,
        "local co = coroutine.wrap(function() while true do coroutine.yield() end end) while true do co() end",
    );
    stopped_by_budget(
        100_000,
        "while true do coroutine.resume(coroutine.create(function() while true do end end)) end",
    );
    stopped_by_budget(
        100_000,
        "local co = coroutine.create(function() while true do end end) coroutine.resume(co) while true do end",
    );
    stopped_by_budget(100_000, "for i = 1, 1e9 do coroutine.wrap(function() end)() end");
}

#[test]
fn library_loops_reach_the_interrupt() {
    stopped_by_budget(100_000, "local s = string.rep('a', 3000) return s:find(string.rep('a*', 12) .. 'b')");
    stopped_by_budget(100_000, "local s = string.rep('a', 3000) return s:gsub(string.rep('a-', 12) .. 'b', '')");
    stopped_by_budget(100_000, "table.sort({3, 2, 1, 4, 5, 6}, function() while true do end end)");
    stopped_by_budget(100_000, "string.gsub(string.rep('x', 100), '.', function() while true do end end)");
}

#[test]
fn the_budget_is_per_call() {
    let vm = ScriptVm::new(FileMap::new(), limits(100_000, 32 << 20)).unwrap();
    let s = vm.run_source("h", "function spin(n) for i = 1, n do end return n end").unwrap();
    let spin = vm.handler(&s, "spin").unwrap().unwrap();
    for _ in 0..8 {
        assert_eq!(vm.call::<u32>(&spin, 60_000).unwrap(), 60_000);
    }
    assert!(matches!(vm.call::<u32>(&spin, 400_000), Err(ScriptError::Budget(Overrun::Checkpoints(100_000)))));
}

#[test]
fn memory_growth_is_stopped_at_the_limit() {
    let m = 16 << 20;
    stopped_by_memory(m, "local t = {} local i = 0 while true do i += 1 t[i] = string.rep('x', 65536) .. i end");
    stopped_by_memory(m, "local s = string.rep('x', 2^30)");
    stopped_by_memory(m, "local s = 'x' while true do s = s .. s end");
    stopped_by_memory(m, "local b = buffer.create(1e9)");
    stopped_by_memory(m, "local ts = {} while true do ts[#ts + 1] = table.create(1e5, 0) end");
    stopped_by_memory(m, "local t = {} while true do t[#t + 1] = {} end");
    stopped_by_memory(m, "local t = {} local i = 0 while true do i += 1 t[tostring(i) .. string.rep('k', 64)] = i end");
    stopped_by_memory(m, "local fs = {} while true do local x = {} fs[#fs + 1] = function() return x end end");
    stopped_by_memory(m, "local t = {} while true do t[#t + 1] = coroutine.create(function() end) end");
    let vm = ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, m)).unwrap();
    let Err(ScriptError::Script(e)) = vm.eval::<()>("local t = table.create(1e8, 0)") else { panic!("not refused") };
    assert!(e.contains("table overflow"), "{e}");
}

#[test]
fn limits_are_checked() {
    let over = Limits { call_backstop: Duration::from_secs(20), ..limits(10, 32 << 20) };
    let zero = Limits { call_backstop: Duration::ZERO, ..limits(10, 32 << 20) };
    let many = limits(Limits::MAX_CHECKPOINTS + 1, 32 << 20);
    for bad in [limits(0, 32 << 20), limits(10, 0), limits(10, 1000), over, zero, many] {
        assert!(matches!(ScriptVm::new(FileMap::new(), bad), Err(ScriptError::BadLimits(_))), "{bad:?}");
    }
    assert!(
        ScriptVm::new(FileMap::new(), Limits::CLIENT).is_ok() && ScriptVm::new(FileMap::new(), Limits::SERVER).is_ok()
    );
}

#[test]
fn require_stays_inside_the_package() {
    let fm = files(&[
        ("shared/config.luau", "return { speed = 3 }"),
        ("shared/util/init.luau", "return { twice = function(x) return 2 * x end }"),
        ("client/main.luau", "local c = require('shared/config') speed = c.speed"),
        ("loop/a.luau", "return require('loop/b')"),
        ("loop/b.luau", "return require('loop/a')"),
        ("bad.luau", "error('module failed')"),
    ]);
    let vm = ScriptVm::new(fm, limits(OUT_OF_REACH, 32 << 20)).unwrap();
    assert_eq!(vm.eval::<i32>("return require('shared/config').speed").unwrap(), 3);
    assert_eq!(vm.eval::<i32>("return require('shared/util').twice(4)").unwrap(), 8);
    assert!(vm.eval::<bool>("return require('./shared/config') == require('shared/config.luau')").unwrap());
    let main = vm.run_file("client/main.luau").unwrap();
    assert_eq!(vm.global::<i32>(&main, "speed").unwrap(), 3);
    for bad in [
        "../x",
        "shared/../client/main",
        "/etc/passwd",
        "C:/Windows/x",
        "shared\\\\config",
        "@self/config",
        "",
        "missing",
        "client",
    ] {
        let e = vm.eval::<Value>(&format!("return require('{bad}')")).unwrap_err().to_string();
        assert!(e.contains("require("), "{bad}: {e}");
    }
    assert!(vm.eval::<Value>("return require({})").is_err());
    assert!(vm.eval::<Value>("return require('loop/a')").unwrap_err().to_string().contains("requires itself"));
    assert!(vm.eval::<Value>("return require('bad')").unwrap_err().to_string().contains("module failed"));
    assert!(vm.eval::<Value>("return require('bad')").unwrap_err().to_string().contains("module failed"), "retried");
    assert!(matches!(vm.run_file("nope.luau"), Err(ScriptError::NoFile(_))));
}

#[test]
fn global_writes_do_not_leak() {
    let vm = vm();
    let a = vm.run_source("a", "leaked = 1; print = nil; loadstring('alsoleaked = 2')()").unwrap();
    assert_eq!(vm.global::<i32>(&a, "leaked").unwrap(), 1);
    assert_eq!(vm.global::<i32>(&a, "alsoleaked").unwrap(), 2, "loadstring writes into its caller's environment");
    let b = "return leaked == nil and alsoleaked == nil and loadstring('return alsoleaked')() == nil and type(print) == 'function'";
    assert!(vm.eval::<bool>(b).unwrap());
    let other = ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, 32 << 20)).unwrap();
    other.run_source("b", "shared_value = 5").unwrap();
    assert!(vm.eval::<bool>("return shared_value == nil").unwrap(), "VMs are separate");
}

#[test]
fn handles_stay_in_their_vm() {
    let (a, b) = (vm(), vm());
    let sa = a.run_source("a", "function f() return 1 end").unwrap();
    let fa = a.handler(&sa, "f").unwrap().unwrap();
    assert_eq!(a.call::<i32>(&fa, ()).unwrap(), 1);
    assert!(matches!(b.call::<i32>(&fa, ()), Err(ScriptError::ForeignHandle)));
    assert!(matches!(b.global::<Value>(&sa, "f"), Err(ScriptError::ForeignHandle)));
    assert!(a.handler(&sa, "missing").unwrap().is_none());
}
