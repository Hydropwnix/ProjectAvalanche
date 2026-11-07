use std::collections::BTreeMap;

use ssx_net::server::{Instances, ReportEvent, RoundEvent};
use ssx_net_proto::{InstanceState, PlayerStatus, ReportKind, StandingState};
use ssx_script::host::ClientId;
use ssx_script::host::server::{InstanceFrame, PlayerView, RunView, ServerEvent, StandingFrame};

pub(super) struct LinkView {
    pub client: u32,
    pub name: String,
    pub instance: Option<u16>,
    pub rider: Option<u16>,
    pub position: Option<[f32; 3]>,
    pub ping_ms: Option<u16>,
}

fn status_name(s: PlayerStatus) -> &'static str {
    match s {
        PlayerStatus::Joining => "joining",
        PlayerStatus::Loading => "loading",
        PlayerStatus::Intro => "intro",
        PlayerStatus::Riding => "riding",
        PlayerStatus::Away => "away",
        PlayerStatus::Spectating => "spectating",
        PlayerStatus::Finished => "finished",
    }
}

fn standing_name(s: StandingState) -> &'static str {
    match s {
        StandingState::Racing => "racing",
        StandingState::Finished => "finished",
        StandingState::Failed => "failed",
        StandingState::Left => "left",
        StandingState::Invalidated => "invalidated",
    }
}

pub(super) fn instances(
    list: &Instances,
    names: &BTreeMap<(u16, u16), String>,
    tick: u32,
    tick_ms: f64,
) -> Vec<InstanceFrame> {
    list.list
        .iter()
        .map(|s| {
            let i = &s.instance;
            let clock_ms = match (i.state(), i.start_tick()) {
                (InstanceState::Running, Some(t)) => f64::from(tick.saturating_sub(t)) * tick_ms,
                _ => 0.0,
            };
            let standings = i
                .standings()
                .iter()
                .map(|r| StandingFrame {
                    rider: r.rider,
                    name: names.get(&(i.id(), r.rider)).cloned().unwrap_or_default(),
                    state: standing_name(r.state),
                    clock_ms: r.clock_ms,
                    points: r.points,
                    split: r.split,
                })
                .collect();
            InstanceFrame {
                id: i.id(),
                mode: i.spec().mode.name().to_string(),
                drop: level_id(&i.spec().level),
                state: i.state().name(),
                round: i.round(),
                clock_ms,
                standings,
            }
        })
        .collect()
}

pub(super) fn players(list: &Instances, links: Vec<LinkView>) -> Vec<PlayerView> {
    links
        .into_iter()
        .map(|l| {
            let placed = l.instance.zip(l.rider).and_then(|(i, r)| list.by_id(i).map(|s| (&s.instance, r)));
            let progress = placed.and_then(|(i, r)| i.progress(r));
            let status = placed.and_then(|(i, r)| i.status(r));
            let run = placed.and_then(|(i, r)| i.member(r)).and_then(|m| {
                m.run.state.map(|state| RunView {
                    split: m.run.split,
                    clock_ms: m.run.clock_ms,
                    points: m.run.points,
                    state: standing_name(state),
                })
            });
            PlayerView {
                client: ClientId(l.client),
                name: l.name,
                instance: l.instance,
                rider: l.rider,
                progress: progress.map_or("joined", |p| p.name()),
                status: status.map_or("joining", status_name),
                ping_ms: l.ping_ms.map_or(0, u32::from),
                position: l.position,
                velocity: None,
                run,
            }
        })
        .collect()
}

fn level_id(level: &ssx_net_proto::instance::Level) -> String {
    match level {
        ssx_net_proto::instance::Level::Drop(name) => format!("ssx:drop/{name}"),
        ssx_net_proto::instance::Level::Map { id, .. } => id.clone(),
    }
}

pub(super) fn round_events<'a>(log: impl IntoIterator<Item = &'a RoundEvent>, out: &mut Vec<ServerEvent>) {
    for e in log {
        match e.state {
            InstanceState::Running => {
                out.push(ServerEvent::RoundStart { instance: e.instance, round: e.round, drop: level_id(&e.level) })
            }
            InstanceState::Results => out.push(ServerEvent::RoundEnd { instance: e.instance, round: e.round }),
            _ => {}
        }
    }
}

pub(super) fn report_event(r: &ReportEvent, client: u32, out: &mut Vec<ServerEvent>) {
    let client = ClientId(client);
    match (r.kind, r.split) {
        (ReportKind::Split, Some(split)) => out.push(ServerEvent::Checkpoint { client, split, clock_ms: r.clock_ms }),
        (ReportKind::Finish, _) => {
            out.push(ServerEvent::Finish { client, clock_ms: r.clock_ms, points: r.points, failed: false })
        }
        (ReportKind::Failed, _) => {
            out.push(ServerEvent::Finish { client, clock_ms: r.clock_ms, points: r.points, failed: true })
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use ssx_net_proto::instance::Level;

    #[test]
    fn a_level_is_named_by_its_content_id() {
        let drop = Level::drop("exp_alps_vorassay_drop_3").expect("a drop");
        assert_eq!(super::level_id(&drop), "ssx:drop/exp_alps_vorassay_drop_3");
        let map = Level::map("slope:map/course", 0).expect("a map");
        assert_eq!(super::level_id(&map), "slope:map/course");
    }
}
