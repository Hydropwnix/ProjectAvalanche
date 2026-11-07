use core::time::Duration;
use std::collections::HashMap;
use std::net::SocketAddr;

use bevy_app::{App, Plugin, PostUpdate, PreUpdate};
use bevy_ecs::prelude::*;
use bevy_replicon::shared::receive_error::ReplicationReceiveError;
use bevy_time::{Fixed, Real, Time};
use lightyear::netcode::ConnectToken;
use lightyear::netcode::prelude::client::{NetcodeClient, NetcodeConfig};
use lightyear::prelude::client::ClientPlugins;
use lightyear::prelude::{
    Authentication, Client, Connected, Disconnect, Link, LocalAddr, LocalTimeline, LocalTimelineSync,
    MessageReceiveError, MessageReceiver, MessageSender, PeerAddr, ProtocolMismatch, RemoteEvent, ReplicationReceiver,
    TransportReceiveError, TransportSendError, UdpIo,
};
use lightyear_replication::metadata::SenderMetadata;
use ssx_gameplay::pose_rules::{follows, order_of};
use ssx_net_proto::join::TOKEN_SIZE;
use ssx_net_proto::{GoodbyeReason, NetPose, RelayedPose, RiderId, ServerControl, netcode_protocol_id};

use crate::limits::{MAX_ACCEPTED_TIMEOUT_SECS, MAX_ACCEPTED_TOKEN_SECS, POSE_TICK_WINDOW};

mod chat;
mod clock;
mod roster;
mod scripts;
use crate::protocol::{PoseChannel, ProtocolPlugin, Wire};
use crate::tick::{NetSystems, client_sync_config, tick_duration};
pub use chat::{ChatBody, ChatFrom, ChatInbox, ChatLine, ChatSend, KEPT_LINES};
pub use clock::{
    ALIGN_BEFORE_START_TICKS, ServerClock, StepGrid, VIEW_MAP_EVERY_TICKS, VIEW_MAP_MAX_AGE_TICKS,
    VIEW_MAP_WINDOW_TICKS, ViewClock, ViewMap, align_due, align_steps, local_time,
};
pub use roster::{ClientRoster, ControlOutbox, RosterPlayer};
pub use scripts::{ScriptReceived, ScriptSend};

pub struct NetClientPlugin;

impl Plugin for NetClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ClientPlugins { tick_duration: tick_duration() });
        app.add_plugins(ProtocolPlugin);
        app.insert_resource(client_sync_config());
        app.init_resource::<PoseOutbox>().init_resource::<RemotePoses>().add_message::<RemotePoseReceived>();
        app.init_resource::<ServerSaid>().init_resource::<MetadataSender>();
        app.add_observer(refuse_metadata_flood).add_observer(forget_metadata_sender);
        app.init_resource::<SyncedAtFrameStart>();
        app.init_resource::<ClientRoster>().init_resource::<ControlOutbox>().init_resource::<ServerClock>();
        app.init_resource::<ViewClock>();
        app.init_resource::<crate::traffic::ClientTraffic>();
        app.init_resource::<ChatInbox>().init_resource::<ChatSend>();
        app.init_resource::<ScriptReceived>().init_resource::<ScriptSend>();
        app.add_systems(PreUpdate, scripts::receive.in_set(NetSystems::Receive).after(receive_control));
        app.add_systems(PostUpdate, scripts::send.in_set(NetSystems::Send));
        app.add_systems(
            PostUpdate,
            crate::traffic::count_sent::<With<Client>, crate::traffic::ClientTraffic>
                .after(lightyear::connection::ConnectionSystems::Send)
                .before(lightyear::link::LinkSystems::Send),
        );
        app.add_systems(
            PreUpdate,
            crate::traffic::count_received::<With<Client>, crate::traffic::ClientTraffic>
                .after(lightyear::link::LinkSystems::Receive)
                .before(lightyear::connection::ConnectionSystems::Receive),
        );
        app.add_systems(
            PreUpdate,
            (
                note_sync,
                clock::update,
                receive_control,
                roster::update_roster,
                receive,
                chat::receive_chat,
                refuse_malformed,
            )
                .chain()
                .in_set(NetSystems::Receive),
        );
        app.add_observer(on_protocol_mismatch).add_observer(clock::count_shifts);
        app.configure_sets(PreUpdate, NetSystems::Receive.after(lightyear::core::timeline::TimelineSystems::Advance));
        app.add_systems(
            bevy_app::RunFixedMainLoop,
            clock::pending_taken.in_set(bevy_app::RunFixedMainLoopSystems::BeforeFixedMainLoop),
        );
        app.add_systems(PostUpdate, (send, roster::send_control, chat::send_chat).in_set(NetSystems::Send));
    }
}

