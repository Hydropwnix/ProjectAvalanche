use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use mlua::{Function, MultiValue, RegistryKey, Value};

use super::monitor::StopReason;
use super::world::{CallFrame, HostShared, World};
use crate::api::lua::element_value;
use crate::value::{ElementId, NetValue, to_lua};
use crate::vm::{Cut, Entered, Stamp};
use crate::{Overrun, ScriptError, ScriptVm};

pub const STACK_BUDGET: usize = 256 << 10;

#[inline(never)]
fn stack_here() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker as *const u8).addr()
}

pub enum Outcome<R> {
    Done(R),
    Failed(String),
    Stopped,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Errors {
    Note,
    Return,
}

pub fn enter<R>(
    shared: &HostShared,
    package: &str,
    during: &str,
    errors: Errors,
    f: impl FnOnce(&ScriptVm) -> Result<R, ScriptError>,
) -> Outcome<R> {
    let here = stack_here();
    let chain = &shared.chain;
    let (vm, caller, s) = {
        let mut w = shared.lock();
        if w.running(package).is_none() {
            return Outcome::Stopped;
        }
        let outermost = w.calls.is_empty();
        if !outermost && chain.cut.load(Ordering::Relaxed) {
            return Outcome::Failed(format!("{during}: refused, the host's call chain ran past its bound and unwinds"));
        }
        if !outermost && w.stack_base.abs_diff(here) > STACK_BUDGET {
            let used = w.stack_base.abs_diff(here) >> 10;
            return Outcome::Failed(format!("{during}: calls nested deeper than the host's stack allows ({used} KiB)"));
        }
        let Some(vm) = w.running(package).map(|p| p.vm.clone()) else { return Outcome::Stopped };
        let caller = w
            .calls
            .last()
            .and_then(|f| w.packages.get(&f.package))
            .map(|p| p.vm.clone())
            .filter(|c| !Arc::ptr_eq(c, &vm));
        let s = chain.stamp();
        if outermost {
            w.stack_base = here;
            chain.begin(s);
        }
        w.calls.push(CallFrame {
            package: package.to_string(),
            during: during.to_string(),
            start: s,
            child_cpu: 0,
            child_ckpt: 0,
            excluded_cpu: 0,
            excluded_ckpt: 0,
        });
        (vm, caller, s)
    };
    let paused = caller.as_ref().is_some_and(|c| c.clock().pause(s));
    let entered = vm.clock().enter(s);
    let r = f(&vm);
    let e = chain.stamp();
    let own = (entered == Entered::Began).then(|| vm.clock().own_checkpoints());
    vm.clock().leave(Some(e), entered);
    if paused && let Some(c) = &caller {
        c.clock().resume(e);
    }
    let heap = vm.used_memory();
    let cut = vm.clock().unwinding.load(Ordering::Relaxed);
    let mut w = shared.lock();
    if let Some(frame) = w.calls.pop() {
        let wall = e.wall.saturating_sub(frame.start.wall);
        let cpu = e.cpu.saturating_sub(frame.start.cpu);
        let ckpt = e.ckpt.saturating_sub(frame.start.ckpt);
        let stopped = matches!(r, Err(ScriptError::Budget(_) | ScriptError::Memory(_) | ScriptError::Stopped(_)));
        let outermost = w.calls.is_empty();
        match w.calls.last_mut() {
            Some(parent) => {
                parent.child_cpu = parent.child_cpu.saturating_add(cpu);
                parent.child_ckpt = parent.child_ckpt.saturating_add(ckpt);
                if stopped || cut {
                    parent.excluded_cpu = parent.excluded_cpu.saturating_add(cpu);
                    parent.excluded_ckpt = parent.excluded_ckpt.saturating_add(ckpt);
                }
            }
            None => end_chain(&mut w, shared, wall, cpu, ckpt),
        }
        if let Some(p) = w.packages.get_mut(package) {
            p.tick_time = p.tick_time.saturating_add(Duration::from_nanos(cpu.saturating_sub(frame.child_cpu)));
            p.tick_count = p.tick_count.saturating_add(ckpt.saturating_sub(frame.child_ckpt));
            p.heap_peak = p.heap_peak.max(heap);
            if let Some(own) = own {
                p.call_peak = p.call_peak.max(own);
            }
            if outermost || stopped || cut {
                p.tick_charge = p.tick_charge.saturating_add(ckpt.saturating_sub(frame.excluded_ckpt));
                let charge_cpu = Duration::from_nanos(cpu.saturating_sub(frame.excluded_cpu));
                p.tick_charge_cpu = p.tick_charge_cpu.saturating_add(charge_cpu);
                during.clone_into(&mut p.last_during);
            }
        }
    }
    if cut && let Err(ScriptError::Script(_)) = &r {
        drop(w);
        let text = "the host's call chain ran past its bound and was cut (another package was stopped)".to_string();
        if errors == Errors::Note {
            vm.note(format!("{during}: {text}"));
        }
        return Outcome::Failed(text);
    }
    match r {
        Ok(v) => Outcome::Done(v),
        Err(ScriptError::Budget(budget)) => {
            w.stop(package, StopReason::CallBudget { budget, during: during.to_string() });
            Outcome::Stopped
        }
        Err(ScriptError::Memory(limit)) => {
            w.stop(package, StopReason::Memory { limit, during: during.to_string() });
            Outcome::Stopped
        }
        Err(ScriptError::Stopped(_)) => Outcome::Stopped,
        Err(e) => {
            if let Some(p) = w.packages.get_mut(package) {
                p.errors += 1;
            }
            drop(w);
            let text = e.to_string();
            if errors == Errors::Note {
                vm.note(format!("error in {during}: {text}"));
            }
            Outcome::Failed(text)
        }
    }
}

fn end_chain(w: &mut World, shared: &HostShared, wall: u64, cpu: u64, ckpt: u64) {
    w.side_tick_time = w.side_tick_time.saturating_add(Duration::from_nanos(cpu));
    w.chain_peak_checkpoints = w.chain_peak_checkpoints.max(ckpt);
    w.chain_peak = w.chain_peak.max(Duration::from_nanos(cpu));
    w.chain_peak_wall = w.chain_peak_wall.max(Duration::from_nanos(wall));
    if shared.chain.end() {
        w.chain_cuts += 1;
        for p in w.packages.values() {
            p.vm.clock().set_unwinding(false);
        }
    }
}

pub(crate) fn cut_chain(shared: &HostShared, now: Stamp, unit: Cut) {
    let mut w = shared.lock();
    let mut culprit: Option<(usize, u64)> = None;
    let at = |s: &Stamp| if unit == Cut::Checkpoints { s.ckpt } else { s.cpu };
    for (i, f) in w.calls.iter().enumerate() {
        if w.running(&f.package).is_none() {
            continue;
        }
        let until = w.calls.get(i + 1).map_or(at(&now), |next| at(&next.start));
        let excluded = if unit == Cut::Checkpoints { f.excluded_ckpt } else { f.excluded_cpu };
        let share = until.saturating_sub(at(&f.start)).saturating_sub(excluded);
        if culprit.is_none_or(|(_, best)| share > best) {
            culprit = Some((i, share));
        }
    }
    let culprit = culprit.map(|(i, _)| (w.calls[i].package.clone(), w.calls[i].during.clone()));
    let unwind: Vec<String> = w.calls.iter().map(|f| f.package.clone()).collect();
    for name in unwind {
        if culprit.as_ref().is_none_or(|(c, _)| *c != name)
            && let Some(p) = w.packages.get(&name)
        {
            p.vm.clock().set_unwinding(true);
        }
    }
    if let Some((name, during)) = culprit {
        let first = w.calls.first().map_or(now, |f| f.start);
        let bound = match unit {
            Cut::Checkpoints => Overrun::Checkpoints(shared.limits.budgets.chain),
            Cut::Backstop => Overrun::Backstop(shared.limits.budgets.chain_backstop),
        };
        let checkpoints = now.ckpt.saturating_sub(first.ckpt);
        let cpu = Duration::from_nanos(now.cpu.saturating_sub(first.cpu));
        let wall = Duration::from_nanos(now.wall.saturating_sub(first.wall));
        w.stop(&name, StopReason::ChainBudget { bound, checkpoints, cpu, wall, during });
    }
}

pub struct HandlerGlobals<'a> {
    pub event: &'a str,
    pub source: ElementId,
    pub this: ElementId,
    pub client: Option<ElementId>,
}

