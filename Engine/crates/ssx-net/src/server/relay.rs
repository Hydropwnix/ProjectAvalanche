use core::time::Duration;
use std::collections::HashMap;

use bevy_ecs::prelude::*;
use bevy_time::{Real, Time};
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{Connected, LocalTimeline, MessageReceiver, MessageSender, Rooms};
use ssx_gameplay::pose_rules::{RiderTrack, TickWindow, Verdict, Violation, check, order_of};
use ssx_gameplay::tiers::{Decimator, Tier, TierRates, distance};
use ssx_net_proto::{NetPose, RelayedPose, RiderId};

use super::guard::{KickReason, Kicked, Leaving, PoseRule, kick};
use super::instance::Instances;
use super::{Lobby, ServerStats};
use crate::limits::{MAX_POSES_PER_SECOND, POSE_BURST, POSE_TICK_WINDOW};
use crate::protocol::{PoseChannel, Wire};

#[derive(Component, Clone, Debug)]
pub struct RiderLink {
    rider: RiderId,
    track: RiderTrack,
    budget: f64,
    refilled: Option<Duration>,
}

impl RiderLink {
    pub(super) fn new(rider: RiderId) -> Self {
        Self { rider, track: RiderTrack::default(), budget: f64::from(POSE_BURST), refilled: None }
    }

    pub fn rider(&self) -> RiderId {
        self.rider
    }

    pub fn newest_tick(&self) -> Option<u32> {
        self.track.newest_tick()
    }

    pub fn position(&self) -> Option<[f32; 3]> {
        self.track.position()
    }

    pub(super) fn restart_track(&mut self) {
        self.track = RiderTrack::default();
    }

