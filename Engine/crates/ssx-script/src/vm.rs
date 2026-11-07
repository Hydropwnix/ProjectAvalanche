use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mlua::chunk::ChunkMode;
use mlua::{FromLua, FromLuaMulti, Function, IntoLuaMulti, Lua, LuaOptions, RegistryKey, Table, Value, VmState};

use crate::env::{self, Output};
use crate::host::limits::Budgets;
use crate::require::{self, FileMap};
use crate::{MAX_ERROR_LEN, MAX_SOURCE, ScriptError, call, compile, cpu};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub call: u64,
    pub call_backstop: Duration,
    pub memory: usize,
}

impl Limits {
    pub const CLIENT: Limits = Limits {
        call: Budgets::CLIENT.call,
        call_backstop: Budgets::CLIENT.call_backstop,
        memory: Budgets::CLIENT.memory,
    };
    pub const SERVER: Limits = Limits {
        call: Budgets::SERVER.call,
        call_backstop: Budgets::SERVER.call_backstop,
        memory: Budgets::SERVER.memory,
    };
    pub const MIN_MEMORY: usize = 1 << 20;
    pub const MAX_MEMORY: usize = 4 << 30;
    pub const MAX_CHECKPOINTS: u64 = 1 << 32;
    pub const MAX_BUDGET: Duration = Duration::from_secs(10);

    pub(crate) fn check(self) -> Result<(), ScriptError> {
        let ok_count = (1..=Self::MAX_CHECKPOINTS).contains(&self.call);
        let ok_backstop = !self.call_backstop.is_zero() && self.call_backstop <= Self::MAX_BUDGET;
        let ok_memory = (Self::MIN_MEMORY..=Self::MAX_MEMORY).contains(&self.memory);
        if ok_count && ok_backstop && ok_memory { Ok(()) } else { Err(ScriptError::BadLimits(self)) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overrun {
    Checkpoints(u64),
    Backstop(Duration),
}

impl fmt::Display for Overrun {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Overrun::Checkpoints(n) => write!(f, "{n} checkpoints"),
            Overrun::Backstop(d) => write!(f, "backstop of {} of CPU time", time_text(*d)),
        }
    }
}

pub(crate) fn time_text(d: Duration) -> String {
    if d.as_micros().is_multiple_of(1000) { format!("{} ms", d.as_millis()) } else { format!("{} µs", d.as_micros()) }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Stamp {
    pub(crate) wall: u64,
    pub(crate) cpu: u64,
    pub(crate) ckpt: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    Checkpoints,
    Backstop,
}

pub(crate) struct Chain {
    epoch: Instant,
    pub(crate) start: AtomicU64,
    start_cpu: AtomicU64,
    count: AtomicU64,
    start_count: AtomicU64,
    slack: AtomicU64,
    pub(crate) bound: u64,
    pub(crate) backstop: u64,
    pub(crate) cut: AtomicBool,
    cutter: Cutter,
}

pub(crate) type Cutter = Box<dyn Fn(Stamp, Cut) + Send + Sync>;

impl Chain {
    pub(crate) fn new(bound: u64, backstop: Duration, cutter: Cutter) -> Arc<Self> {
        Arc::new(Chain {
            epoch: Instant::now(),
            start: AtomicU64::new(IDLE),
            start_cpu: AtomicU64::new(0),
            count: AtomicU64::new(0),
            start_count: AtomicU64::new(0),
            slack: AtomicU64::new(0),
            bound,
            backstop: u64::try_from(backstop.as_nanos()).unwrap_or(IDLE / 2),
            cut: AtomicBool::new(false),
            cutter,
        })
    }

    pub(crate) fn now(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_nanos()).unwrap_or(IDLE - 1)
    }

    pub(crate) fn stamp(&self) -> Stamp {
        Stamp { wall: self.now(), cpu: cpu::charged(), ckpt: self.count.load(Ordering::Relaxed) }
    }

    pub(crate) fn begin(&self, s: Stamp) {
        self.start_cpu.store(s.cpu, Ordering::Relaxed);
        self.start_count.store(s.ckpt, Ordering::Relaxed);
        self.slack.store(0, Ordering::Relaxed);
        self.cut.store(false, Ordering::Relaxed);
        self.start.store(s.wall, Ordering::Relaxed);
    }

    pub(crate) fn end(&self) -> bool {
        self.start.store(IDLE, Ordering::Relaxed);
        self.cut.swap(false, Ordering::Relaxed)
    }
}

pub(crate) struct Clock {
    chain: Arc<Chain>,
    checkpoints: u64,
    backstop: u64,
    own_count: AtomicU64,
    own: AtomicU64,
    resumed: AtomicU64,
    resumed_cpu: AtomicU64,
    slack: AtomicU64,
    calls: AtomicU32,
    pub(crate) expired: AtomicBool,
    over: AtomicU8,
    pub(crate) unwinding: AtomicBool,
    interrupts: AtomicU64,
}

pub(crate) const IDLE: u64 = u64::MAX;
const OVER_COUNT: u8 = 1;
const OVER_BACKSTOP: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entered {
    Began,
    Resumed,
    Running,
}

impl Clock {
    fn start_segment(&self, s: Stamp) {
        self.resumed_cpu.store(s.cpu, Ordering::Relaxed);
        self.slack.store(0, Ordering::Relaxed);
        self.resumed.store(s.wall, Ordering::Relaxed);
    }

