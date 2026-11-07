use std::collections::BTreeMap;

use bevy_ecs::prelude::*;
use lightyear::prelude::{Client, Disconnect};
use ssx_net_proto::{Appearance, ClientControl, PlayerStatus, RiderId};

use super::{RemotePoses, ServerSaid};
use crate::protocol::{ControlChannel, Standings, Wire};
use crate::roster::{
    InInstance, InstanceInfo, PlayerAppearance, PlayerKeyId, PlayerName, PlayerPing, PlayerState, RiderNumber,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterPlayer {
    pub entity: Entity,
    pub name: String,
    pub key_id: [u8; 8],
    pub instance: u16,
    pub appearance: Option<Appearance>,
    pub status: Option<PlayerStatus>,
    pub ping_ms: Option<u16>,
}

#[derive(Resource, Default)]
pub struct ClientRoster {
    pub own: Option<(RiderId, u16)>,
    pub instance: Option<InstanceInfo>,
    pub players: BTreeMap<RiderId, RosterPlayer>,
    pub standings: Option<(u16, Vec<ssx_net_proto::StandingRow>)>,
    pub unknown_samples: u64,
}

impl ClientRoster {
    pub fn others(&self) -> impl Iterator<Item = (RiderId, &RosterPlayer)> {
        let own = self.own;
        self.players
            .iter()
            .filter(move |(r, p)| own.is_some_and(|(me, instance)| **r != me && p.instance == instance))
            .map(|(r, p)| (*r, p))
    }
}

type Player<'a> = (
    Entity,
    &'a RiderNumber,
    &'a PlayerName,
    &'a PlayerKeyId,
    &'a InInstance,
    Option<&'a PlayerAppearance>,
    Option<&'a PlayerState>,
    Option<&'a PlayerPing>,
);

pub(super) fn update_roster(
    (players, instances, tables): (Query<Player>, Query<&InstanceInfo>, Query<&Standings>),
    said: Res<ServerSaid>,
    mut roster: ResMut<ClientRoster>,
    mut poses: ResMut<RemotePoses>,
    links: Query<Entity, With<Client>>,
    mut commands: Commands,
) {
    let mut refused = None;
    let mut table = BTreeMap::new();
    for (entity, rider, name, key, instance, appearance, state, ping) in &players {
        let player = RosterPlayer {
            entity,
            name: name.as_str().to_string(),
            key_id: key.0,
            instance: instance.0,
            appearance: appearance.map(|a| a.0),
            status: state.map(|s| s.0),
            ping_ms: ping.map(|p| p.millis()),
        };
        if table.insert(RiderId(rider.0), player).is_some() {
            refused = Some(format!("the server names rider {} twice", rider.0));
        }
    }
    roster.own = said.welcome;
    let wanted = said.welcome.map(|(_, i)| i);
    let mut instance = None;
    for info in &instances {
        if Some(info.id) == wanted {
            if instance.is_some() {
                refused = Some(format!("the server names instance {} twice", info.id));
            }
            instance = Some(info.clone());
        }
    }
    let mut standings = None;
    for t in &tables {
        if Some(t.instance()) != wanted {
            continue;
        }
        if standings.is_some() {
            refused = Some(format!("the server names the standings of instance {} twice", t.instance()));
        }
        let mut rows = Vec::with_capacity(t.rows().len());
        for row in t.rows() {
            if !rows.iter().any(|r: &ssx_net_proto::StandingRow| r.rider == row.rider) {
                rows.push(*row);
            }
        }
        standings = Some((t.round(), rows));
    }
    if let Some(reason) = refused {
        if poses.refused.is_none() {
            poses.refused = Some(reason);
        }
        for link in &links {
            commands.trigger(Disconnect { entity: link });
        }
        return;
    }
    roster.instance = instance;
    roster.players = table;
    roster.standings = standings;
    poses.riders.retain(|r, _| roster.players.contains_key(r));
}

#[derive(Resource, Default)]
pub struct ControlOutbox(pub Vec<ClientControl>);

pub(super) fn send_control(
    mut outbox: ResMut<ControlOutbox>,
    mut links: Query<&mut lightyear::prelude::MessageSender<Wire<ClientControl>>, super::Connection>,
) {
    for message in outbox.0.drain(..) {
        let wire = Wire::client_control(&message);
        for mut sender in &mut links {
            sender.send::<ControlChannel>(wire.clone());
        }
    }
}
