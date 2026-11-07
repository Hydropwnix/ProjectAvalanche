use std::time::Duration;

use mlua::{Function, MultiValue, Table, Value};

use super::call::{Dispatch, Errors, Outcome, call_function, dispatch, enter};
use super::monitor::StopReason;
use super::world::World;
use super::{CommandOrigin, ScriptHost, Side};
use crate::Overrun;
use crate::value::NetValue;

pub(crate) const RUNNING: Duration = Duration::MAX;

fn side_used(w: &World) -> Duration {
    w.side_tick_time
}

impl ScriptHost {
    pub fn tick(&self, now: Duration) {
        let limits = self.shared.limits;
        let budget = limits.budgets.tick;
        {
            let mut w = self.shared.lock();
            let dt = now.saturating_sub(w.now);
            w.now = w.now.max(now);
            w.tick += 1;
            for p in w.packages.values_mut() {
                p.tick_messages = 0;
                p.tick_bytes = 0;
            }
            super::server::rights::refresh(&mut w);
            let World { latent_out, outbox, .. } = &mut *w;
            latent_out.step(dt, limits.latent_chunk, limits.latent_send_total, outbox);
        }
        self.run_timers(now, budget);
        self.run_queues(budget);
        self.flush_stores(now);
        self.account();
        self.settle();
    }

    fn run_timers(&self, now: Duration, budget: Duration) {
        loop {
            let next = {
                let mut w = self.shared.lock();
                if side_used(&w) >= budget {
                    return;
                }
                let Some((&(due, id), _)) = w.timers.due.first_key_value() else { return };
                if due > now {
                    return;
                }
                let rec = w.timers.due.remove(&(due, id));
                if let Some(at) = w.timers.at.get_mut(&id) {
                    at.0 = RUNNING;
                }
                rec.map(|r| (due, r))
            };
            let Some((due, rec)) = next else { return };
            let during = format!("timer {} ({})", rec.id, rec.callback.at);
            let (key, args) = (rec.callback.key.clone(), rec.args.clone());
            let _ = enter(&self.shared, &rec.package, &during, Errors::Note, |vm| {
                let (f, argv) = vm.guarded(|| {
                    let lua = vm.lua();
                    let f: Function = lua.registry_value(&key)?;
                    let t: Table = lua.registry_value(&args)?;
                    let argv = t.sequence_values::<Value>().collect::<mlua::Result<MultiValue>>()?;
                    Ok((f, argv))
                })?;
                vm.run::<()>(&f, argv)
            });
            let mut w = self.shared.lock();
            if w.timers.at.get(&rec.id).map(|a| a.0) != Some(RUNNING) {
                continue;
            }
            if w.running(&rec.package).is_none() || rec.left == 1 {
                w.timers.at.remove(&rec.id);
                if let Some(p) = w.packages.get_mut(&rec.package) {
                    p.timers = p.timers.saturating_sub(1);
                }
                continue;
            }
            let mut next_due = due.saturating_add(rec.interval);
            if next_due <= now {
                next_due = now.saturating_add(rec.interval);
            }
            let mut rec = rec;
            if rec.left > 1 {
                rec.left -= 1;
            }
            if let Some(at) = w.timers.at.get_mut(&rec.id) {
                at.0 = next_due;
            }
            w.timers.due.insert((next_due, rec.id), rec);
        }
    }

    fn run_queues(&self, budget: Duration) {
        loop {
            let next = {
                let mut w = self.shared.lock();
                if side_used(&w) >= budget || w.order.is_empty() {
                    return;
                }
                let n = w.order.len();
                let start = w.rotation % n;
                let pick = (0..n)
                    .map(|i| w.order[(start + i) % n].clone())
                    .find(|name| w.packages.get(name).is_some_and(|p| !p.queue.is_empty()));
                let Some(name) = pick else { return };
                w.rotation = w.order.iter().position(|o| *o == name).map_or(0, |i| i + 1);
                let p = w.packages.get_mut(&name).and_then(|p| {
                    let q = p.queue.pop_front()?;
                    p.queue_bytes = p.queue_bytes.saturating_sub(q.bytes);
                    Some(q)
                });
                p.map(|q| (name, q))
            };
            let Some((name, q)) = next else { return };
            let d = Dispatch { name: &q.name, source: q.source, args: &q.args, only: Some(&name), client: q.client };
            let _ = dispatch(&self.shared, &d);
        }
    }