    pub(crate) fn enter(&self, s: Stamp) -> Entered {
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            self.own.store(0, Ordering::Relaxed);
            self.own_count.store(0, Ordering::Relaxed);
            self.start_segment(s);
            Entered::Began
        } else if self.resumed.load(Ordering::Relaxed) == IDLE {
            self.start_segment(s);
            Entered::Resumed
        } else {
            Entered::Running
        }
    }

    pub(crate) fn leave(&self, e: Option<Stamp>, how: Entered) {
        match how {
            Entered::Began => self.resumed.store(IDLE, Ordering::Relaxed),
            Entered::Resumed => {
                self.pause(e.unwrap_or_else(|| self.chain.stamp()));
            }
            Entered::Running => {}
        }
        self.calls.fetch_sub(1, Ordering::Relaxed);
    }

    pub(crate) fn set_unwinding(&self, on: bool) {
        self.unwinding.store(on, Ordering::Relaxed);
    }

    pub(crate) fn pause(&self, s: Stamp) -> bool {
        let r = self.resumed.swap(IDLE, Ordering::Relaxed);
        if r == IDLE {
            return false;
        }
        self.own.fetch_add(s.cpu.saturating_sub(self.resumed_cpu.load(Ordering::Relaxed)), Ordering::Relaxed);
        true
    }

    pub(crate) fn resume(&self, s: Stamp) {
        if self.calls.load(Ordering::Relaxed) > 0 {
            self.start_segment(s);
        }
    }