    fn spend(&mut self, now: Duration) -> bool {
        if let Some(then) = self.refilled {
            let gained = now.saturating_sub(then).as_secs_f64() * f64::from(MAX_POSES_PER_SECOND);
            self.budget = (self.budget + gained).min(f64::from(POSE_BURST));
        }
        self.refilled = Some(now);
        if self.budget >= 1.0 {
            self.budget -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct RelayState {
    decimators: HashMap<(u16, u16), Decimator>,
}

#[derive(Resource, Clone, Copy, Debug, Default, PartialEq)]
pub struct RelayRates(pub TierRates);

impl RelayState {
    pub(super) fn forget(&mut self, rider: u16) {
        self.decimators.retain(|&(from, to), _| from != rider && to != rider);
    }
}

type Riders<'a> = (
    Entity,
    &'a mut RiderLink,
    Option<&'a mut MessageReceiver<Wire<NetPose>>>,
    &'a mut MessageSender<Wire<RelayedPose>>,
    (Option<&'a Rooms>, Option<&'a super::time::LinkTime>),
);

type Active = (With<ClientOf>, With<Connected>, Without<Kicked>, Without<Leaving>);

pub(super) fn drop_leaving(mut leaving: Query<&mut MessageReceiver<Wire<NetPose>>, With<Leaving>>) {
    for mut receiver in &mut leaving {
        receiver.receive().for_each(drop);
    }
}

fn rule(v: Violation) -> KickReason {
    match v {
        Violation::Tick(_) => KickReason::PoseTick,
        Violation::ViewLead(_) => KickReason::PoseView,
        Violation::Coordinate(_) => KickReason::AbsurdPose(PoseRule::Coordinate),
        Violation::RotationLength(_) => KickReason::AbsurdPose(PoseRule::RotationLength),
        Violation::Speed(_) => KickReason::AbsurdPose(PoseRule::Speed),
        Violation::Step(_) => KickReason::AbsurdPose(PoseRule::Step),
    }
}

type LinkPlace = (Entity, u16, Option<[f32; 3]>, Vec<lightyear::prelude::RoomId>);

fn tier_index(t: Tier) -> usize {
    match t {
        Tier::Near => 0,
        Tier::Middle => 1,
        Tier::Far => 2,
    }
}

fn relay_roles(instances: Option<&Instances>, rider: u16) -> (bool, bool) {
    instances.map_or((true, true), |i| i.relay_roles(rider))
}

fn places(instances: Option<&Instances>) -> HashMap<u16, (u16, usize)> {
    let mut out = HashMap::new();
    for s in instances.iter().flat_map(|i| &i.list) {
        for (place, row) in s.instance.standings().iter().enumerate() {
            out.entry(row.rider).or_insert((s.instance.id(), place));
        }
    }
    out
}

#[derive(bevy_ecs::system::SystemParam)]
pub(super) struct RelayWorld<'w> {
    timeline: Res<'w, LocalTimeline>,
    time: Res<'w, Time<Real>>,
    server_time: Res<'w, super::time::ServerTime>,
    lobby: Res<'w, Lobby>,
    instances: Option<Res<'w, Instances>>,
    rates: Option<Res<'w, RelayRates>>,
    validation: Option<ResMut<'w, super::validation::ValidationInbox>>,
}

pub(super) fn relay(
    mut riders: Query<Riders, Active>,
    world: RelayWorld,
    mut state: ResMut<RelayState>,
    mut stats: ResMut<ServerStats>,
    mut commands: Commands,
) {
    let RelayWorld { timeline, time, server_time, lobby, instances, rates, mut validation } = world;
    let instances = instances.as_deref();
    let rates = rates.map_or_else(TierRates::default, |r| r.0);
    let server_tick = timeline.tick().0;
    let now = time.elapsed();
    let mut accepted: Vec<(Entity, RelayedPose)> = Vec::new();
    for (entity, mut link, receiver, _, (_, link_time)) in &mut riders {
        let Some(mut receiver) = receiver else { continue };
        let lost = server_time.lost_since(link_time).ceil().min(f64::from(u32::MAX)) as u32;
        let rates = link_time
            .zip(time.last_update())
            .map_or(0.0, |(l, at)| l.rates_since_connect(at))
            .ceil()
            .min(f64::from(u32::MAX)) as u32;
        let window = TickWindow {
            behind: POSE_TICK_WINDOW,
            ahead: POSE_TICK_WINDOW.saturating_add(lost),
            view_behind: POSE_TICK_WINDOW.saturating_add(rates),
            view_ahead: POSE_TICK_WINDOW.saturating_add(lost).saturating_add(rates),
        };
        for wire in receiver.receive() {
            stats.poses_received += 1;
            if !link.spend(now) {
                stats.poses_over_rate += 1;
                continue;
            }
            let pose = match NetPose::decode(wire.bytes()) {
                Ok(pose) => pose,
                Err(_) => {
                    kick(&mut commands, entity, KickReason::MalformedPose);
                    break;
                }
            };
            let verdict = check(&mut link.track, server_tick, window, &pose);
            if let (Ok(Verdict::Relay), Some(inbox)) = (&verdict, validation.as_deref_mut()) {
                let (rider, pose) = (link.rider.0, Box::new(pose.clone()));
                inbox.0.push(super::validation::Input::Sample { rider, link: entity, pose });
            }
            match verdict {
                Ok(Verdict::Relay) if !relay_roles(instances, link.rider.0).0 => stats.poses_before_look += 1,
                Ok(Verdict::Relay) => accepted.push((entity, RelayedPose { rider: link.rider, pose })),
                Ok(Verdict::Stale) => stats.poses_stale += 1,
                Ok(Verdict::OlderEpoch) => stats.poses_older_epoch += 1,
                Err(v) => {
                    kick(&mut commands, entity, rule(v));
                    break;
                }
            }
        }
    }
    if accepted.is_empty() {
        return;
    }
    let links: Vec<LinkPlace> = riders
        .iter()
        .map(|(e, link, _, _, (rooms, _))| {
            let rooms = rooms.map_or_else(Vec::new, |r| r.rooms().filter(|&room| room != lobby.0).collect());
            (e, link.rider.0, link.position(), rooms)
        })
        .collect();
    let find = |e: Entity| links.iter().find(|l| l.0 == e);
    let places = places(instances);
    let neighbours = |a: u16, b: u16| match (places.get(&a), places.get(&b)) {
        (Some((ia, pa)), Some((ib, pb))) => ia == ib && pa.abs_diff(*pb) == 1,
        _ => false,
    };
    for (from, relayed) in &accepted {
        let Some((_, from_rider, _, from_rooms)) = find(*from) else { continue };
        let at = relayed.pose.block().trajectory_position;
        let wire = Wire::relayed(relayed);
        for (to, _, _, mut sender, _) in &mut riders {
            let Some((_, to_rider, to_at, to_rooms)) = find(to) else { continue };
            if to == *from || !from_rooms.iter().any(|r| to_rooms.contains(r)) || !relay_roles(instances, *to_rider).1 {
                continue;
            }
            let tier =
                if neighbours(*from_rider, *to_rider) { Tier::Near } else { Tier::at(to_at.map(|p| distance(at, p))) };
            let decimator = state.decimators.entry((*from_rider, *to_rider)).or_default();
            if decimator.admit_every(order_of(&relayed.pose), rates.interval(tier)) {
                sender.send_with_priority::<PoseChannel>(wire.clone(), tier.priority());
                stats.poses_relayed += 1;
                stats.poses_by_tier[tier_index(tier)] += 1;
            } else {
                stats.poses_decimated += 1;
            }
        }
    }
}
