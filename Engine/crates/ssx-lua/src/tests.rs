use crate::Lua;

fn run(src: &str, chunk: &str) -> String {
    run_with(src, chunk, false)
}

fn check(src: &str, chunk: &str, expected: &str) {
    assert_eq!(run_with(src, chunk, false), expected);
    assert_eq!(run_with(src, chunk, true), expected, "with the collector at every safe point");
}

fn run_with(src: &str, chunk: &str, stress: bool) -> String {
    let mut l = Lua::new();
    l.heap.stress = stress;
    l.heap.threshold = if stress { 0 } else { l.heap.threshold };
    crate::game::open_libs(&mut l, &crate::game::LIBS_ALL);
    match l.exec(src.as_bytes(), chunk) {
        Ok(_) => l.printed.join("\n"),
        Err(e) => format!("{}\nERROR: {}", l.printed.join("\n"), e.message()),
    }
}

#[test]
fn closures() {
    let src = r#"local function counter() local n = 0 return function() n = n + 1 return n end end
local c1, c2 = counter(), counter()
print(c1(), c1(), c2(), c1())
local fs = {}
for i = 1, 3 do fs[i] = function() return i end end
print(fs[1](), fs[2](), fs[3]())
local gs = {}
for _, v in ipairs({"a", "b"}) do local w = v .. "!" gs[#gs + 1] = function() return v, w end end
print(gs[1]()) print(gs[2]())
local k = 0
local hs = {}
while k < 3 do k = k + 1 local x = k * 10 hs[k] = function() x = x + 1 return x end end
print(hs[1](), hs[1](), hs[2](), hs[3]())
local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end
print(fib(20))
local a, b, c = (function() return 1, 2, 3 end)()
print(a, b, c, select('#', (function() return 1, 2, 3 end)()))
print(select(2, "x", "y", "z"), select(-1, "x", "y", "z"))
local t = {(function() return 1, 2 end)(), (function() return 3, 4 end)()}
print(#t, t[1], t[2], t[3])
print(unpack({1, 2, 3}))
local function va(...) local n = select('#', ...) return n, ... end
print(va(nil, nil))
local function compat(a, ...) return arg.n, arg[1], arg[2] end
print(compat(1, 2, 3))
x, y = 1, 2 x, y = y, x print(x, y)
local q = {} q.a, q.b = 1 print(q.a, q.b)
local function tail(n) if n == 0 then return "done" end return tail(n - 1) end
print(tail(100000))
print(2^10, 7 % 3, -7 % 3, 7 % -3)"#;
    let expected = r#"1	2	1	3
1	2	3
a	a!
b	b!
11	12	21	31
6765
1	2	3	3
y	z
3	1	3	4
1	2	3
2	nil	nil
2	2	3
2	1
1	nil
done
1024	1	2	-2"#;
    check(src, "@closures.lua", expected);
}

#[test]
fn meta() {
    let src = r#"local V = {}
V.__index = V
V.__add = function(a, b) return setmetatable({x = a.x + b.x}, V) end
V.__eq = function(a, b) return a.x == b.x end
V.__lt = function(a, b) return a.x < b.x end
V.__le = function(a, b) return a.x <= b.x end
V.__concat = function(a, b) return "V" .. (type(a) == "table" and a.x or a) .. (type(b) == "table" and b.x or b) end
V.__call = function(self, y) return self.x * y end
V.__tostring = function(self) return "V(" .. self.x .. ")" end
function V.new(x) return setmetatable({x = x}, V) end
function V:get() return self.x end
local a, b = V.new(1), V.new(2)
print((a + b).x, a == V.new(1), a < b, b <= a, a .. "z", "z" .. b, a(5), tostring(a), a:get())
local log = {}
local p = setmetatable({}, {__index = function(t, k) return k .. "?" end, __newindex = function(t, k, v) log[#log + 1] = k rawset(t, k, v) end})
print(p.foo) p.bar = 1 p.bar = 2 print(p.bar, #log, log[1])
local ro = setmetatable({}, {__metatable = "locked"})
print(getmetatable(ro), pcall(setmetatable, ro, {}))
print(getmetatable("abc").__index == string, ("x"):rep(3), #"hello")
local inh = setmetatable({}, {__index = setmetatable({a = 1}, {__index = {b = 2}})})
print(inh.a, inh.b, inh.c)
print(rawequal(a, V.new(1)), rawget(p, "zzz"), next({}))"#;
    let expected = r#"3	true	true	false	V1z	Vz2	5	V(1)	1
foo?
2	1	bar
locked	false	cannot change a protected metatable
true	xxx	5
1	2	nil
false	nil	nil"#;
    check(src, "@meta.lua", expected);
}

#[test]
fn strings() {
    let src = r#"print(string.find("hello world", "o w"), string.find("hello", "l+"), string.find("a.b", ".", 1, true))
print(string.match("key = value", "(%w+)%s*=%s*(%w+)"), string.match("abc", "()b()"))
print(string.gsub("hello world", "o", "0"), string.gsub("abc", "%w", "%0%0"), string.gsub("abc", "", "-"))
print(string.gsub("$name is $age", "%$(%w+)", {name = "bob", age = 3}))
print(string.gsub("abc", "b", function(c) return c:upper() end), ("x y z"):gsub("%s", "", 1))
local acc = {} for w in string.gmatch("one two three", "%a+") do acc[#acc + 1] = w .. "," end print(table.concat(acc))
acc = {} for k, v in string.gmatch("a=1, b=2", "(%w+)=(%w+)") do acc[#acc + 1] = k .. v .. ";" end print(table.concat(acc))
print(string.format("%5d|%-5d|%05d|%x|%X|%o|%c|%s|%q", 42, 42, 42, 255, 255, 8, 65, "hi", 'a"b\n'))
print(string.format("%.3f|%10.2f|%-10.1f|%g|%g|%g|%e", 3.14159, 2.5, 1.25, 0.0001, 100000, 1e20, 12345.678))
print(string.format("%5.2s|%s|%d", "abcdef", 12, 3.99), string.format("%%"))
print(("abc"):byte(1, -1), string.char(72, 105), ("hello"):sub(2, -2), ("hello"):sub(-3), ("x"):sub(5))
print(string.rep("ab", 3), string.upper("abC"), string.lower("ABc"), string.reverse("abc"), string.len("\0a"))
print(string.find("abc", "[a-b]+"), string.find("a]b", "[]]"), string.match("  trim  ", "^%s*(.-)%s*$"))
print(string.match("(foo(bar))baz", "%b()"), string.find("THE (quick) fox", "%f[%a]%a+", 5))
print(pcall(string.rep), pcall(string.find, "a", "(%"), pcall(string.format, "%d", "x"))
print(tostring(12), tostring(-0.5), 10 .. "", 1e100, 123456789012, 0.1 + 0.2, 100 / 3)
print(tonumber("0x1F"), tonumber("  12  "), tonumber("1e2"), tonumber("z", 36), tonumber("12a"), tonumber("ff", 16))
print("10" + 5, "3" * "4", 10 == "10", #{1, 2, nil, 4})"#;
    let expected = r#"5	3	2	2
key	2	3
hell0 w0rld	aabbcc	-a-b-c-	4
bob is 3	2
aBc	xy z	1
one,two,three,
a1;b2;
   42|42   |00042|ff|FF|10|A|hi|"a\"b\
"
3.142|      2.50|1.2       |0.0001|100000|1e+020|1.234568e+004
   ab|12|3	%
97	Hi	ell	llo	
ababab	ABC	abc	cba	2
1	2	trim
(foo(bar))	6	10
false	false	false	bad argument #2 to '?' (number expected, got string)
12	-0.5	10	1e+100	123456789012	0.3	33.333333333333
31	12	100	35	nil	255
15	12	false	4"#;
    check(src, "@str.lua", expected);
}

#[test]
fn co() {
    let src = r#"local function gen(n) return coroutine.wrap(function() for i = 1, n do coroutine.yield(i) end return "end" end) end
local g = gen(3) print(g(), g(), g(), g())
local co = coroutine.create(function(a, b)
  print("start", a, b)
  local c = coroutine.yield(a + b)
  print("got", c)
  local d, e = coroutine.yield(c * 2)
  return d + e
end)
print(coroutine.resume(co, 1, 2))
print(coroutine.status(co))
print(coroutine.resume(co, 10))
print(coroutine.resume(co, 3, 4))
print(coroutine.status(co), coroutine.resume(co))
local bad = coroutine.create(function() error("boom") end)
print(coroutine.resume(bad))
local bad2 = coroutine.create(function() error({code = 1}) end)
local ok, e = coroutine.resume(bad2) print(ok, type(e), e.code)
print(coroutine.resume(coroutine.create(function() return pcall(coroutine.yield, 1) end)))
print(pcall(coroutine.yield, 1))
print(coroutine.running())
local inner = coroutine.create(function() print(coroutine.status(coroutine.running())) coroutine.yield() end)
coroutine.resume(inner)
local nested = coroutine.wrap(function()
  local w = coroutine.wrap(function() coroutine.yield("a") coroutine.yield("b") end)
  coroutine.yield(w() .. w())
end)
print(nested())
print(select('#', coroutine.resume(coroutine.create(function() coroutine.yield() end))))
print(pcall(coroutine.wrap(function() error("x") end)))"#;
    let expected = r#"1	2	3	end
start	1	2
true	3
suspended
got	10
true	20
true	7
dead	false	cannot resume dead coroutine
false	co.lua:15: boom
false	table	1
true	false	attempt to yield across metamethod/C-call boundary
false	attempt to yield across metamethod/C-call boundary
nil
running
ab
1
false	co.lua:30: x"#;
    check(src, "@co.lua", expected);
}

#[test]
fn err() {
    let src = r#"print(pcall(error, "msg"))
print(pcall(function() error("lvl1") end))
print(pcall(function() error("lvl2", 2) end))
print(pcall(function() error("lvl0", 0) end))
print(pcall(function() local x = nil; return x.y end))
print(pcall(function() return undefinedglobal.y end))
print(pcall(function() undefinedfn() end))
print(pcall(function() local t = {} t.a.b = 1 end))
print(pcall(function() return 1 + {} end))
print(pcall(function() return "a" .. {} end))
print(pcall(function() return {} < {} end))
print(pcall(function() return 1 < "2" end))
print(pcall(function() return #nil end))
print(pcall(function() local s = "x" return s:nomethod() end))
print(pcall(function() return ("x"):bad() end))
print(pcall(function() for i = 1, "x" do end end))
print(pcall(function() local t = setmetatable({}, {__index = function(t, k) return t[k] end}) return t.x end) == false)
print(xpcall(function() error("in x") end, function(m) return "handled: " .. m end))
print(select('#', pcall(error)))
print(pcall(error, nil))
print(pcall(error, 42))
local ok, e = pcall(function() local a = {} ; return a.b.c end) print(e)
print(pcall((function() local up = nil; return function() return up.x end end)()))
print(loadstring("return 1 +"))
print(loadstring("x =", "=mychunk"))
print(loadstring("return ...", "c")(1, 2))
local env = {y = 5}
local f = loadstring("return y") setfenv(f, env) print(f(), getfenv(f) == env)
print(pcall(string.sub))
print(pcall(("x").rep, "x", "y"))
print(pcall(setmetatable, 1))
print(pcall(ipairs))
print(pcall(table.insert, {}, 1, 2, 3))
print(table.concat({1, 2, 3}, ", "), pcall(table.concat, {{}}))"#;
    let expected = r#"false	msg
false	err.lua:2: lvl1
false	lvl2
false	lvl0
false	err.lua:5: attempt to index local 'x' (a nil value)
false	err.lua:6: attempt to index global 'undefinedglobal' (a nil value)
false	err.lua:7: attempt to call global 'undefinedfn' (a nil value)
false	err.lua:8: attempt to index field 'a' (a nil value)
false	err.lua:9: attempt to perform arithmetic on a table value
false	err.lua:10: attempt to concatenate a table value
false	err.lua:11: attempt to compare two table values
false	err.lua:12: attempt to compare number with string
false	err.lua:13: attempt to get length of a nil value
false	err.lua:14: attempt to call method 'nomethod' (a nil value)
false	err.lua:15: attempt to call method 'bad' (a nil value)
false	err.lua:16: 'for' limit must be a number
true
false	handled: err.lua:18: in x
2
false	nil
false	42
err.lua:22: attempt to index field 'b' (a nil value)
false	err.lua:23: attempt to index upvalue 'up' (a nil value)
nil	[string "return 1 +"]:1: unexpected symbol near '<eof>'
nil	mychunk:1: unexpected symbol near '<eof>'
1	2
5	true
false	bad argument #1 to '?' (string expected, got no value)
false	bad argument #2 to '?' (number expected, got string)
false	bad argument #1 to '?' (table expected, got number)
false	bad argument #1 to '?' (table expected, got no value)
false	wrong number of arguments to 'insert'
1, 2, 3	false	bad argument #1 to '?' (table contains non-strings)"#;
    check(src, "@err.lua", expected);
}

#[test]
fn order() {
    let src = r#"local t = {} t.a = 1 t.b = 2 t.c = 3
local acc = {} for k in pairs(t) do acc[#acc + 1] = k end print(table.concat(acc, " "))
acc = {} for k in pairs({x = 1, y = 2, z = 3, w = 4}) do acc[#acc + 1] = k end print(table.concat(acc, " "))
acc = {} for k, v in pairs({10, 20, 30, a = 1, [3.5] = 2, [true] = 3, n = 5}) do acc[#acc + 1] = tostring(k) end print(table.concat(acc, " "))
local big = {} for i = 1, 20 do big["k" .. i] = i end
acc = {} for k in pairs(big) do acc[#acc + 1] = k end print(table.concat(acc, " "))
local h = {} h[1] = 1 h[2] = 2 h[4] = 4 print(#h)
local h2 = {1, 2, nil, 4} print(#h2)
local h3 = {} for i = 1, 10 do h3[i] = i end h3[5] = nil print(#h3)
local h4 = {nil, nil, 3} print(#h4)
local s = {5, 2, 8, 1, 9, 3, 7, 4, 6, 0} table.sort(s) print(table.concat(s, ","))
local s2 = {"pear", "apple", "fig", "kiwi"} table.sort(s2, function(a, b) return #a < #b end) print(table.concat(s2, ","))
local s3 = {3, 1, 2} table.sort(s3, function(a, b) return a > b end) print(table.concat(s3, ","))
local r = {1, 2, 3} table.insert(r, 4) table.insert(r, 1, 0) print(table.concat(r, ","), table.remove(r), table.remove(r, 1), table.concat(r, ","), table.maxn({[7] = 1, 2}))
acc = {} for i, v in ipairs({1, 2, nil, 4}) do acc[#acc + 1] = v end print(table.concat(acc, ","))
for i = 10, 1, -3 do acc[#acc + 1] = i end print(table.concat(acc, ","))
for i = 1, 0 do print("never") end
for i = 0.1, 0.35, 0.1 do acc[#acc + 1] = i end print(#acc)
local d = {} d[1.0] = "one" d[2^53] = "big" print(d[1], d[2^53])
local u = {} u.x = nil print(next(u))
local del = {a = 1, b = 2, c = 3} for k in pairs(del) do del[k] = nil end print(next(del))"#;
    let expected = r#"a c b
y x w z
1 2 3 a 3.5 true n
k16 k19 k11 k17 k15 k20 k10 k8 k1 k12 k3 k2 k9 k13 k18 k14 k6 k7 k4 k5
4
4
10
3
0,1,2,3,4,5,6,7,8,9
fig,pear,kiwi,apple
3,2,1
0,1,2,3,4	4	0	1,2,3	7
1,2
1,2,10,7,4,1
9
one	big
nil
nil"#;
    check(src, "@order.lua", expected);
}

#[test]
fn limits_and_budget() {
    let out = run("local function f() return 1 + f() end print(pcall(f))", "@lim.lua");
    assert!(out.starts_with("false\tlim.lua:1: stack overflow"), "{out}");
    let src = "local t = setmetatable({}, {__index = function(t, k) return t[k] end}) print(pcall(function() return t.x end))";
    let out = run(src, "@c.lua");
    assert!(out.contains("C stack overflow"), "{out}");
    let mut l = Lua::new();
    crate::game::open_libs(&mut l, &crate::game::LIBS_ALL);
    l.budget = 10_000;
    let e = l.exec(b"while true do end", "=loop").unwrap_err();
    assert_eq!(e.message(), "loop:1: instruction budget exhausted");
}

#[test]
fn collector_frees_garbage() {
    let mut l = Lua::new();
    crate::game::open_libs(&mut l, &crate::game::LIBS_ALL);
    let src = "keep = {} for i = 1, 50000 do local t = {i} if i % 1000 == 0 then keep[#keep + 1] = t end end";
    l.exec(src.as_bytes(), "=gc").expect("run");
    l.collect_garbage();
    assert!(l.heap.live() < 2000, "live objects after collection: {}", l.heap.live());
    let out = l.exec(b"local n = 0 for _, t in ipairs(keep) do n = n + t[1] end return n", "=gc2").expect("run");
    assert!(matches!(out.first(), Some(crate::Value::Num(n)) if *n == 1_275_000.0));
}

#[test]
fn msvc_number_text() {
    let out = run("print(1e20, -1e-7, 2^63, string.format('%e|%g|%.3g', 1234.5, 1e-10, 0.0001234), 1/0, -1/0)", "=n");
    assert_eq!(out, "1e+020\t-1e-007\t9.2233720368548e+018\t1.234500e+003|1e-010|0.000123\t1.#INF\t-1.#INF");
    let out = run("print(tostring({}):match('^table: %x%x%x%x%x%x%x%x$') ~= nil, tostring(print):sub(1, 10))", "=p");
    assert_eq!(out, "true\tfunction: ");
}

#[test]
fn packages_through_the_host_reader() {
    use std::rc::Rc;
    let rec = Rc::new(std::cell::RefCell::new(crate::game::record::Recorder::default()));
    let read: crate::vm::FileReader = Rc::new(|f: &str| match f {
        "./mod1.lua" => Some(b"module('mod1') value = 42 function get() return value end".to_vec()),
        "./mod2.lua" => Some(b"local P = {x = 1} return P".to_vec()),
        _ => None,
    });
    let mut l = crate::game::record::frontend_state(&rec, read, ".");
    let out = l
        .exec(b"require 'mod1' local m2 = require 'mod2' return mod1.get(), m2.x, require('mod2') == m2, pcall(require, 'nope')", "=r")
        .expect("run");
    let txt: Vec<String> = out.iter().map(|v| l.default_tostring(v)).collect();
    assert_eq!(txt[..4], ["42", "1", "true", "false"]);
    assert!(txt[4].contains("module 'nope' not found:"), "{}", txt[4]);
}

#[test]
fn binding_stubs_record_and_check() {
    use crate::game::Binder;
    use crate::game::record::{Recorder, world_state};
    let rec = std::rc::Rc::new(std::cell::RefCell::new(Recorder { keep_log: true, ..Default::default() }));
    let mut l = world_state(Binder::Objectives, &rec, true);
    let src = "BeginObjective(\"A\")\n WAIT_GameStateValue(\"Player1\", BLN_NisActive, EqualTo, false)\n DO_SetTemporaryNamedFlag(\"F\", true)\nEndObjective()";
    l.exec(src.as_bytes(), "t.lua").expect("run");
    assert_eq!(
        rec.borrow().log,
        [
            "BeginObjective(\"A\")",
            "WAIT_GameStateValue(\"Player1\", 131, 2, false)",
            "DO_SetTemporaryNamedFlag(\"F\", true)",
            "EndObjective()"
        ]
    );
    let e = l.exec(b"BeginObjective(1, 2)\nWAIT_Time('x')", "u.lua").unwrap_err();
    assert_eq!(e.message(), "[string \"u.lua\"]:2: bad argument #1 to 'WAIT_Time' (number expected, got string)");
    let mut s = world_state(Binder::StateSets, &rec, true);
    let src = "local s = CreateStateSet('S') local st = s:AddState('A') local t = LightTrack('g') t:BlinkOnUpdate(0.5, 2) st:AddUpdateTrack(t) return st:AddTransition('B')";
    let r = s.exec(src.as_bytes(), "s.lua").expect("run");
    assert!(matches!(r.first(), Some(crate::Value::UserData(_))));
    let e = s.exec(b"local t = LightTrack('g') t:NoSuchMethod()", "v.lua").unwrap_err();
    assert_eq!(e.message(), "[string \"v.lua\"]:1: attempt to call method 'NoSuchMethod' (a nil value)");
}

#[test]
#[ignore]
fn bench_interpreter() {
    let mut l = Lua::new();
    crate::game::open_libs(&mut l, &crate::game::LIBS_ALL);
    l.budget = u64::MAX;
    let src = "local x = 0 for i = 1, 5000000 do x = x + i % 7 end \
               local function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end \
               local t = {} for i = 1, 200000 do t[i] = {v = i} end return x, fib(25)";
    let t0 = std::time::Instant::now();
    l.exec(src.as_bytes(), "=bench").expect("run");
    let dt = t0.elapsed().as_secs_f64();
    println!("{} instructions in {dt:.3} s: {:.1} M/s", l.executed, l.executed as f64 / dt / 1e6);
}

#[test]
fn table_setn_is_obsolete_and_unpack_takes_the_games_stack() {
    let src = r#"print(table.getn({1, 2, 3, n = 100}))
print(pcall(table.setn, {}, 7))
print(pcall(table.setn, 1, 7))
local t = {} for k = 1, 8001 do t[k] = k end
print(select('#', unpack(t)))
print(select('#', unpack(t, 1, 32764)))
print(pcall(unpack, t, 1, 32765))
print(select('#', unpack(t, 3, 2)), select('#', unpack(t, 2147483647, -2147483648)))
print(pcall(unpack, t, 1, 32765, "x"))"#;
    let out = run(src, "t");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "3");
    assert!(lines[1].starts_with("false") && lines[1].ends_with("'setn' is obsolete"), "{}", lines[1]);
    assert!(lines[2].contains("table expected"), "{}", lines[2]);
    assert_eq!(&lines[3..5], ["8001", "32764"]);
    assert!(lines[5].starts_with("false") && lines[5].ends_with("stack overflow (table too big to unpack)"));
    assert_eq!(lines[6], "0\t0");
    assert!(lines[7].starts_with("false"), "a fourth argument takes a slot: {}", lines[7]);
}

#[test]
fn the_parsers_limits_are_the_games() {
    let locals = |n: usize| format!("local {}", (0..n).map(|k| format!("v{k}")).collect::<Vec<_>>().join(", "));
    let ups = |n: usize| {
        let decl = (0..n).map(|k| format!("local u{k} = {k}")).collect::<Vec<_>>().join("\n");
        let body = (0..n).map(|k| format!("u{k}")).collect::<Vec<_>>().join(" + ");
        format!("local function outer()\n{decl}\nreturn function() return {body} end\nend")
    };
    let ok = |src: String| !run(&src, "t").contains("ERROR");
    assert!(ok(locals(200)) && !ok(locals(201)));
    assert!(run(&locals(201), "t").contains("local variables"));
    assert!(ok(ups(60)) && !ok(ups(61)));
    assert!(run(&ups(61), "t").contains("upvalues"));
}