pub fn call_function(
    vm: &ScriptVm,
    key: &RegistryKey,
    globals: Option<&HandlerGlobals>,
    args: &[NetValue],
) -> Result<(), ScriptError> {
    let lua = vm.lua();
    let (f, env, saved, argv) = vm.guarded(|| {
        let f: Function = lua.registry_value(key)?;
        let env = f.environment().filter(|t| !t.is_readonly());
        let mut saved = Vec::new();
        if let (Some(env), Some(g)) = (&env, globals) {
            let client = match g.client {
                Some(c) => element_value(lua, c)?,
                None => Value::Nil,
            };
            let set = [
                ("source", element_value(lua, g.source)?),
                ("this", element_value(lua, g.this)?),
                ("client", client),
                ("eventName", Value::String(lua.create_string(g.event)?)),
            ];
            for (k, v) in set {
                saved.push((k, env.raw_get::<Value>(k)?));
                env.raw_set(k, v)?;
            }
        }
        let argv = args.iter().map(|a| to_lua(lua, a, &element_value)).collect::<mlua::Result<MultiValue>>()?;
        Ok((f, env, saved, argv))
    })?;
    let r = vm.run::<()>(&f, argv);
    if let Some(env) = env {
        for (k, v) in saved {
            let _ = env.raw_set(k, v);
        }
    }
    r
}

pub struct Dispatch<'a> {
    pub name: &'a str,
    pub source: ElementId,
    pub args: &'a [NetValue],
    pub only: Option<&'a str>,
    pub client: Option<ElementId>,
}

pub fn dispatch(shared: &Arc<HostShared>, d: &Dispatch) -> Result<bool, String> {
    let ids = {
        let mut w = shared.lock();
        let max = shared.limits.event_depth;
        if w.frames.len() >= max {
            return Err(format!("events nested deeper than {max}"));
        }
        let ids = w.events.reached(&w.tree, d.name, d.source, d.only);
        w.frames.push(false);
        ids
    };
    for id in ids {
        let over = {
            let mut w = shared.lock();
            let over = w.game.bound.is_some_and(|b| w.calls.is_empty() && w.side_tick_time >= b);
            if over {
                w.game.skipped += 1;
            }
            over
        };
        if over {
            continue;
        }
        let found = {
            let w = shared.lock();
            w.events
                .handler(id)
                .filter(|h| w.running(&h.package).is_some())
                .map(|h| (h.package.clone(), h.callback.key.clone(), h.callback.at.clone(), h.element))
        };
        let Some((package, key, at, this)) = found else { continue };
        let globals = HandlerGlobals { event: d.name, source: d.source, this, client: d.client };
        let during = format!("the handler of {} ({at})", d.name);
        let _ = enter(shared, &package, &during, Errors::Note, |vm| call_function(vm, &key, Some(&globals), d.args));
    }
    let mut w = shared.lock();
    let cancelled = w.frames.pop().unwrap_or(false);
    w.last_cancelled = cancelled;
    Ok(cancelled)
}
