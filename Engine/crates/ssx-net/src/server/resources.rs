use bevy_ecs::prelude::*;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{Connected, MessageSender};
use ssx_net_proto::ServerControl;

use super::ServerStats;
use super::guard::{Kicked, Leaving};
use crate::protocol::{ControlChannel, Wire};

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourcesNow(pub Option<[u8; 32]>);

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq)]
pub struct StandIns(pub std::collections::HashMap<u32, u32>);

impl StandIns {
    pub fn of(&self, key: u32) -> u32 {
        self.0.get(&key).copied().unwrap_or(key)
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Told(pub Option<[u8; 32]>);

type Active = (With<ClientOf>, With<Connected>, Without<Kicked>, Without<Leaving>);

pub(super) fn tell_changes(
    now: Res<ResourcesNow>,
    mut links: Query<(&mut Told, &mut MessageSender<Wire<ServerControl>>), Active>,
    mut stats: ResMut<ServerStats>,
) {
    let Some(manifest) = now.0 else { return };
    let mut wire = None;
    for (mut told, mut sender) in &mut links {
        if told.0 == Some(manifest) {
            continue;
        }
        let wire = wire.get_or_insert_with(|| Wire::control(&ServerControl::ResourcesChanged { manifest }));
        sender.send::<ControlChannel>(wire.clone());
        told.0 = Some(manifest);
        stats.resources_told += 1;
    }
}