    fn expire(&self, over: u8) -> &'static str {
        let _ = self.over.compare_exchange(0, over, Ordering::Relaxed, Ordering::Relaxed);
        self.expired.store(true, Ordering::Relaxed);
        BUDGET_ERROR
    }

    pub(crate) fn own_checkpoints(&self) -> u64 {
        self.own_count.load(Ordering::Relaxed)
    }

    fn check(&self) -> Result<(), &'static str> {
        if self.expired.load(Ordering::Relaxed) {
            return Err(BUDGET_ERROR);
        }
        let own_count = self.own_count.load(Ordering::Relaxed).saturating_add(1);
        self.own_count.store(own_count, Ordering::Relaxed);
        let chain = &self.chain;
        let count = chain.count.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        if own_count > self.checkpoints {
            return Err(self.expire(OVER_COUNT));
        }
        let now = chain.now();
        let mut cpu_now = None;
        let r = self.resumed.load(Ordering::Relaxed);
        if r != IDLE {
            let segment = now.saturating_sub(r);
            let own = self.own.load(Ordering::Relaxed);
            if own.saturating_add(segment.saturating_sub(self.slack.load(Ordering::Relaxed))) > self.backstop {
                let cpu = *cpu_now.get_or_insert_with(cpu::charged);
                let used = cpu.saturating_sub(self.resumed_cpu.load(Ordering::Relaxed));
                self.slack.store(segment.saturating_sub(used), Ordering::Relaxed);
                if own.saturating_add(used) > self.backstop {
                    return Err(self.expire(OVER_BACKSTOP));
                }
            }
        }
        let start = chain.start.load(Ordering::Relaxed);
        if start != IDLE
            && count.saturating_sub(chain.start_count.load(Ordering::Relaxed)) > chain.bound
            && !chain.cut.swap(true, Ordering::Relaxed)
        {
            (chain.cutter)(chain.stamp(), Cut::Checkpoints);
        }
        if start != IDLE {
            let wall = now.saturating_sub(start);
            if wall.saturating_sub(chain.slack.load(Ordering::Relaxed)) > chain.backstop {
                let cpu = *cpu_now.get_or_insert_with(cpu::charged);
                let used = cpu.saturating_sub(chain.start_cpu.load(Ordering::Relaxed));
                chain.slack.store(wall.saturating_sub(used), Ordering::Relaxed);
                if used > chain.backstop && !chain.cut.swap(true, Ordering::Relaxed) {
                    (chain.cutter)(Stamp { wall: now, cpu, ckpt: count }, Cut::Backstop);
                }
            }
        }
        if self.expired.load(Ordering::Relaxed) {
            return Err(BUDGET_ERROR);
        }
        if self.unwinding.load(Ordering::Relaxed) {
            return Err("the host's call chain ran past its bound and is unwinding");
        }
        Ok(())
    }
}

const BUDGET_ERROR: &str = "script budget exceeded";

#[derive(Debug)]
pub struct Script(pub(crate) RegistryKey);

#[derive(Debug)]
pub struct Handler(pub(crate) RegistryKey);

fn is_memory_error(e: &mlua::Error) -> bool {
    match e {
        mlua::Error::MemoryError(_) => true,
        mlua::Error::CallbackError { cause, .. } | mlua::Error::WithContext { cause, .. } => is_memory_error(cause),
        _ => false,
    }
}

pub struct ScriptVm {
    lua: Lua,
    calls: call::Calls,
    parts: Arc<env::EnvParts>,
    files: Arc<FileMap>,
    limits: Limits,
    clock: Arc<Clock>,
    output: Arc<Mutex<Output>>,
    modules: require::SharedModules,
    stopped: Mutex<Option<String>>,
}

impl ScriptVm {
    pub fn new(files: FileMap, limits: Limits) -> Result<Self, ScriptError> {
        Self::build(files, limits, |_, _| Ok(()))
    }

    pub fn build(
        files: FileMap,
        limits: Limits,
        install: impl FnOnce(&Lua, &Table) -> mlua::Result<()>,
    ) -> Result<Self, ScriptError> {
        let chain = Chain::new(u64::MAX, Duration::MAX, Box::new(|_, _| {}));
        Self::build_on(files, Arc::new(Default::default()), limits, chain, install)
    }

