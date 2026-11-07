mod cases;
pub mod chat;
pub mod craft;
pub mod hostile;
pub mod instance;
pub mod look;
pub mod ping;
pub mod round;
pub mod scripts;
pub mod stream;

use core::time::Duration;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Instant;

use bevy_app::{App, PostUpdate, PreUpdate, TaskPoolPlugin};
use bevy_ecs::prelude::*;
use bevy_state::app::StatesPlugin;
use bevy_time::TimePlugin;
use lightyear::connection::ConnectionSystems;
use lightyear::crossbeam::CrossbeamIo;
use lightyear::link::{LinkSystems, RecvLinkConditioner};
use lightyear::netcode::prelude::client::{NetcodeClient, NetcodeConfig};
use lightyear::prelude::server::{ClientOf, ServerPlugins, Start};
use lightyear::prelude::{
    AppMessageExt, Authentication, ChannelRegistry, Client, Connect, Connected, Link, LinkConditionerConfig, LinkOf,
    Linked, LocalAddr, LocalTimelineSync, MessageReceiver, MessageSender, NetworkDirection, PeerAddr, PeerId, RemoteId,
    ReplicationReceiver, ReplicationSender, RoomPlugin, Transport,
};
use lightyear::transport::plugin::TransportSystems;
use ssx_net_proto::netcode_protocol_id;

use ssx_net_proto::keys::{PlayerKey, PlayerSecret};

use crate::client::{NetClientPlugin, connect_token, spawn_udp_client};
use crate::protocol::ProtocolPlugin;
use crate::server::{NetServerPlugin, Sessions, TicketRequest, netcode_server, spawn_udp_server};
use crate::tick::tick_duration;

pub const HANG: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKind {
    Guarded,
    Bare,
    Mismatched,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ForeignMessage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Io {
    Crossbeam,
    Udp,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    pub latency: Duration,
    pub jitter: Duration,
    pub loss: f32,
}

impl Conditions {
    pub fn conditioner(&self) -> Result<RecvLinkConditioner, String> {
        if !self.jitter.subsec_nanos().is_multiple_of(1_000_000) {
            return Err(format!("jitter {:?}: zero or whole milliseconds", self.jitter));
        }
        if !(0.0..=1.0).contains(&self.loss) {
            return Err(format!("loss {}: a probability", self.loss));
        }
        Ok(RecvLinkConditioner::new(
            LinkConditionerConfig::default()
                .with_incoming_latency(self.latency)
                .with_incoming_jitter(self.jitter)
                .with_fixed_loss(self.loss),
        ))
    }
}

#[derive(Component, Clone, Debug, Default)]
pub struct WireMeter {
    pub sent_bytes: u64,
    pub sent_datagrams: u64,
    pub recv_bytes: u64,
    pub recv_datagrams: u64,
    pub sent_sizes: Vec<u16>,
    pub log_sizes: bool,
}

pub const UDP_IPV4_OVERHEAD: u64 = 28;

fn meter_sent(mut links: Query<(&mut Link, &mut WireMeter)>) {
    for (mut link, mut meter) in &mut links {
        for _ in 0..link.send.len() {
            let Some(payload) = link.send.pop() else { break };
            meter.sent_bytes += payload.len() as u64;
            meter.sent_datagrams += 1;
            if meter.log_sizes {
                meter.sent_sizes.push(u16::try_from(payload.len()).unwrap_or(u16::MAX));
            }
            link.send.push(payload);
        }
    }
}

fn meter_received(mut links: Query<(&mut Link, &mut WireMeter)>) {
    for (mut link, mut meter) in &mut links {
        for _ in 0..link.recv.len() {
            let Some(payload) = link.recv.pop() else { break };
            meter.recv_bytes += payload.len() as u64;
            meter.recv_datagrams += 1;
            link.recv.push_raw(payload);
        }
    }
}

fn add_meter(trigger: On<Add<Link>>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(WireMeter::default());
}

#[derive(Resource, Default)]
pub struct Injector {
    pub on_channel: Vec<(u16, Vec<u8>)>,
    pub raw: Vec<Vec<u8>>,
    pub hijack_pings: bool,
    pub hijacked: u32,
}

type PingLink<'a> = (
    &'a mut MessageReceiver<lightyear_sync::ping::message::Ping>,
    &'a mut MessageSender<lightyear_sync::ping::message::Pong>,
);
type ConnectedClient = (With<Client>, With<Connected>);

