use super::*;

#[test]
fn forbidden_libraries_are_absent() {
    let vm = vm();
    for absent in [
        "io",
        "os.exit",
        "os.execute",
        "os.getenv",
        "os.remove",
        "os.rename",
        "os.tmpname",
        "os.difftime",
        "debug.sethook",
        "debug.getinfo",
        "debug.info",
        "debug.getlocal",
        "debug.setupvalue",
        "debug.setmetatable",
        "debug.loadmodule",
        "getfenv",
        "setfenv",
        "load",
        "loadfile",
        "dofile",
        "package",
        "module",
        "_ENV",
        "string.dump",
        "vector",
        "integer",
    ] {
        assert!(vm.eval::<bool>(&format!("return {absent} == nil")).unwrap(), "{absent} is present");
    }
    for present in [
        "os.clock",
        "os.time",
        "os.date",
        "debug.traceback",
        "require",
        "loadstring",
        "print",
        "pcall",
        "xpcall",
        "error",
        "table.insert",
        "table.freeze",
        "string.format",
        "math.floor",
        "bit32.band",
        "utf8.char",
        "coroutine.wrap",
        "buffer.create",
        "collectgarbage",
    ] {
        assert!(vm.eval::<bool>(&format!("return type({present}) == 'function'")).unwrap(), "{present} is missing");
    }
    assert!(
        vm.eval::<bool>("return not pcall(collectgarbage, 'collect') and type(collectgarbage('count')) == 'number'")
            .unwrap()
    );
}

#[test]
fn bytecode_is_refused() {
    let vm = vm();
    let s = vm.run_source("l", "function load(b) return loadstring(b) end").unwrap();
    let load = vm.handler(&s, "load").unwrap().unwrap();
    let bytecode = mlua::chunk::Compiler::new().compile("return 7").unwrap();
    let (f, e): (Value, Option<String>) = vm.call(&load, mlua::BString::from(bytecode)).unwrap();
    assert!(f.is_nil(), "bytecode was loaded");
    assert!(e.unwrap().contains("binary chunk"));
    assert_eq!(vm.eval::<i32>("return loadstring('return 6')()").unwrap(), 6, "text still loads");
    let (f, e): (Value, Option<String>) = vm.eval("return loadstring('x = ')").unwrap();
    assert!(f.is_nil() && e.is_some(), "a syntax error is a value, as in Lua");
    let big = format!("return loadstring(string.rep(' ', {}))", MAX_LOADSTRING + 1);
    assert!(vm.eval::<Value>(&big).unwrap_err().to_string().contains("source over"));
}

#[test]
fn libraries_and_metatables_are_read_only() {
    let vm = vm();
    for write in [
        "string.upper = nil",
        "math.pi = 3",
        "os.clock = nil",
        "debug.traceback = nil",
        "_G.x = 1",
        "rawset(_G, 'x', 1)",
        "rawset(string, 'x', 1)",
        "table.insert(table, 1)",
        "setmetatable(_G, {})",
        "getmetatable('').__index = {}",
        "getmetatable('').__index.upper = nil",
        "getmetatable('').__add = function() end",
        "setmetatable(string, {})",
        "coroutine.wrap = nil",
        "buffer.create = nil",
    ] {
        let e = vm.eval::<()>(write).unwrap_err().to_string();
        assert!(e.contains("readonly") || e.contains("protected metatable"), "{write}: {e}");
    }
    assert!(vm.eval::<bool>("return string.upper('a') == 'A' and ('b'):upper() == 'B'").unwrap());
    assert!(vm.eval::<bool>("return getmetatable(_G) == nil and rawequal(_G, _G._G)").unwrap());
    let copy = "local s = table.clone(string) s.upper = nil return string.upper('a') == 'A'";
    assert!(vm.eval::<bool>(copy).unwrap(), "a clone is the script's own table");
}

#[test]
fn recursion_is_an_error_not_a_crash() {
    let vm = ScriptVm::new(FileMap::new(), limits(OUT_OF_REACH, 64 << 20)).unwrap();
    for deep in [
        "local function f(n) return 1 + f(n + 1) end return f(1)",
        "local t = setmetatable({}, {}) getmetatable(t).__index = function(s, k) return s[k] end return t.x",
        "local t = setmetatable({}, {}) getmetatable(t).__tostring = function(s) return tostring(s) end return tostring(t)",
        "local function g(s) return (string.gsub(s, '.', g)) end return g('ab')",
        "local f f = function() return pcall(f) end return f()",
        "local t = {} t = setmetatable({}, {__eq = function(a, b) return a == b end}) return t == setmetatable({}, getmetatable(t))",
        "local function c() return coroutine.wrap(c)() end return c()",
        "local s = string.rep('(', 100000) return loadstring('return ' .. s)",
    ] {
        match vm.eval::<Value>(deep) {
            Err(ScriptError::Script(_)) | Ok(_) => {}
            Err(e) => panic!("{deep}: {e}"),
        }
    }
    assert!(vm.stopped().is_none(), "recursion errors do not stop the VM");
    assert_eq!(vm.eval::<i32>("return 1 + 1").unwrap(), 2);
}

