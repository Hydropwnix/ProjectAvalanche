use bevy::prelude::*;
use ssx_net::ServerStats;
use ssx_net::server::{Instances, PlayerSession, RiderLink, RoundLog};

pub fn status(world: &World, stats: &ServerStats, out: &mut Vec<String>) {
    if let Some(instances) = world.get_resource::<Instances>() {
        for s in &instances.list {
            let i = &s.instance;
            let watching = i.members().filter(|(r, _)| i.status(*r) == Some(ssx_net_proto::PlayerStatus::Spectating));
            out.push(format!(
                "instance {}: {} on {}, {} round {}{}; {} players ({} watching), standings {}",
                i.id(),
                i.spec().mode.name(),
                i.spec().level,
                i.state().name(),
                i.round(),
                i.start_tick().map_or(String::new(), |t| format!(", start tick {t}")),
                i.len(),
                watching.count(),
                i.standings().iter().map(|r| r.rider.to_string()).collect::<Vec<_>>().join(" "),
            ));
        }
    }
    out.push(format!(
        "rounds: starts sent {}, control messages of an older round dropped {}",
        stats.starts_sent, stats.control_stale
    ));
    crate::rules::status(stats, out);
}

pub fn log_rounds(mut log: ResMut<RoundLog>, sessions: Query<(&PlayerSession, &RiderLink)>) {
    let name = |rider: u16| {
        sessions
            .iter()
            .find(|(_, r)| r.rider().0 == rider)
            .map_or_else(|| format!("rider {rider}"), |(s, _)| s.name.clone())
    };
    for e in log.events.drain(..) {
        let head = format!("instance {} round {}: {} on {}", e.instance, e.round, e.state.name(), e.level);
        match (&e.start, e.standings.is_empty()) {
            (Some((tick, riders)), _) => {
                let riders: Vec<String> = riders.iter().map(|&r| name(r)).collect();
                info!("{head}; the start at tick {tick}; riders: {}", riders.join(", "));
            }
            (None, false) => {
                let rows: Vec<String> = e
                    .standings
                    .iter()
                    .enumerate()
                    .map(|(k, r)| {
                        format!(
                            "{}. {} {:?} {} ms {} pts {} mm",
                            k + 1,
                            name(r.rider),
                            r.state,
                            r.clock_ms,
                            r.points,
                            r.descent_mm
                        )
                    })
                    .collect();
                info!("{head}; the standings: {}", rows.join("; "));
            }
            (None, true) => info!("{head}"),
        }
    }
}