fn hijack_pings(mut injector: ResMut<Injector>, mut links: Query<PingLink, ConnectedClient>) {
    use lightyear::core::time::PositiveTickDelta;
    use lightyear_sync::ping::message::Pong;
    if !injector.hijack_pings {
        return;
    }
    for (mut pings, mut pongs) in &mut links {
        let ids: Vec<_> = pings.receive().map(|ping| ping.id).collect();
        for ping_id in ids {
            let pong = Pong { ping_id, frame_time: PositiveTickDelta::lit("2147483648") };
            pongs.send::<lightyear_sync::ping::PingChannel>(pong);
            injector.hijacked += 1;
        }
    }
}

fn inject_on_channels(
    mut injector: ResMut<Injector>,
    registry: Res<ChannelRegistry>,
    links: Query<&Transport, (With<Client>, With<Connected>)>,
) {
    let Ok(transport) = links.single() else { return };
    for (net_id, bytes) in injector.on_channel.drain(..) {
        if let Some(kind) = registry.get_kind_from_net_id(net_id) {
            transport.send_erased(*kind, bytes.into(), 1.0).ok();
        }
    }
}

fn inject_raw(mut injector: ResMut<Injector>, mut links: Query<&mut Link, (With<Client>, With<Connected>)>) {
    let Ok(mut link) = links.single_mut() else { return };
    for bytes in injector.raw.drain(..) {
        link.send.push(bytes.into());
    }
}

fn bare_link_setup(trigger: On<Add<Connected>>, links: Query<(), With<ClientOf>>, mut commands: Commands) {
    if links.contains(trigger.entity) {
        commands.entity(trigger.entity).insert(ReplicationSender);
    }
}

fn base_app() -> App {
    let mut app = App::new();
    app.add_plugins((TaskPoolPlugin::default(), TimePlugin, StatesPlugin));
    app.add_observer(add_meter);
    app.add_systems(PostUpdate, meter_sent.after(ConnectionSystems::Send).before(LinkSystems::Send));
    app.add_systems(PreUpdate, meter_received.after(LinkSystems::Receive).before(ConnectionSystems::Receive));
    app
}

pub fn client_app() -> App {
    let mut app = base_app();
    app.add_plugins(NetClientPlugin);
    app.finish();
    app.cleanup();
    app
}

pub struct Peers {
    pub server: App,
    pub server_entity: Entity,
    pub server_addr: SocketAddr,
    pub clients: Vec<App>,
    pub client_entities: Vec<Entity>,
    pub client_ids: Vec<u64>,
    pub players: Vec<Option<PlayerKey>>,
    pub frame: Duration,
    io: Io,
    conditioner: Option<RecvLinkConditioner>,
    key: [u8; 32],
    next: Instant,
}

impl Peers {
    pub fn new(io: Io, conditions: Option<Conditions>, frame: Duration) -> Result<Self, String> {
        Self::with_server(ServerKind::Guarded, io, conditions, frame)
    }

    pub fn with_server(
        kind: ServerKind,
        io: Io,
        conditions: Option<Conditions>,
        frame: Duration,
    ) -> Result<Self, String> {
        Self::with_instances(kind, io, conditions, frame, test_instance())
    }

    pub fn with_instances(
        kind: ServerKind,
        io: Io,
        conditions: Option<Conditions>,
        frame: Duration,
        instances: crate::server::InstanceConfig,
    ) -> Result<Self, String> {
        Self::with_server_setup(kind, io, conditions, frame, instances, |_| {})
    }