#[derive(Resource, Default)]
pub struct PoseOutbox {
    pub next: Option<NetPose>,
    pub sent: u64,
    pub off_tick: u64,
    pub implausible: u64,
    pub implausible_why: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RemoteRider {
    pub entity: Entity,
    pub pose: NetPose,
    pub received_at: Duration,
    pub samples: u64,
    pub stale: u64,
}

#[derive(Resource, Default)]
pub struct RemotePoses {
    pub riders: HashMap<RiderId, RemoteRider>,
    pub refused: Option<String>,
}

#[derive(Resource, Default)]
pub struct ServerSaid {
    pub goodbye: Option<(GoodbyeReason, String)>,
    pub ban: Option<ssx_net_proto::operator::Expiry>,
    pub welcome: Option<(RiderId, u16)>,
    pub chat: Option<bool>,
    pub start: Option<(u16, u32)>,
    pub starts: u64,
    pub invalidated: Option<(u16, ssx_net_proto::Check)>,
    pub resources_changed: Option<[u8; 32]>,
    pub resources_changes: u64,
}

#[derive(Message, Clone, Debug)]
pub struct RemotePoseReceived {
    pub rider: RiderId,
    pub pose: NetPose,
    pub received_at: Duration,
}

type ClientLink<'a> =
    (Entity, Option<&'a mut MessageReceiver<Wire<RelayedPose>>>, Option<&'a mut MessageReceiver<Wire<NetPose>>>);

fn receive(
    mut links: Query<ClientLink, Connection>,
    mut poses: ResMut<RemotePoses>,
    mut roster: ResMut<ClientRoster>,
    mut events: MessageWriter<RemotePoseReceived>,
    time: Res<Time<Real>>,
    mut commands: Commands,
) {
    let now = time.elapsed();
    for (entity, relayed, wrong) in &mut links {
        if let Some(mut wrong) = wrong
            && wrong.receive().count() > 0
        {
            poses.refused = Some("the server sent a client-to-server message".into());
            commands.trigger(Disconnect { entity });
            continue;
        }
        let Some(mut relayed) = relayed else { continue };
        for wire in relayed.receive() {
            let sample = match RelayedPose::decode(wire.bytes()) {
                Ok(sample) => sample,
                Err(e) => {
                    poses.refused = Some(format!("the server relayed a malformed sample: {e}"));
                    commands.trigger(Disconnect { entity });
                    break;
                }
            };
            let RelayedPose { rider, pose } = sample;
            if let Err(v) = ssx_gameplay::pose_rules::plausible(&pose) {
                poses.refused = Some(format!("the server relayed a sample of rider {}: {v}", rider.0));
                commands.trigger(Disconnect { entity });
                break;
            }
            let own = roster.own.is_some_and(|(me, _)| me == rider);
            let Some(player) = roster.players.get(&rider).filter(|_| !own).map(|p| p.entity) else {
                roster.unknown_samples += 1;
                continue;
            };
            let fresh = || RemoteRider { entity: player, pose: pose.clone(), received_at: now, samples: 0, stale: 0 };
            let entry = poses.riders.entry(rider).or_insert_with(fresh);
            if entry.entity != player {
                *entry = fresh();
            }
            if entry.samples > 0 && follows(order_of(&pose), order_of(&entry.pose)).is_err() {
                entry.stale += 1;
                continue;
            }
            entry.samples += 1;
            entry.pose = pose.clone();
            entry.received_at = now;
            events.write(RemotePoseReceived { rider, pose, received_at: now });
        }
    }
}

type Connection = (With<Client>, With<Connected>);

fn receive_control(
    mut links: Query<(Entity, &mut MessageReceiver<Wire<ServerControl>>), Connection>,
    mut said: ResMut<ServerSaid>,
    mut poses: ResMut<RemotePoses>,
    mut commands: Commands,
) {
    for (entity, mut receiver) in &mut links {
        for wire in receiver.receive() {
            match ServerControl::decode(wire.bytes()) {
                Ok(ServerControl::Goodbye { reason, text, ban }) => {
                    said.goodbye = Some((reason, text));
                    said.ban = ban;
                }
                Ok(ServerControl::Welcome { rider, instance, chat }) => {
                    if said.welcome.is_some() {
                        poses.refused = Some("the server welcomed this connection twice".into());
                        commands.trigger(Disconnect { entity });
                        break;
                    }
                    said.welcome = Some((RiderId(rider), instance));
                    said.chat = Some(chat);
                }
                Ok(ServerControl::StartAt { round, tick }) => {
                    if said.welcome.is_none() {
                        poses.refused = Some("the server sent a round's start before its welcome".into());
                        commands.trigger(Disconnect { entity });
                        break;
                    }
                    said.start = Some((round, tick));
                    said.starts += 1;
                }
                Ok(ServerControl::Invalidated { round, check }) => {
                    if said.welcome.is_none() {
                        poses.refused = Some("the server invalidated a run before its welcome".into());
                        commands.trigger(Disconnect { entity });
                        break;
                    }
                    said.invalidated = Some((round, check));
                }
                Ok(ServerControl::ResourcesChanged { manifest }) => {
                    said.resources_changed = Some(manifest);
                    said.resources_changes += 1;
                }
                Err(e) => {
                    poses.refused = Some(format!("the server sent a malformed control message: {e}"));
                    commands.trigger(Disconnect { entity });
                    break;
                }
            }
        }
    }
}

fn refuse_malformed(
    mut packets: MessageReader<TransportReceiveError>,
    mut messages: MessageReader<MessageReceiveError>,
    mut replication: MessageReader<ReplicationReceiveError>,
    mut sends: MessageReader<TransportSendError>,
    links: Query<Entity, With<Client>>,
    mut poses: ResMut<RemotePoses>,
    mut commands: Commands,
) {
    let mut refused: Vec<(Entity, String)> = sends
        .read()
        .map(|e| (e.entity, format!("the server does not acknowledge what this client sends: {}", e.error)))
        .chain(
            packets.read().map(|e| (e.entity, format!("the server sent a packet the transport refused: {}", e.error))),
        )
        .chain(
            messages.read().map(|e| (e.entity, format!("the server sent a message that does not read: {}", e.error))),
        )
        .collect();
    for e in replication.read() {
        let reason = format!("the server's replication data does not apply: {}", e.error);
        refused.extend(links.iter().map(|link| (link, reason.clone())));
    }
    for (entity, reason) in refused {
        if links.contains(entity) && poses.refused.is_none() {
            poses.refused = Some(reason);
            commands.trigger(Disconnect { entity });
        }
    }
}

#[derive(Resource, Default)]
struct MetadataSender(Option<Entity>);

fn refuse_metadata_flood(
    metadata: On<RemoteEvent<SenderMetadata>>,
    links: Query<Entity, (With<Client>, With<Connected>)>,
    mut first: ResMut<MetadataSender>,
    mut poses: ResMut<RemotePoses>,
    mut commands: Commands,
) {
    let named = metadata.trigger.sender_entity;
    match first.0 {
        None => first.0 = Some(named),
        Some(entity) if entity == named => {}
        Some(_) => {
            for link in &links {
                if poses.refused.is_none() {
                    poses.refused = Some("the server's replication metadata names a second sender".into());
                }
                commands.trigger(Disconnect { entity: link });
            }
        }
    }
}

fn forget_metadata_sender(_connected: On<Add<Connected>>, mut first: ResMut<MetadataSender>) {
    first.0 = None;
}

fn on_protocol_mismatch(mismatch: On<ProtocolMismatch>, mut poses: ResMut<RemotePoses>) {
    poses.refused = Some(format!("the server's protocol differs: {}", mismatch.error));
}

#[derive(Resource, Default)]
struct SyncedAtFrameStart(bool);

fn note_sync(sync: Res<LocalTimelineSync>, mut at_start: ResMut<SyncedAtFrameStart>) {
    at_start.0 = sync.is_synced();
}

fn send(
    mut outbox: ResMut<PoseOutbox>,
    (sync, at_start, timeline, clock): (
        Res<LocalTimelineSync>,
        Res<SyncedAtFrameStart>,
        Res<LocalTimeline>,
        Res<ServerClock>,
    ),
    fixed: Res<Time<Fixed>>,
    mut links: Query<&mut MessageSender<Wire<NetPose>>, Connection>,
) {
    if !sync.is_synced() || !at_start.0 {
        outbox.next = None;
        return;
    }
    let Some(pose) = outbox.next.take() else { return };
    let server = clock.server_now(&timeline, &fixed);
    if server.is_none_or(|now| (pose.time() - now).abs() > f64::from(POSE_TICK_WINDOW / 2)) {
        outbox.off_tick += 1;
        return;
    }
    if let Err(v) = ssx_gameplay::pose_rules::plausible(&pose) {
        outbox.implausible += 1;
        outbox.implausible_why = Some(v.to_string());
        return;
    }
    for mut sender in &mut links {
        sender.send::<PoseChannel>(Wire::pose(&pose));
        outbox.sent += 1;
    }
}

pub fn connect_token(bytes: &[u8; TOKEN_SIZE], joined: SocketAddr) -> Result<Authentication, String> {
    let token = ConnectToken::try_from_bytes(bytes).map_err(|e| format!("the connect token does not read: {e}"))?;
    if token.protocol_id() != netcode_protocol_id() {
        return Err(format!("the connect token is for protocol {:#018x}", token.protocol_id()));
    }
    let addresses = token.server_addresses();
    if addresses != [joined] {
        return Err(format!("the connect token names {addresses:?}, not the server joined ({joined})"));
    }
    let lifetime = token.expire_timestamp().saturating_sub(token.create_timestamp());
    if lifetime > MAX_ACCEPTED_TOKEN_SECS {
        return Err(format!("the connect token is valid for {lifetime} s"));
    }
    if !(1..=MAX_ACCEPTED_TIMEOUT_SECS).contains(&token.timeout_seconds()) {
        return Err(format!("the connect token's timeout is {} s", token.timeout_seconds()));
    }
    Ok(Authentication::Token(token))
}

pub fn reset_session(world: &mut World) {
    let doomed: Vec<Entity> =
        world.query_filtered::<Entity, Or<(With<Client>, With<bevy_replicon::client::Remote>)>>().iter(world).collect();
    for e in doomed {
        world.despawn(e);
    }
    world.insert_resource(PoseOutbox::default());
    world.insert_resource(RemotePoses::default());
    world.insert_resource(ServerSaid::default());
    world.insert_resource(MetadataSender::default());
    world.insert_resource(SyncedAtFrameStart::default());
    world.insert_resource(ClientRoster::default());
    world.insert_resource(ControlOutbox::default());
    world.insert_resource(ServerClock::default());
    world.insert_resource(crate::traffic::ClientTraffic::default());
    world.insert_resource(ChatInbox::default());
    world.insert_resource(ChatSend::default());
    world.insert_resource(ScriptReceived::default());
    world.insert_resource(ScriptSend::default());
}

pub fn spawn_udp_client(
    world: &mut World,
    local: SocketAddr,
    server: SocketAddr,
    auth: Authentication,
    link: Link,
) -> Result<Entity, lightyear::netcode::Error> {
    let netcode = NetcodeClient::new(auth, NetcodeConfig::default())?;
    Ok(world
        .spawn((Client, LocalAddr(local), PeerAddr(server), link, ReplicationReceiver, netcode, UdpIo::default()))
        .id())
}
