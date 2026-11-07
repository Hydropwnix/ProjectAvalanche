use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use mlua::RegistryKey;

use super::Side;
use super::elements::{ClientId, Tree};
use super::events::{Callback, Events};
use super::latent::{LatentIn, LatentOut};
use super::limits::HostLimits;
use super::monitor::{STOPS_KEPT, StopReason, StopRecord, Window};
use super::outbox::{Outbox, Peer};
use super::storage::{Store, Writer};
use crate::ScriptVm;
use crate::env::Output;
use crate::value::{ElementId, NetValue};
use crate::vm::{Chain, Cutter, Stamp};

pub const LAST_OUTPUT_KEPT: usize = 16;
pub const LAST_OUTPUT_LINES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PkgState {
    Starting,
    Running,
    Stopped,
}

pub struct Queued {
    pub name: String,
    pub source: ElementId,
    pub args: Vec<NetValue>,
    pub client: Option<ElementId>,
    pub bytes: usize,
}

pub struct Pkg {
    pub vm: Arc<ScriptVm>,
    pub state: PkgState,
    pub ran: bool,
    pub client_scripts: bool,
    pub acknowledged: std::collections::BTreeSet<super::elements::ClientId>,
    pub root: ElementId,
    pub settings: NetValue,
    pub elements: usize,
    pub data_keys: usize,
    pub data_bytes: usize,
    pub timers: usize,
    pub exports: BTreeMap<String, Callback>,
    pub queue: VecDeque<Queued>,
    pub queue_bytes: usize,
    pub store: Option<Store>,
    pub window: Window<Duration>,
    pub tick_time: Duration,
    pub count_window: Window<u64>,
    pub tick_count: u64,
    pub tick_charge: u64,
    pub tick_charge_cpu: Duration,
    pub charged: u64,
    pub charged_cpu: Duration,
    pub over: u32,
    pub call_peak: u64,
    pub heap_peak: usize,
    pub errors: u64,
    pub refusals: u64,
    pub tick_messages: usize,
    pub tick_bytes: usize,
    pub last_during: String,
    pub calls: Option<super::server::rights::CallRights>,
}

pub struct TimerRec {
    pub id: u64,
    pub package: String,
    pub callback: Callback,
    pub args: Arc<RegistryKey>,
    pub interval: Duration,
    pub left: u32,
}

pub struct CallFrame {
    pub package: String,
    pub during: String,
    pub(crate) start: Stamp,
    pub child_cpu: u64,
    pub child_ckpt: u64,
    pub excluded_cpu: u64,
    pub excluded_ckpt: u64,
}

#[derive(Default)]
pub struct Timers {
    pub due: BTreeMap<(Duration, u64), TimerRec>,
    pub at: HashMap<u64, (Duration, String)>,
    pub next: u64,
}

pub struct World {
    pub tick: u64,
    pub now: Duration,
    pub tree: Tree,
    pub events: Events,
    pub packages: BTreeMap<String, Pkg>,
    pub order: Vec<String>,
    pub timers: Timers,
    pub outbox: Outbox,
    pub latent_out: LatentOut,
    pub latent_in: LatentIn,
    pub clients: BTreeMap<ClientId, ElementId>,
    pub client_data: HashMap<ClientId, (usize, usize)>,
    pub mirror: (usize, usize, usize),
    pub local_player: Option<ElementId>,
    pub stops: VecDeque<StopRecord>,
    pub pending_stops: Vec<(String, StopReason)>,
    pub last_output: VecDeque<(String, Output)>,
    pub frames: Vec<bool>,
    pub last_cancelled: bool,
    pub export_depth: usize,
    pub calls: Vec<CallFrame>,
    pub stack_base: usize,
    pub chain_peak: Duration,
    pub chain_peak_wall: Duration,
    pub chain_peak_checkpoints: u64,
    pub chain_cuts: u64,
    pub refused_inbound: u64,
    pub side_window: Window<Duration>,
    pub side_tick_time: Duration,
    pub rotation: usize,
    pub writer: Option<Writer>,
    pub game: super::game::GameState,
    pub server: super::server::ServerState,
}

pub struct HostShared {
    pub side: Side,
    pub limits: HostLimits,
    pub(crate) chain: Arc<Chain>,
    world: Mutex<World>,
}

impl HostShared {
    pub(crate) fn new(side: Side, limits: HostLimits, writer: Option<Writer>, cutter: Cutter) -> Self {
        let world = World {
            tick: 0,
            now: Duration::ZERO,
            tree: Tree::default(),
            events: Events::default(),
            packages: BTreeMap::new(),
            order: Vec::new(),
            timers: Timers::default(),
            outbox: Outbox::new(limits.outbox_messages, limits.outbox_bytes),
            latent_out: LatentOut::default(),
            latent_in: LatentIn::default(),
            clients: BTreeMap::new(),
            client_data: HashMap::new(),
            mirror: (0, 0, 0),
            local_player: None,
            stops: VecDeque::new(),
            pending_stops: Vec::new(),
            last_output: VecDeque::new(),
            frames: Vec::new(),
            last_cancelled: false,
            export_depth: 0,
            calls: Vec::new(),
            stack_base: 0,
            chain_peak: Duration::ZERO,
            chain_peak_wall: Duration::ZERO,
            chain_peak_checkpoints: 0,
            chain_cuts: 0,
            refused_inbound: 0,
            side_window: Window::default(),
            side_tick_time: Duration::ZERO,
            rotation: 0,
            writer,
            game: Default::default(),
            server: Default::default(),
        };
        let chain = Chain::new(limits.budgets.chain, limits.budgets.chain_backstop, cutter);
        HostShared { side, limits, chain, world: Mutex::new(world) }
    }

    pub fn lock(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl World {
    pub fn running(&self, name: &str) -> Option<&Pkg> {
        self.packages.get(name).filter(|p| p.state != PkgState::Stopped)
    }

    pub fn stop(&mut self, package: &str, reason: StopReason) {
        let Some(p) = self.packages.get_mut(package) else { return };
        if p.state == PkgState::Stopped {
            return;
        }
        p.state = PkgState::Stopped;
        p.vm.stop(&reason.to_string());
        self.pending_stops.push((package.to_string(), reason));
    }

    pub fn keep_last_output(&mut self, package: &str, mut out: Output) {
        if out.lines.is_empty() && out.dropped == 0 {
            return;
        }
        if out.lines.len() > LAST_OUTPUT_LINES {
            let cut = out.lines.len() - LAST_OUTPUT_LINES;
            out.lines.drain(..cut);
            out.dropped = out.dropped.saturating_add(cut);
        }
        if self.last_output.len() == LAST_OUTPUT_KEPT {
            self.last_output.pop_front();
        }
        self.last_output.push_back((package.to_string(), out));
    }

    pub fn record_stop(&mut self, package: &str, reason: StopReason) {
        if self.stops.len() == STOPS_KEPT {
            self.stops.pop_front();
        }
        self.stops.push_back(StopRecord { package: package.to_string(), tick: self.tick, reason });
    }

    pub fn player_of(&self, client: ClientId) -> Option<ElementId> {
        self.clients.get(&client).copied()
    }

    pub fn client_of_player(&self, element: ElementId) -> Option<ClientId> {
        self.clients.iter().find(|(_, e)| **e == element).map(|(c, _)| *c)
    }

    pub fn may_source(&self, peer: Peer, element: ElementId) -> bool {
        match peer {
            Peer::Server => self.tree.get(element).is_some(),
            Peer::Client(c) => {
                self.player_of(c) == Some(element) || self.tree.get(element).is_some_and(|e| e.owner == Some(c))
            }
        }
    }
}
