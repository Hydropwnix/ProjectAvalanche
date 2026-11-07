use bevy_ecs::prelude::*;
use bevy_replicon::prelude::EntityCommandsDiffExt;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{Connected, LocalTimeline, MessageSender};
use ssx_net_proto::{InstanceState, ServerControl, StandingRow};

use super::ServerStats;
use super::guard::{Kicked, Leaving};
use super::instance::{Instances, PlacedIn, PlayerEntity};
use super::relay::RiderLink;
use crate::protocol::{ControlChannel, MAX_STANDINGS, Standings, StandingsDiff, Wire};
use crate::roster::{InstanceInfo, PlayerAppearance, PlayerState};

type StartLink<'a> = (&'a RiderLink, &'a PlacedIn, &'a mut MessageSender<Wire<ServerControl>>);
type Active = (With<ClientOf>, With<Connected>, Without<Kicked>, Without<Leaving>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoundEvent {
    pub instance: u16,
    pub round: u16,
    pub state: InstanceState,
    pub level: ssx_net_proto::instance::Level,
    pub start: Option<(u32, Vec<u16>)>,
    pub standings: Vec<StandingRow>,
}

#[derive(Resource, Default)]
pub struct RoundLog {
    pub events: std::collections::VecDeque<RoundEvent>,
}

const MAX_ROUND_LOG: usize = 256;

pub(super) fn drive(
    instances: Option<ResMut<Instances>>,
    timeline: Res<LocalTimeline>,
    mut infos: Query<&mut InstanceInfo>,
    tables: Query<&Standings>,
    mut links: Query<StartLink, Active>,
    (mut log, mut stats): (ResMut<RoundLog>, ResMut<ServerStats>),
    mut commands: Commands,
) {
    let Some(mut instances) = instances else { return };
    let now = timeline.tick().0;
    for s in &mut instances.list {
        let changes = s.instance.tick(now);
        let (id, round, state) = (s.instance.id(), s.instance.round(), s.instance.state());
        if changes.state {
            if let Ok(mut info) = infos.get_mut(s.entity) {
                *info = InstanceInfo { id, spec: s.instance.spec().clone(), state, round };
            }
            let last = state == InstanceState::Results;
            let event = RoundEvent {
                instance: id,
                round,
                state,
                level: s.instance.spec().level.clone(),
                start: changes.start.as_ref().map(|(_, t, riders)| (*t, riders.clone())),
                standings: if last { s.instance.standings().to_vec() } else { Vec::new() },
            };
            if log.events.len() == MAX_ROUND_LOG {
                log.events.pop_front();
            }
            log.events.push_back(event);
        }
        if let Some((round, tick, riders)) = &changes.start {
            let start = Wire::control(&ServerControl::StartAt { round: *round, tick: *tick });
            for (link, placed, mut sender) in &mut links {
                if placed.0 == id && riders.contains(&link.rider().0) {
                    sender.send::<ControlChannel>(start.clone());
                    stats.starts_sent += 1;
                }
            }
        }
        if changes.standings
            && let Some(entity) = s.standings
            && let Ok(table) = tables.get(entity)
        {
            for diff in diffs(table, round, s.instance.standings()) {
                commands.entity(entity).apply_diff::<Standings>(diff);
            }
        }
    }
}

fn diffs(table: &Standings, round: u16, rows: &[StandingRow]) -> Vec<StandingsDiff> {
    let rows = &rows[..rows.len().min(MAX_STANDINGS)];
    let mut out = Vec::new();
    let old: &[StandingRow] = if table.round() == round {
        table.rows()
    } else {
        out.push(StandingsDiff::Round(round));
        &[]
    };
    for (slot, row) in rows.iter().enumerate() {
        if old.get(slot) != Some(row) {
            out.push(StandingsDiff::Set { slot: slot as u8, row: *row });
        }
    }
    if old.len() > rows.len() {
        out.push(StandingsDiff::Truncate(rows.len() as u8));
    }
    out
}

type Player<'a> = (&'a RiderLink, &'a PlacedIn, &'a PlayerEntity);

pub(super) fn sync_players(
    instances: Option<Res<Instances>>,
    links: Query<Player, With<ClientOf>>,
    players: Query<(Option<&PlayerState>, Option<&PlayerAppearance>)>,
    mut commands: Commands,
) {
    let Some(instances) = instances else { return };
    for (link, placed, player) in &links {
        let rider = link.rider().0;
        let Some(s) = instances.by_id(placed.0) else { continue };
        let (Some(status), Some(progress)) = (s.instance.status(rider), s.instance.progress(rider)) else { continue };
        let Ok((state, look)) = players.get(player.0) else { continue };
        let Ok(mut e) = commands.get_entity(player.0) else { continue };
        if state.map(|s| s.0) != Some(status) {
            e.insert(PlayerState(status));
        }
        match (progress.appearance(), look.map(|l| l.0)) {
            (Some(a), Some(b)) if a == b => {}
            (Some(a), _) => {
                e.insert(PlayerAppearance(a));
            }
            (None, Some(_)) => {
                e.remove::<PlayerAppearance>();
            }
            (None, None) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_replicon::prelude::Diffable;
    use ssx_net_proto::StandingState;

    fn row(rider: u16, clock_ms: u32) -> StandingRow {
        StandingRow {
            rider,
            state: StandingState::Racing,
            split: Some(1),
            clock_ms,
            points: 0,
            descent_mm: 0,
            tick: 0,
            fraction: 0,
            invalid: None,
        }
    }

    #[test]
    fn the_diffs_take_a_table_to_the_new_rows() {
        let mut table = Standings::new(1, 4);
        for rows in [
            vec![row(1, 10), row(2, 20), row(3, 30)],
            vec![row(2, 20), row(1, 25), row(3, 30)],
            vec![row(2, 20)],
            vec![row(2, 20), row(9, 1)],
        ] {
            for d in diffs(&table, 4, &rows) {
                table.apply_diff(&d).expect("a diff the table takes");
            }
            assert_eq!(table.rows(), rows.as_slice());
        }
        let next = vec![row(5, 1)];
        let d = diffs(&table, 5, &next);
        assert_eq!(d[0], StandingsDiff::Round(5));
        for d in d {
            table.apply_diff(&d).expect("a diff the table takes");
        }
        assert_eq!((table.round(), table.rows()), (5, next.as_slice()));
        assert!(diffs(&table, 5, &next).is_empty(), "nothing changed: no diff");
    }
}
