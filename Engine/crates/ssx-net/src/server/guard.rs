use core::time::Duration;

use bevy_app::{App, Last, PostUpdate, PreUpdate};
use bevy_ecs::prelude::*;
use bevy_replicon::shared::receive_error::ReplicationReceiveError;
use bevy_time::{Real, Time};
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{
    Connected, Connecting, Disconnect, LinkOf, MessageReceiveError, MessageReceiver, MessageSender, RemoteEvent,
    RemoteId, TransportReceiveError, TransportSendError,
};
use lightyear::transport::plugin::TransportSystems;
use lightyear_replication::metadata::SenderMetadata;
use ssx_net_proto::control::MAX_CONTROL_TEXT;
use ssx_net_proto::{GoodbyeReason, RelayedPose, ServerChat, ServerControl};

use super::ServerStats;
use super::session::TicketRefusal;
use crate::limits::{LEAVE_GRACE, MAX_PENDING_LINKS, PENDING_LINK_TIMEOUT, UNAUTHENTICATED_LINK_TIMEOUT};
use crate::protocol::{ControlChannel, Wire};
use crate::tick::NetSystems;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuardSystems {
    Direction,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct PendingSince(pub Duration);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KickReason {
    ServerFull,
    Ticket(TicketRefusal),
    Replaced,
    Operator,
    ServerStopping,
    MalformedPose,
    PoseTick,
    PoseView,
    WrongDirection,
    MalformedPacket,
    MalformedMessage,
    MalformedReplication,
    Metadata,
    AbsurdPose(PoseRule),
    MalformedControl,
    ControlOrder,
    Round(ssx_gameplay::instance::Refusal),
    Unacknowledged,
    MalformedChat,
    ChatFlood,
    Validation(ssx_net_proto::Check),
    Banned(ssx_net_proto::operator::Expiry),
    ScriptFlood,
    MalformedScript,
    ScriptBacklog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PoseRule {
    Coordinate,
    RotationLength,
    Speed,
    Step,
}

impl KickReason {
    pub fn goodbye(self) -> Option<GoodbyeReason> {
        match self {
            Self::ServerFull => Some(GoodbyeReason::ServerFull),
            Self::Ticket(_) => Some(GoodbyeReason::Ticket),
            Self::Replaced => Some(GoodbyeReason::Replaced),
            Self::Operator => Some(GoodbyeReason::Kicked),
            Self::ServerStopping => Some(GoodbyeReason::ServerStopping),
            Self::ChatFlood => Some(GoodbyeReason::ChatFlood),
            Self::Validation(_) => Some(GoodbyeReason::Validation),
            Self::Banned(_) => Some(GoodbyeReason::Banned),
            Self::ScriptFlood => Some(GoodbyeReason::ScriptFlood),
            Self::ScriptBacklog => Some(GoodbyeReason::ScriptBacklog),
            Self::MalformedScript => None,
            Self::MalformedPose
            | Self::PoseTick
            | Self::PoseView
            | Self::WrongDirection
            | Self::MalformedPacket
            | Self::MalformedMessage
            | Self::MalformedReplication
            | Self::Metadata
            | Self::AbsurdPose(_)
            | Self::MalformedControl
            | Self::ControlOrder
            | Self::Round(_)
            | Self::Unacknowledged
            | Self::MalformedChat => None,
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct Kicked(pub KickReason);

#[derive(Component, Clone, Debug)]
pub struct Leaving {
    pub reason: KickReason,
    pub text: String,
    pub since: Duration,
}

const RECORDED_KICKS: usize = 256;

pub(super) fn register(app: &mut App) {
    app.add_observer(on_link).add_observer(refuse_client_metadata);
    app.configure_sets(PreUpdate, GuardSystems::Direction.in_set(NetSystems::Receive));
    app.add_systems(PreUpdate, (refuse_wrong_direction, refuse_malformed).in_set(GuardSystems::Direction));
    app.add_systems(PreUpdate, end_leaving.in_set(NetSystems::Receive).after(GuardSystems::Direction));
    app.add_systems(PostUpdate, send_goodbyes.in_set(NetSystems::Send));
    app.add_systems(PostUpdate, apply_kicks.before(TransportSystems::Send));
    app.add_systems(Last, reap_pending);
}

type Watched = (With<ClientOf>, Without<Kicked>);
type Unconnected = (With<LinkOf>, Without<Connected>);

pub(crate) fn kick(commands: &mut Commands, link: Entity, reason: KickReason) {
    commands.entity(link).insert(Kicked(reason));
}

pub fn leave(commands: &mut Commands, link: Entity, reason: KickReason, text: &str, now: Duration) {
    let mut text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    while text.len() > MAX_CONTROL_TEXT {
        text.pop();
    }
    commands.entity(link).insert_if_new(Leaving { reason, text, since: now });
}

type ControlSender<'a> = &'a mut MessageSender<Wire<ServerControl>>;
type StartedLeaving = (Added<Leaving>, Without<Kicked>);

fn send_goodbyes(mut leaving: Query<(&Leaving, ControlSender), StartedLeaving>) {
    for (leaving, mut sender) in &mut leaving {
        let Some(reason) = leaving.reason.goodbye() else { continue };
        let goodbye = match leaving.reason {
            KickReason::Banned(until) => ServerControl::banned(&leaving.text, until),
            _ => ServerControl::goodbye(reason, &leaving.text),
        };
        if let Ok(goodbye) = goodbye {
            sender.send::<ControlChannel>(Wire::control(&goodbye));
        }
    }
}

fn end_leaving(leaving: Query<(Entity, &Leaving), Without<Kicked>>, time: Res<Time<Real>>, mut commands: Commands) {
    let now = time.elapsed();
    for (entity, leaving) in &leaving {
        if now.saturating_sub(leaving.since) >= LEAVE_GRACE {
            kick(&mut commands, entity, leaving.reason);
        }
    }
}

fn on_link(
    trigger: On<Add<LinkOf>>,
    links: Query<(), Without<Connected>>,
    time: Res<Time<Real>>,
    mut stats: ResMut<ServerStats>,
    mut commands: Commands,
) {
    stats.links += 1;
    commands.entity(trigger.entity).insert(crate::traffic::LinkTraffic::default());
    if links.contains(trigger.entity) {
        commands.entity(trigger.entity).insert(PendingSince(time.elapsed()));
    }
}

fn reap_pending(
    pending: Query<(Entity, &PendingSince, Has<Connecting>), Unconnected>,
    time: Res<Time<Real>>,
    mut stats: ResMut<ServerStats>,
    mut commands: Commands,
) {
    let now = time.elapsed();
    let (mut unauthenticated, mut requested) = (Vec::new(), Vec::new());
    for (entity, since, connecting) in &pending {
        let limit = if connecting { PENDING_LINK_TIMEOUT } else { UNAUTHENTICATED_LINK_TIMEOUT };
        if now.saturating_sub(since.0) > limit {
            commands.entity(entity).despawn();
            stats.pending_reaped += 1;
        } else if connecting {
            requested.push((since.0, entity));
        } else {
            unauthenticated.push((since.0, entity));
        }
    }
    let alive = unauthenticated.len() + requested.len();
    if alive > MAX_PENDING_LINKS {
        unauthenticated.sort_unstable();
        requested.sort_unstable();
        for &(_, entity) in unauthenticated.iter().chain(&requested).take(alive - MAX_PENDING_LINKS) {
            commands.entity(entity).despawn();
            stats.pending_reaped += 1;
        }
    }
}

fn refuse_wrong_direction(
    mut poses: Query<(Entity, &mut MessageReceiver<Wire<RelayedPose>>), Watched>,
    mut controls: Query<(Entity, &mut MessageReceiver<Wire<ServerControl>>), Watched>,
    mut chats: Query<(Entity, &mut MessageReceiver<Wire<ServerChat>>), Watched>,
    mut commands: Commands,
) {
    let mut wrong: Vec<Entity> = Vec::new();
    wrong.extend(poses.iter_mut().filter_map(|(e, mut r)| (r.receive().count() > 0).then_some(e)));
    wrong.extend(controls.iter_mut().filter_map(|(e, mut r)| (r.receive().count() > 0).then_some(e)));
    wrong.extend(chats.iter_mut().filter_map(|(e, mut r)| (r.receive().count() > 0).then_some(e)));
    wrong.sort_unstable();
    wrong.dedup();
    for entity in wrong {
        kick(&mut commands, entity, KickReason::WrongDirection);
    }
}

type NewlyKicked = Added<Kicked>;

#[derive(Component, Clone, Copy, Debug)]
struct MetadataSender(Entity);

fn refuse_client_metadata(
    metadata: On<RemoteEvent<SenderMetadata>>,
    links: Query<(Entity, &RemoteId, Option<&MetadataSender>), Watched>,
    mut commands: Commands,
) {
    let named = metadata.trigger.sender_entity;
    let Some((link, _, first)) = links.iter().find(|(_, remote, _)| remote.0 == metadata.from) else { return };
    match first {
        None => {
            commands.entity(link).insert(MetadataSender(named));
        }
        Some(first) if first.0 == named => {}
        Some(_) => kick(&mut commands, link, KickReason::Metadata),
    }
}

fn refuse_malformed(
    mut packets: MessageReader<TransportReceiveError>,
    mut messages: MessageReader<MessageReceiveError>,
    mut replication: MessageReader<ReplicationReceiveError>,
    mut sends: MessageReader<TransportSendError>,
    links: Query<(), Watched>,
    mut commands: Commands,
) {
    let mut refused: Vec<(Entity, KickReason)> = Vec::new();
    refused.extend(sends.read().map(|e| (e.entity, KickReason::Unacknowledged)));
    refused.extend(packets.read().map(|e| (e.entity, KickReason::MalformedPacket)));
    refused.extend(messages.read().map(|e| (e.entity, KickReason::MalformedMessage)));
    refused.extend(replication.read().filter_map(|e| e.client.map(|c| (c, KickReason::MalformedReplication))));
    let mut kicked: Vec<Entity> = Vec::new();
    for (entity, reason) in refused {
        if links.contains(entity) && !kicked.contains(&entity) {
            kicked.push(entity);
            kick(&mut commands, entity, reason);
        }
    }
}

fn apply_kicks(kicked: Query<(Entity, &Kicked), NewlyKicked>, mut stats: ResMut<ServerStats>, mut commands: Commands) {
    for (entity, kicked) in &kicked {
        stats.kicked += 1;
        if stats.kick_reasons.len() == RECORDED_KICKS {
            stats.kick_reasons.pop_front();
        }
        stats.kick_reasons.push_back(kicked.0);
        commands.trigger(Disconnect { entity });
    }
}