#[test]
fn error_objects_reach_the_host_as_capped_text() {
    let vm = vm();
    for (src, has) in [
        ("error(function() end)", "function"),
        ("error({})", "table"),
        ("error(coroutine.create(function() end))", "thread"),
        ("error(setmetatable({}, {__tostring = function() return 'custom' end}))", "custom"),
        ("error(setmetatable({}, {__tostring = function() return {} end}))", "error in error handling"),
        ("error(nil)", "nil"),
    ] {
        match vm.run_source("err", src) {
            Err(ScriptError::Script(msg)) => assert!(msg.contains(has), "{src}: {msg}"),
            other => panic!("{src}: {:?}", other.err()),
        }
    }
    let Err(ScriptError::Script(msg)) = vm.run_source("big", "error(string.rep('x', 4e6))") else { panic!() };
    assert!(msg.len() < MAX_ERROR_LEN + 64 && msg.contains("bytes cut"), "{}", msg.len());
}

#[test]
fn print_is_captured_and_bounded() {
    let vm = vm();
    vm.run_source("p", "print('a', 1, nil, true) print(setmetatable({}, {__tostring = function() return 'T' end}))")
        .unwrap();
    assert_eq!(vm.take_output(), Output { lines: vec!["a\t1\tnil\ttrue".into(), "T".into()], dropped: 0 });
    vm.run_source("flood", "for i = 1, 3000 do print(i) end print(string.rep('y', 1e5))").unwrap();
    let out = vm.take_output();
    assert_eq!((out.lines.len(), out.dropped), (MAX_OUTPUT_LINES, 2001));
    vm.run_source("long", "print(string.rep('y', 1e5))").unwrap();
    assert!(vm.take_output().lines[0].len() < MAX_OUTPUT_LINE + 64);
}

#[test]
fn os_date_and_time_take_hostile_input() {
    let vm = vm();
    for src in [
        "return os.date('%Ez')",
        "return os.date('%')",
        "return os.date(string.rep('%c', 20000))",
        "return os.date('*t', -1e300)",
        "return os.date('!%Y', 2^62)",
        "return os.time({year = 1e300, month = 1, day = 1})",
        "return os.time({year = -2^53, month = 1, day = 1})",
        "return os.time({year = 2026, month = 'x', day = 1})",
    ] {
        match vm.eval::<Value>(src) {
            Ok(_) | Err(ScriptError::Script(_)) => {}
            Err(e) => panic!("{src}: {e}"),
        }
    }
    assert!(vm.stopped().is_none());
}

#[test]
fn errors_keep_luau_semantics_and_reach_the_host_bounded() {
    let vm = vm();
    let s = |src: &str| vm.eval::<String>(src).unwrap();
    assert_eq!(s("local ok, e = pcall(function() error('boom') end) return e"), "eval:1: boom");
    assert_eq!(s("local ok, e = pcall(function() error('boom', 0) end) return e"), "boom");
    assert_eq!(
        s("local function f() error('up', 2) end local ok, e = pcall(function()\nf() end) return e"),
        "eval:2: up"
    );
    assert!(
        vm.eval::<bool>("local t = {} local ok, e = pcall(error, t) return e == t").unwrap(),
        "objects pass unchanged"
    );
    let inside =
        vm.eval::<i64>("local ok, e = pcall(function() error(string.rep('x', 100000)) end) return #e").unwrap();
    assert_eq!(inside, 100_000 + "eval:1: ".len() as i64, "inside the VM a message stays whole");
    let Err(ScriptError::Script(host)) = vm.eval::<()>("error(string.rep('x', 100000))") else { panic!() };
    assert!(host.len() < MAX_ERROR_LEN + 64 && host.contains("bytes cut"), "{}", host.len());
    let deep = "local function f(n) if n == 0 then error('deep') end return f(n - 1) + 1 end return f(15000)";
    let Err(ScriptError::Script(host)) = vm.eval::<()>(deep) else { panic!() };
    assert!(host.starts_with("eval:1: deep") && host.len() < MAX_ERROR_LEN + 64, "{}", host.len());
}

#[test]
fn script_names_are_bounded_before_they_are_copied() {
    let vm = vm();
    let name = format!("return require(string.rep('a', {}))", MAX_REQUIRE_NAME + 1);
    let e = vm.eval::<Value>(&name).unwrap_err().to_string();
    assert!(e.contains(&format!("a name of {} bytes", MAX_REQUIRE_NAME + 1)), "{e}");
    let (f, e): (Value, Option<String>) = vm.eval("return loadstring('x = ', string.rep('n', 1e6))").unwrap();
    assert!(f.is_nil() && e.unwrap().len() < 4096, "the chunk name is cut before it is copied");
    let (f, _): (Value, Value) = vm.eval("return loadstring('return 1', string.rep('\\255', 1e5))").unwrap();
    assert!(f.is_function(), "a binary chunk name is fine once cut");
}