    fn flush_stores(&self, now: Duration) {
        let mut w = self.shared.lock();
        let every = self.shared.limits.storage.flush_every;
        let World { packages, writer, .. } = &mut *w;
        let Some(writer) = writer else { return };
        for (name, p) in packages.iter_mut() {
            if let Some(store) = p.store.as_mut()
                && store.dirty
                && now.saturating_sub(store.last_flush) >= every
                && writer.queue(name, store)
            {
                store.last_flush = now;
            }
        }
        for (name, r) in writer.results() {
            if let (Err(e), Some(p)) = (r, packages.get(&name)) {
                p.vm.note(format!("storage: the write failed: {e}"));
            }
        }
    }

    fn account(&self) {
        let mut w = self.shared.lock();
        let side_time = std::mem::take(&mut w.side_tick_time);
        w.side_window.push(side_time);
        let b = self.shared.limits.budgets;
        let mut stops = Vec::new();
        let server = self.shared.side == Side::Server;
        let world = &mut *w;
        for (name, p) in world.packages.iter_mut() {
            let t = std::mem::take(&mut p.tick_time);
            if server {
                let spent = world.server.spent.entry(name.clone()).or_default();
                *spent = spent.saturating_add(t);
            }
            p.window.push(t);
            let n = std::mem::take(&mut p.tick_count);
            p.count_window.push(n);
            p.charged = std::mem::take(&mut p.tick_charge);
            p.charged_cpu = std::mem::take(&mut p.tick_charge_cpu);
            let over = if p.charged > b.tick_checkpoints {
                Some(Overrun::Checkpoints(b.tick_checkpoints))
            } else {
                (p.charged_cpu > b.tick_backstop).then_some(Overrun::Backstop(b.tick_backstop))
            };
            p.over = if over.is_some() { p.over.saturating_add(1) } else { 0 };
            if let Some(budget) = over
                && p.over >= b.over_tick
            {
                stops.push((
                    name.clone(),
                    StopReason::TickBudget { budget, ticks: p.over, during: p.last_during.clone() },
                ));
            }
        }
        for (name, reason) in stops {
            w.stop(&name, reason);
        }
    }

    pub fn has_command(&self, name: &str) -> bool {
        !self.shared.lock().events.command(name).is_empty()
    }

    pub fn execute_command(&self, origin: CommandOrigin, line: &str) -> Result<usize, String> {
        let limits = self.shared.limits;
        if self.shared.side == Side::Client && origin != CommandOrigin::Console {
            return Err("a client runs commands from its own console only".into());
        }
        if line.len() > limits.command_line {
            return Err(format!("a command line of {} bytes (at most {})", line.len(), limits.command_line));
        }
        let mut parts = line.split_ascii_whitespace();
        let name = parts.next().ok_or("an empty command")?;
        crate::api::check_command_name(name)?;
        let args: Vec<&str> = parts.collect();
        if args.len() > limits.command_args {
            return Err(format!("{} arguments (at most {})", args.len(), limits.command_args));
        }
        let (player, handlers) = {
            let w = self.shared.lock();
            let player = match origin {
                CommandOrigin::Console => None,
                CommandOrigin::Client(c) => Some(w.player_of(c).ok_or("the client is not connected")?),
            };
            let hs: Vec<_> = w
                .events
                .command(name)
                .iter()
                .map(|c| (c.package.clone(), c.callback.key.clone(), c.callback.at.clone()))
                .collect();
            (player, hs)
        };
        if handlers.is_empty() {
            return Err(format!("no handler for the command {name}"));
        }
        let mut values = Vec::new();
        if self.shared.side == Side::Server {
            values.push(player.map_or(NetValue::Nil, NetValue::Element));
        }
        values.push(NetValue::String(name.as_bytes().to_vec()));
        values.extend(args.iter().map(|a| NetValue::String(a.as_bytes().to_vec())));
        let mut ran = 0;
        for (package, key, at) in handlers {
            let during = format!("the command handler of {name} ({at})");
            let call = |vm: &crate::ScriptVm| call_function(vm, &key, None, &values);
            if let Outcome::Done(()) = enter(&self.shared, &package, &during, Errors::Note, call) {
                ran += 1;
            }
        }
        self.settle();
        Ok(ran)
    }
}