    pub fn with_server_setup(
        kind: ServerKind,
        io: Io,
        conditions: Option<Conditions>,
        frame: Duration,
        instances: crate::server::InstanceConfig,
        setup: impl FnOnce(&mut App),
    ) -> Result<Self, String> {
        let conditioner = conditions.map(|c| c.conditioner()).transpose()?;
        let key = lightyear::netcode::try_generate_key().map_err(|e| format!("no private key: {e}"))?;
        let mut server = base_app();
        match kind {
            ServerKind::Guarded => {
                server.add_plugins(NetServerPlugin);
                server.insert_resource(instances);
            }
            ServerKind::Mismatched => {
                server.add_plugins(NetServerPlugin);
                server.insert_resource(instances);
                server.register_message::<ForeignMessage>().add_direction(NetworkDirection::ServerToClient);
            }
            ServerKind::Bare => {
                server.add_plugins(ServerPlugins { tick_duration: tick_duration() });
                server.add_plugins((RoomPlugin, ProtocolPlugin));
                server.add_observer(bare_link_setup);
            }
        }
        setup(&mut server);
        if let (Some(c), Some(mut time)) =
            (conditions, server.world_mut().get_resource_mut::<crate::server::ServerTime>())
        {
            time.set_receive_hold(c.latency + c.jitter);
        }
        server.finish();
        server.cleanup();
        let world = server.world_mut();
        let server_entity = match io {
            Io::Udp => spawn_udp_server(world, SocketAddr::from((Ipv4Addr::LOCALHOST, 0)), key, conditioner.clone()),
            Io::Crossbeam => {
                world.get_resource_or_init::<Sessions>().set_netcode_key(key);
                world.spawn(netcode_server(key, false)).id()
            }
        };
        world.trigger(Start { entity: server_entity });
        server.update();
        let server_addr = match io {
            Io::Udp => {
                server.world().get::<LocalAddr>(server_entity).map(|a| a.0).ok_or("the server has no address")?
            }
            Io::Crossbeam => SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
        };
        Ok(Self {
            server,
            server_entity,
            server_addr,
            clients: Vec::new(),
            client_entities: Vec::new(),
            client_ids: Vec::new(),
            players: Vec::new(),
            frame,
            io,
            conditioner,
            key,
            next: Instant::now(),
        })
    }

    pub fn auth(&self, client_id: u64) -> Authentication {
        Authentication::Manual {
            server_addr: self.server_addr,
            client_id,
            private_key: self.key,
            protocol_id: netcode_protocol_id(),
        }
    }

    pub fn issue(&mut self, player: PlayerKey, name: &str) -> Result<(Authentication, u64), String> {
        self.issue_from(player, name, Ipv4Addr::LOCALHOST.into())
    }

    pub fn issue_from(
        &mut self,
        player: PlayerKey,
        name: &str,
        address: std::net::IpAddr,
    ) -> Result<(Authentication, u64), String> {
        let world = self.server.world_mut();
        let now = world.resource::<bevy_time::Time<bevy_time::Real>>().elapsed();
        let bound = world.get::<LocalAddr>(self.server_entity).map_or(self.server_addr, |a| a.0);
        let request =
            TicketRequest { player, name: name.into(), address, public_addr: self.server_addr, manifest: None };
        let issued =
            world.resource_mut::<Sessions>().issue(now, bound, &request).map_err(|e| format!("no ticket: {e:?}"))?;
        Ok((connect_token(&issued.token, self.server_addr)?, issued.client_id))
    }

    pub fn add_client(&mut self, _tag: u64) -> Result<usize, String> {
        self.add_player(PlayerSecret::generate().public(), "Rider")
    }

    pub fn add_player(&mut self, player: PlayerKey, name: &str) -> Result<usize, String> {
        let (auth, client_id) = self.issue(player, name)?;
        let c = self.add_client_with(client_id, auth)?;
        self.players[c] = Some(player);
        Ok(c)
    }

    pub fn add_client_with(&mut self, client_id: u64, auth: Authentication) -> Result<usize, String> {
        let mut app = base_app();
        app.add_plugins(NetClientPlugin).init_resource::<Injector>();
        app.add_systems(PostUpdate, inject_on_channels.before(TransportSystems::Send));
        app.add_systems(PostUpdate, inject_raw.after(TransportSystems::Send).before(ConnectionSystems::Send));
        app.add_systems(
            PreUpdate,
            hijack_pings
                .after(lightyear::prelude::MessageSystems::Receive)
                .before(lightyear_sync::ping::plugin::PingSystems::Receive),
        );
        app.finish();
        app.cleanup();
        let entity = self.link_client(&mut app, auth, self.clients.len())?;
        self.clients.push(app);
        self.client_entities.push(entity);
        self.client_ids.push(client_id);
        self.players.push(None);
        Ok(self.clients.len() - 1)
    }

