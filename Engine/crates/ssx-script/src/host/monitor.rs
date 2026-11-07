use std::collections::VecDeque;
use std::fmt::{self, Write as _};
use std::time::Duration;

use super::limits::Budgets;
use crate::Overrun;
use ssx_core::text::{Text, escape};

pub const WINDOW: usize = 120;
pub const STOPS_KEPT: usize = 64;

pub trait Tally: Copy + Default + Ord {
    fn plus(self, other: Self) -> Self;
    fn mean(sum: Self, n: u32) -> Self;
}

impl Tally for Duration {
    fn plus(self, other: Self) -> Self {
        self.saturating_add(other)
    }
    fn mean(sum: Self, n: u32) -> Self {
        sum / n.max(1)
    }
}

impl Tally for u64 {
    fn plus(self, other: Self) -> Self {
        self.saturating_add(other)
    }
    fn mean(sum: Self, n: u32) -> Self {
        sum / u64::from(n.max(1))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats<T> {
    pub last: T,
    pub mean: T,
    pub peak: T,
    pub total: T,
}

pub type TimeStats = Stats<Duration>;
pub type CountStats = Stats<u64>;

#[derive(Default)]
pub struct Window<T> {
    ticks: VecDeque<T>,
    total: T,
}

impl<T: Tally> Window<T> {
    pub fn push(&mut self, t: T) {
        if self.ticks.len() == WINDOW {
            self.ticks.pop_front();
        }
        self.ticks.push_back(t);
        self.total = self.total.plus(t);
    }

    pub fn stats(&self) -> Stats<T> {
        let n = u32::try_from(self.ticks.len()).unwrap_or(u32::MAX);
        Stats {
            last: self.ticks.back().copied().unwrap_or_default(),
            mean: T::mean(self.ticks.iter().fold(T::default(), |a, b| a.plus(*b)), n),
            peak: self.ticks.iter().max().copied().unwrap_or_default(),
            total: self.total,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub elements: usize,
    pub data_keys: usize,
    pub data_bytes: usize,
    pub handlers: usize,
    pub events: usize,
    pub timers: usize,
    pub commands: usize,
    pub exports: usize,
    pub latent: usize,
    pub queued: usize,
    pub outbox: usize,
    pub storage_keys: usize,
    pub storage_bytes: usize,
    pub markers: usize,
    pub binds: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageMonitor {
    pub name: String,
    pub checkpoints: CountStats,
    pub time: TimeStats,
    pub call_peak: u64,
    pub charged: u64,
    pub charged_cpu: Duration,
    pub heap: usize,
    pub heap_peak: usize,
    pub counts: Counts,
    pub errors: u64,
    pub refusals: u64,
    pub over_budget: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    CallBudget { budget: Overrun, during: String },
    ChainBudget { bound: Overrun, checkpoints: u64, cpu: Duration, wall: Duration, during: String },
    TickBudget { budget: Overrun, ticks: u32, during: String },
    Memory { limit: usize, during: String },
    Start(String),
    Requested(String),
}

fn bound_text(what: &str, o: Overrun) -> String {
    match o {
        Overrun::Checkpoints(n) => format!("{what} of {n} checkpoints"),
        Overrun::Backstop(d) => format!("{what}'s backstop of {} of CPU time", crate::vm::time_text(d)),
    }
}

impl fmt::Display for StopReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StopReason::CallBudget { budget, during } => {
                write!(f, "ran past {} in {during}", bound_text("its call budget", *budget))
            }
            StopReason::ChainBudget { bound, checkpoints, cpu, wall, during } => write!(
                f,
                "held the host's call past {} (the outermost call at {checkpoints} checkpoints, {:.1} ms of CPU \
                 time, {:.1} ms of wall time) in {during}",
                bound_text("its chain bound", *bound),
                cpu.as_secs_f64() * 1e3,
                wall.as_secs_f64() * 1e3
            ),
            StopReason::TickBudget { budget, ticks, during } => write!(
                f,
                "its calls used more than {} in {ticks} ticks in a row (last in {during})",
                bound_text("the tick budget", *budget)
            ),
            StopReason::Memory { limit, during } => {
                write!(f, "went over its memory limit of {} MiB in {during}", limit >> 20)
            }
            StopReason::Start(why) => write!(f, "did not start: {why}"),
            StopReason::Requested(why) => write!(f, "stopped by the host: {why}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopRecord {
    pub package: String,
    pub tick: u64,
    pub reason: StopReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub server: bool,
    pub tick: u64,
    pub budgets: Budgets,
    pub side_time: TimeStats,
    pub packages: Vec<PackageMonitor>,
    pub stops: Vec<StopRecord>,
    pub outbox_messages: usize,
    pub outbox_bytes: usize,
    pub latent_receiving: usize,
    pub refused_inbound: u64,
    pub chain_peak_checkpoints: u64,
    pub chain_peak: Duration,
    pub chain_peak_wall: Duration,
    pub chain_cuts: u64,
    pub lifecycle: super::server::lifecycle::LifecycleCounts,
    pub rights: super::server::rights::RightsCounts,
}

fn ms(d: Duration) -> String {
    format!("{:.3}", d.as_secs_f64() * 1e3)
}

impl Monitor {
    pub fn render(&self) -> String {
        let mut out = String::new();
        let side = if self.server { "server" } else { "client" };
        let b = &self.budgets;
        let _ = writeln!(
            out,
            "[{side} budgets] call {} ckpt (backstop {} ms CPU) | tick {} ckpt (backstop {} ms CPU) x{} in a row \
             | chain {} ckpt (backstop {} ms CPU) | queued work waits past {} ms CPU a tick",
            b.call,
            ms(b.call_backstop),
            b.tick_checkpoints,
            ms(b.tick_backstop),
            b.over_tick,
            b.chain,
            ms(b.chain_backstop),
            ms(b.tick)
        );
        let _ = writeln!(
            out,
            "[{side} tick {}] scripts {} ms CPU (mean {}, peak {}) | longest call {} ckpt, {} ms CPU / {} ms wall \
             (cut {}) | outbox {} / {} B, receiving {}, refused {}",
            self.tick,
            ms(self.side_time.last),
            ms(self.side_time.mean),
            ms(self.side_time.peak),
            self.chain_peak_checkpoints,
            ms(self.chain_peak),
            ms(self.chain_peak_wall),
            self.chain_cuts,
            self.outbox_messages,
            self.outbox_bytes,
            self.latent_receiving,
            self.refused_inbound
        );
        if self.server {
            let l = &self.lifecycle;
            let _ = writeln!(
                out,
                "[server lifecycle] starts cancelled {} | cancels refused (no right) {} | acknowledgements {} \
                 (refused {})",
                l.starts_cancelled, l.cancels_refused, l.acknowledgements, l.acknowledgements_refused
            );
            let r = &self.rights;
            let _ = writeln!(
                out,
                "[server rights] privileged calls checked {} | refused {} | ordered {} | other calls refused by the ACL \
                 {} | packages' rights resolved {}",
                r.checks, r.refused, r.ordered, r.calls_refused, r.resolved
            );
        }
        for p in &self.packages {
            let c = &p.counts;
            let _ = writeln!(
                out,
                "  {:<24} {:>7} ckpt mean {:>7} peak {:>7} call peak {:>7} charged {:>7} | {:>8} ms CPU mean {:>8} \
                 peak {:>8} charged {:>8} | heap {:>6} KiB (peak {}) | el {} data {}/{} B h {} ev {} tm {} cmd {} \
                 ex {} lat {} q {} out {} st {}/{} B mk {} keys {} | err {} ref {} over {}",
                escape(&p.name),
                p.checkpoints.last,
                p.checkpoints.mean,
                p.checkpoints.peak,
                p.call_peak,
                p.charged,
                ms(p.time.last),
                ms(p.time.mean),
                ms(p.time.peak),
                ms(p.charged_cpu),
                p.heap >> 10,
                p.heap_peak >> 10,
                c.elements,
                c.data_keys,
                c.data_bytes,
                c.handlers,
                c.events,
                c.timers,
                c.commands,
                c.exports,
                c.latent,
                c.queued,
                c.outbox,
                c.storage_keys,
                c.storage_bytes,
                c.markers,
                c.binds,
                p.errors,
                p.refusals,
                p.over_budget
            );
        }
        for s in &self.stops {
            let _ = writeln!(out, "  stopped {} at tick {}: {}", escape(&s.package), s.tick, Text::new(&s.reason));
        }
        out
    }
}
