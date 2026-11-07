use bevy_ecs::prelude::*;
use bevy_ecs::query::QueryFilter;
use lightyear::prelude::Link;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traffic {
    pub sent_bytes: u64,
    pub sent_datagrams: u64,
    pub recv_bytes: u64,
    pub recv_datagrams: u64,
}

#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkTraffic(pub Traffic);

pub trait TrafficCounter: Resource<Mutability = bevy_ecs::component::Mutable> {
    fn traffic(&mut self) -> &mut Traffic;
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClientTraffic(pub Traffic);

impl TrafficCounter for ClientTraffic {
    fn traffic(&mut self) -> &mut Traffic {
        &mut self.0
    }
}

pub(crate) fn count_sent<F: QueryFilter, R: TrafficCounter>(
    mut links: Query<(&mut Link, Option<&mut LinkTraffic>), F>,
    mut counter: ResMut<R>,
) {
    let t = counter.traffic();
    for (mut link, mut own) in &mut links {
        for _ in 0..link.send.len() {
            let Some(payload) = link.send.pop() else { break };
            t.sent_bytes += payload.len() as u64;
            t.sent_datagrams += 1;
            if let Some(own) = own.as_deref_mut() {
                own.0.sent_bytes += payload.len() as u64;
                own.0.sent_datagrams += 1;
            }
            link.send.push(payload);
        }
    }
}

pub(crate) fn count_received<F: QueryFilter, R: TrafficCounter>(
    mut links: Query<(&mut Link, Option<&mut LinkTraffic>), F>,
    mut counter: ResMut<R>,
) {
    let t = counter.traffic();
    for (mut link, mut own) in &mut links {
        for _ in 0..link.recv.len() {
            let Some(payload) = link.recv.pop() else { break };
            t.recv_bytes += payload.len() as u64;
            t.recv_datagrams += 1;
            if let Some(own) = own.as_deref_mut() {
                own.0.recv_bytes += payload.len() as u64;
                own.0.recv_datagrams += 1;
            }
            link.recv.push_raw(payload);
        }
    }
}