    pub(crate) fn build_on(
        files: FileMap,
        compiled: require::Compiled,
        limits: Limits,
        chain: Arc<Chain>,
        install: impl FnOnce(&Lua, &Table) -> mlua::Result<()>,
    ) -> Result<Self, ScriptError> {
        limits.check()?;
        cpu::check().map_err(ScriptError::Clock)?;
        if let Some((path, _)) = files.iter().find(|(_, src)| src.len() > MAX_SOURCE) {
            return Err(ScriptError::SourceTooLarge(env::cap(path, 128)));
        }
        let lua = Lua::new_with(env::libs(), LuaOptions::new())?;
        env::cut_globals(&lua)?;
        let output = Arc::new(Mutex::new(Output::default()));
        let parts = Arc::new(env::EnvParts::new(&lua, lua.globals(), output.clone())?);
        let files = Arc::new(files);
        let modules = require::Modules::shared();
        lua.globals().set("require", require::make_require(&lua, parts.clone(), files.clone(), modules.clone())?)?;
        let clock = Arc::new(Clock {
            chain,
            checkpoints: limits.call,
            backstop: u64::try_from(limits.call_backstop.as_nanos()).unwrap_or(IDLE / 2),
            own_count: AtomicU64::new(0),
            own: AtomicU64::new(0),
            resumed: AtomicU64::new(IDLE),
            resumed_cpu: AtomicU64::new(0),
            slack: AtomicU64::new(0),
            calls: AtomicU32::new(0),
            expired: false.into(),
            over: AtomicU8::new(0),
            unwinding: false.into(),
            interrupts: AtomicU64::new(0),
        });
        let c = clock.clone();
        lua.set_interrupt(move |_| {
            c.interrupts.store(c.interrupts.load(Ordering::Relaxed).wrapping_add(1), Ordering::Relaxed);
            c.check().map(|()| VmState::Continue).map_err(mlua::Error::runtime)
        });
        let calls = call::Calls::new(&lua, &lua.globals(), clock.clone())?;
        install(&lua, &lua.globals())?;
        lua.sandbox(true)?;
        lua.set_memory_limit(limits.memory)?;
        let vm = ScriptVm { lua, calls, parts, files, limits, clock, output, modules, stopped: Mutex::new(None) };
        vm.unbudgeted(|| require::preload(&vm.lua, &vm.parts, &vm.modules, &compiled))?;
        Ok(vm)
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub(crate) fn lua(&self) -> &Lua {
        &self.lua
    }

    pub fn stopped(&self) -> Option<String> {
        self.stopped.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn stop(&self, why: &str) {
        let mut s = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        if s.is_none() {
            *s = Some(env::cap(why, MAX_ERROR_LEN));
        }
        self.clock.expired.store(true, Ordering::Relaxed);
    }

    pub fn used_memory(&self) -> usize {
        self.lua.used_memory()
    }

    pub fn interrupts(&self) -> u64 {
        self.clock.interrupts.load(Ordering::Relaxed)
    }

    pub fn take_output(&self) -> Output {
        std::mem::take(&mut *self.output.lock().unwrap_or_else(|e| e.into_inner()))
    }

    pub(crate) fn note(&self, line: String) {
        let mut o = self.output.lock().unwrap_or_else(|e| e.into_inner());
        if o.lines.len() < env::MAX_OUTPUT_LINES {
            o.lines.push(env::cap(&line, env::MAX_OUTPUT_LINE));
        } else {
            o.dropped = o.dropped.saturating_add(1);
        }
    }

    pub(crate) fn clock(&self) -> &Clock {
        &self.clock
    }

    pub(crate) fn guarded<R>(&self, f: impl FnOnce() -> mlua::Result<R>) -> Result<R, ScriptError> {
        if let Some(why) = self.stopped() {
            return Err(ScriptError::Stopped(why));
        }
        let c = &self.clock;
        let own_call = (c.calls.load(Ordering::Relaxed) == 0).then(|| c.enter(c.chain.stamp()));
        let out = f();
        if let Some(how) = own_call {
            c.leave(None, how);
        }
        out.map_err(|e| self.failed(Value::Error(Box::new(e))))
    }

    pub(crate) fn unbudgeted<R>(&self, f: impl FnOnce() -> mlua::Result<R>) -> Result<R, ScriptError> {
        if let Some(why) = self.stopped() {
            return Err(ScriptError::Stopped(why));
        }
        f().map_err(|e| self.failed(Value::Error(Box::new(e))))
    }

    pub(crate) fn run<R: FromLuaMulti>(&self, f: &Function, args: impl IntoLuaMulti) -> Result<R, ScriptError> {
        self.guarded(|| self.calls.call::<R>(&self.lua, f, args))?.map_err(|v| self.failed(v))
    }

    fn failed(&self, v: Value) -> ScriptError {
        if let Some(why) = self.stopped() {
            return ScriptError::Stopped(why);
        }
        let err = if self.clock.expired.load(Ordering::Relaxed) {
            ScriptError::Budget(match self.clock.over.load(Ordering::Relaxed) {
                OVER_BACKSTOP => Overrun::Backstop(self.limits.call_backstop),
                _ => Overrun::Checkpoints(self.limits.call),
            })
        } else {
            match v {
                Value::Error(e) if is_memory_error(&e) => ScriptError::Memory(self.limits.memory),
                Value::Error(e) => return (*e).into(),
                Value::String(s) if s.as_bytes().as_ref() == call::MEMORY_ERROR => {
                    ScriptError::Memory(self.limits.memory)
                }
                Value::String(s) => return ScriptError::Script(env::lua_text(&s, MAX_ERROR_LEN)),
                other => return ScriptError::Script(format!("an error object of type {}", other.type_name())),
            }
        };
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner()) = Some(err.to_string());
        err
    }

    pub(crate) fn compile(&self, name: &str, source: &str) -> Result<(Function, Table), ScriptError> {
        if source.len() > MAX_SOURCE {
            return Err(ScriptError::SourceTooLarge(env::cap(name, 128)));
        }
        let code = compile::bytecode(name.trim_start_matches(['@', '=']), source.as_bytes())
            .map_err(|e| ScriptError::Script(e.0))?;
        self.guarded(|| {
            let env = self.parts.script_env(&self.lua)?;
            let chunk = self.lua.load(code).set_name(name.to_string()).set_mode(ChunkMode::Binary);
            Ok((chunk.set_environment(env.clone()).into_function()?, env))
        })
    }

    pub fn run_source(&self, name: &str, source: &str) -> Result<Script, ScriptError> {
        let (f, env) = self.compile_script(name, source)?;
        self.run::<()>(&f, ())?;
        self.guarded(|| self.lua.create_registry_value(env)).map(Script)
    }

    pub(crate) fn compile_script(&self, name: &str, source: &str) -> Result<(Function, Table), ScriptError> {
        self.compile(&format!("@{}", env::cap(name, 128)), source)
    }

    pub(crate) fn load_script(&self, name: &str, code: &[u8]) -> Result<(Function, Table), ScriptError> {
        self.unbudgeted(|| {
            let env = self.parts.script_env(&self.lua)?;
            let chunk = self.lua.load(code).set_name(format!("@{}", env::cap(name, 128))).set_mode(ChunkMode::Binary);
            Ok((chunk.set_environment(env.clone()).into_function()?, env))
        })
    }

    pub fn run_file(&self, path: &str) -> Result<Script, ScriptError> {
        let source = self.files.get(path).ok_or_else(|| ScriptError::NoFile(env::cap(path, 128)))?;
        self.run_source(path, source)
    }

    pub fn global<R: FromLua>(&self, script: &Script, name: &str) -> Result<R, ScriptError> {
        self.guarded(|| self.lua.registry_value::<Table>(&script.0)?.raw_get(name))
    }

    pub fn handler(&self, script: &Script, name: &str) -> Result<Option<Handler>, ScriptError> {
        match self.global::<Value>(script, name)? {
            Value::Function(f) => self.guarded(|| self.lua.create_registry_value(f)).map(|k| Some(Handler(k))),
            _ => Ok(None),
        }
    }

    pub fn text(&self, value: &Value, max: usize) -> Result<String, ScriptError> {
        match value {
            Value::String(s) => Ok(env::lua_text(s, max)),
            other => {
                let tostring: Function = self.guarded(|| self.parts.globals.get("tostring"))?;
                Ok(env::lua_text(&self.run::<mlua::LuaString>(&tostring, other.clone())?, max))
            }
        }
    }

    pub fn call<R: FromLuaMulti>(&self, handler: &Handler, args: impl IntoLuaMulti) -> Result<R, ScriptError> {
        let f = self.guarded(|| self.lua.registry_value::<Function>(&handler.0))?;
        self.run(&f, args)
    }

    pub fn eval<R: FromLuaMulti>(&self, source: &str) -> Result<R, ScriptError> {
        let (f, _) = self.compile("=eval", source)?;
        self.run(&f, ())
    }
}