    fn link_client(&mut self, app: &mut App, auth: Authentication, n: usize) -> Result<Entity, String> {
        let link = Link::default().with_conditioner(self.conditioner.clone());
        let entity = match self.io {
            Io::Udp => {
                let local = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
                spawn_udp_client(app.world_mut(), local, self.server_addr, auth, link).map_err(|e| e.to_string())?
            }
            Io::Crossbeam => {
                let (client_io, server_io) = CrossbeamIo::new_pair();
                let netcode = NetcodeClient::new(auth, NetcodeConfig::default()).map_err(|e| e.to_string())?;
                let peer = SocketAddr::from((Ipv4Addr::LOCALHOST, 1024 + (n as u16 % 60_000)));
                self.server.world_mut().spawn((
                    LinkOf { endpoint: self.server_entity },
                    Link::default().with_conditioner(self.conditioner.clone()),
                    PeerAddr(peer),
                    Linked,
                    server_io,
                ));
                app.world_mut().spawn((Client, link, ReplicationReceiver, netcode, client_io)).id()
            }
        };
        app.world_mut().trigger(Connect { entity });
        Ok(entity)
    }

    pub fn rejoin(&mut self, client: usize, player: PlayerKey, name: &str, reset: bool) -> Result<(), String> {
        if reset {
            crate::client::reset_session(self.clients[client].world_mut());
        }
        let (auth, client_id) = self.issue(player, name)?;
        let n = self.clients.len() + 100 + client;
        let mut app = std::mem::replace(&mut self.clients[client], App::empty());
        let linked = self.link_client(&mut app, auth, n);
        self.clients[client] = app;
        self.client_entities[client] = linked?;
        self.client_ids[client] = client_id;
        self.players[client] = Some(player);
        Ok(())
    }

    pub fn step(&mut self) {
        let now = Instant::now();
        if self.next > now {
            std::thread::sleep(self.next - now);
        }
        self.next = self.next.max(now) + self.frame;
        for client in &mut self.clients {
            client.update();
        }
        self.server.update();
    }

    pub fn stall_server(&mut self, stall: Duration, mut each: impl FnMut(&mut Self)) {
        let end = Instant::now() + stall;
        while Instant::now() < end {
            let now = Instant::now();
            if self.next > now {
                std::thread::sleep(self.next - now);
            }
            self.next = self.next.max(now) + self.frame;
            each(self);
            for client in &mut self.clients {
                client.update();
            }
        }
    }

    pub fn steps_for(&mut self, within: Duration, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        let end = Instant::now() + HANG;
        for _ in 0..self.steps_in(within) {
            if Instant::now() >= end {
                return false;
            }
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    pub fn send_pose(&mut self, client: usize, pose: &ssx_net_proto::NetPose, within: Duration) -> bool {
        let sent = |p: &Self| p.clients[client].world().resource::<crate::client::PoseOutbox>().sent;
        let before = sent(self);
        let end = Instant::now() + HANG;
        for _ in 0..self.steps_in(within) {
            if Instant::now() >= end {
                return false;
            }
            self.clients[client].world_mut().resource_mut::<crate::client::PoseOutbox>().next = Some(pose.clone());
            self.step();
            if sent(self) > before {
                return true;
            }
        }
        false
    }

    fn steps_in(&self, within: Duration) -> u32 {
        (within.as_secs_f64() / self.frame.as_secs_f64()).ceil() as u32
    }

    pub fn run_until(&mut self, timeout: Duration, mut done: impl FnMut(&mut Self) -> bool) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            self.step();
            if done(self) {
                return true;
            }
        }
        false
    }

    pub fn knows(&self, client: usize, n: usize) -> bool {
        let roster = self.clients[client].world().resource::<crate::client::ClientRoster>();
        roster.own.is_some() && roster.players.len() >= n
    }

