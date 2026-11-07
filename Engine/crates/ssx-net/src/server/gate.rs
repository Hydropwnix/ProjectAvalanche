use std::net::IpAddr;
use std::sync::Arc;

use bevy_ecs::prelude::*;
use ssx_net_proto::keys::PlayerKey;
use ssx_net_proto::operator::Expiry;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub reason: String,
    pub until: Expiry,
}

pub trait Operator: Send + Sync + 'static {
    fn banned(&self, player: &PlayerKey, address: IpAddr) -> Option<Decision>;
    fn muted(&self, player: &PlayerKey) -> Option<Decision>;
}

#[derive(Resource, Clone)]
pub struct OperatorGate(pub Arc<dyn Operator>);
