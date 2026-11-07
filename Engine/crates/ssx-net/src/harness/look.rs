use core::time::Duration;

use ssx_net_proto::{Appearance, ClientControl, PlayerStatus, RiderId};

use super::Peers;
use super::hostile::{sample, send_wire};
use super::instance::{FRAME, riders, run};
use crate::client::{ClientRoster, ControlOutbox, RemotePoses};
use crate::protocol::Wire;
use crate::server::{KickReason, ServerStats};

pub const NEW_CHARACTER: u32 = ssx_formats::attrib::fnv1(b"zoe");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookCase {
    ChangeOfLook,
    LoadedWhileAway,
    LevelLeftTwice,
}

#[derive(Clone, Debug, Default)]
pub struct LookOutcome {
    pub kicks: Vec<KickReason>,
    pub rider0_connected: bool,
    pub away: Option<(Option<PlayerStatus>, Option<Appearance>)>,
    pub end: Option<(Option<PlayerStatus>, Option<Appearance>)>,
    pub relayed_while_away: u64,
    pub relayed_after: u64,
    pub dropped_before_look: u64,
}

fn seen(p: &Peers) -> Option<(Option<PlayerStatus>, Option<Appearance>)> {
    let own0 = p.clients[0].world().resource::<ClientRoster>().own.map(|(r, _)| r)?;
    let r = p.clients[1].world().resource::<ClientRoster>().players.get(&own0)?;
    Some((r.status, r.appearance))
}

fn relayed(p: &Peers, rider: RiderId) -> u64 {
    p.clients[1].world().resource::<RemotePoses>().riders.get(&rider).map_or(0, |r| r.samples)
}

fn control(p: &mut Peers, c: usize, messages: &[ClientControl]) {
    p.clients[c].world_mut().resource_mut::<ControlOutbox>().0.extend_from_slice(messages);
}

pub fn run_look(case: LookCase) -> Result<LookOutcome, String> {
    let (mut peers, mut none) = riders(false)?;
    let mut o = LookOutcome::default();
    let rider0 = peers.clients[0].world().resource::<ClientRoster>().own.map(|(r, _)| r).ok_or("rider 0 unplaced")?;
    let kicked = |p: &mut Peers| p.server.world().resource::<ServerStats>().kicked > 0;
    match case {
        LookCase::ChangeOfLook => {
            control(&mut peers, 0, &[ClientControl::LevelLeft]);
            let away = |p: &mut Peers| seen(p).is_some_and(|(s, a)| s == Some(PlayerStatus::Away) && a.is_none());
            if !run(&mut peers, &mut none, true, Duration::from_secs(3), away)? {
                return Err(format!("rider 1 never saw rider 0 away: {:?}", seen(&peers)));
            }
            o.away = seen(&peers);
            let before = relayed(&peers, rider0);
            for k in 0..10 {
                let pose = sample(&peers, 100 + k);
                send_wire(&mut peers, 0, Wire::pose(&pose));
                run(&mut peers, &mut none, true, FRAME * 2, |_| false)?;
            }
            run(&mut peers, &mut none, true, Duration::from_millis(500), |_| false)?;
            o.relayed_while_away = relayed(&peers, rider0) - before;
            let look = Appearance::new(NEW_CHARACTER, 0).map_err(|e| e.to_string())?;
            control(
                &mut peers,
                0,
                &[ClientControl::Appearance(look), ClientControl::Loaded { round: 0 }, super::ready(0)],
            );
            if !run(&mut peers, &mut none, true, Duration::from_secs(3), |p| p.riding(0))? {
                return Err("rider 0 did not ride again".into());
            }
            let back = relayed(&peers, rider0);
            for k in 10..20 {
                let pose = sample(&peers, 100 + k);
                send_wire(&mut peers, 0, Wire::pose(&pose));
                run(&mut peers, &mut none, true, FRAME * 2, |_| false)?;
            }
            run(&mut peers, &mut none, true, Duration::from_millis(500), |_| false)?;
            o.relayed_after = relayed(&peers, rider0) - back;
        }
        LookCase::LoadedWhileAway => {
            control(&mut peers, 0, &[ClientControl::LevelLeft, ClientControl::Loaded { round: 0 }]);
            run(&mut peers, &mut none, true, Duration::from_secs(3), kicked)?;
        }
        LookCase::LevelLeftTwice => {
            control(&mut peers, 0, &[ClientControl::LevelLeft, ClientControl::LevelLeft]);
            run(&mut peers, &mut none, true, Duration::from_secs(3), kicked)?;
        }
    }
    run(&mut peers, &mut none, true, Duration::from_millis(500), |_| false)?;
    let stats = peers.server.world().resource::<ServerStats>();
    o.kicks = stats.kick_reasons.iter().copied().collect();
    o.dropped_before_look = stats.poses_before_look;
    o.rider0_connected = peers.connected(0);
    o.end = seen(&peers);
    Ok(o)
}