    pub fn dress(&mut self, client: usize, character: u32) -> Result<(), String> {
        let look = ssx_net_proto::Appearance::new(character, 0).map_err(|e| e.to_string())?;
        let mut outbox = self.clients[client].world_mut().resource_mut::<crate::client::ControlOutbox>();
        outbox.0.extend([
            ssx_net_proto::ClientControl::Appearance(look),
            ssx_net_proto::ClientControl::Loaded { round: 0 },
            ready(0),
        ]);
        Ok(())
    }

    pub fn riding(&self, client: usize) -> bool {
        let roster = self.clients[client].world().resource::<crate::client::ClientRoster>();
        let Some((rider, _)) = roster.own else { return false };
        let Some(instances) = self.server.world().get_resource::<crate::server::Instances>() else { return false };
        let progress = instances.of_rider(rider.0).and_then(|i| i.instance.progress(rider.0));
        matches!(progress, Some(ssx_gameplay::instance::Progress::Ready(_)))
    }

    pub fn connected(&self, client: usize) -> bool {
        self.clients[client].world().get::<Connected>(self.client_entities[client]).is_some()
    }

    pub fn rider(&self, client: usize) -> Option<ssx_net_proto::RiderId> {
        self.clients[client].world().resource::<crate::client::ServerSaid>().welcome.map(|(r, _)| r)
    }

    pub fn synced(&self, client: usize) -> bool {
        self.clients[client].world().get_resource::<LocalTimelineSync>().is_some_and(LocalTimelineSync::is_synced)
    }

    pub fn server_link(&mut self, client: usize) -> Option<Entity> {
        let id = self.client_ids[client];
        let world = self.server.world_mut();
        let mut links = world.query_filtered::<(Entity, &RemoteId), With<ClientOf>>();
        links.iter(world).find(|(_, r)| r.0 == PeerId::Netcode(id)).map(|(e, _)| e)
    }

    pub fn key(&self) -> [u8; 32] {
        self.key
    }

    pub fn client_meter(&self, client: usize) -> WireMeter {
        self.clients[client].world().get::<WireMeter>(self.client_entities[client]).cloned().unwrap_or_default()
    }
}

pub fn test_instance() -> crate::server::InstanceConfig {
    let spec = ssx_net_proto::InstanceSpec::new(
        "exp_alps_vorassay_drop_3",
        ssx_net_proto::Mode::Freeride,
        ssx_net_proto::Rewinds::On,
        false,
        ssx_net_proto::GhostStyle::Character,
    );
    let instances =
        spec.ok().map(|spec| crate::server::InstanceSetup { spec, capacity: crate::limits::MAX_CLIENTS, rounds: None });
    crate::server::InstanceConfig { instances: instances.into_iter().collect() }
}

pub fn ready(round: u16) -> ssx_net_proto::ClientControl {
    let sample = ssx_net_proto::NetPose::new(0, ssx_net_proto::PoseFlags::default(), sample_block(0))
        .unwrap_or_else(|e| unreachable!("a packer-shaped block makes a sample: {e}"));
    let second = sample.clone().with_execution(2).unwrap_or_else(|e| unreachable!("2 is within the wire's count: {e}"));
    ssx_net_proto::ClientControl::Ready { round, sample: Box::new(sample), second: Box::new(second) }
}

pub fn sample_block(seed: u64) -> ssx_formats::ghost::PoseBlock {
    use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};
    let mut noise = craft::Noise(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut b = [0u8; POSE_BLOCK_SIZE];
    for (i, v) in [0.0f32, 0.0, 0.0, 1.0, seed as f32, 2.0, -3.0].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    for (start, end) in [(0x1C, 0x24), (0x30, 0x47), (0x4C, POSE_BLOCK_SIZE)] {
        let bytes = noise.bytes(end - start);
        b[start..end].copy_from_slice(&bytes);
    }
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    b[0x22..0x24].copy_from_slice(&(ssx_formats::ghost::pack_half(0.5) | 0x8000).to_be_bytes());
    PoseBlock::parse(&b).unwrap_or_else(|e| unreachable!("240 bytes always parse: {e}"))
}
