use core::time::Duration;

use super::instance::{CHARACTER, FRAME, run};
use super::{Conditions, Io, Peers};
use crate::client::ClientRoster;

#[derive(Clone, Debug, Default)]
pub struct PingOutcome {
    pub first: [Option<u16>; 2],
    pub settled: [Option<(u16, u16)>; 2],
    pub lost: u32,
}

pub fn run_pings(one_way: Duration, settle: Duration, seconds: Duration) -> Result<PingOutcome, String> {
    let conditions = Conditions { latency: one_way, jitter: Duration::ZERO, loss: 0.0 };
    let mut peers = Peers::new(Io::Udp, Some(conditions), FRAME)?;
    let mut none = None;
    peers.add_client(1)?;
    peers.add_client(2)?;
    let up = |p: &mut Peers| (0..2).all(|c| p.connected(c) && p.synced(c) && p.knows(c, 2));
    if !run(&mut peers, &mut none, true, Duration::from_secs(10), up)? {
        return Err("the two riders did not connect".into());
    }
    let mut o = PingOutcome::default();
    let watch = |p: &mut Peers, o: &mut PingOutcome, settled: bool| {
        let roster = p.clients[0].world().resource::<ClientRoster>();
        for (rider, player) in &roster.players {
            let k = usize::from(rider.0);
            if k >= 2 {
                continue;
            }
            match player.ping_ms {
                Some(ms) => {
                    o.first[k].get_or_insert(ms);
                    if settled {
                        let (lo, hi) = o.settled[k].get_or_insert((ms, ms));
                        (*lo, *hi) = ((*lo).min(ms), (*hi).max(ms));
                    }
                }
                None if o.first[k].is_some() => o.lost += 1,
                None => {}
            }
        }
    };
    for c in 0..2 {
        peers.dress(c, CHARACTER)?;
    }
    let ready = |p: &mut Peers| p.riding(0) && p.riding(1);
    if !run(&mut peers, &mut none, true, Duration::from_secs(10), |p| {
        watch(p, &mut o, false);
        ready(p)
    })? {
        return Err("the two riders did not get ready".into());
    }
    let start = std::time::Instant::now();
    run(&mut peers, &mut none, true, seconds, |p| {
        watch(p, &mut o, start.elapsed() >= settle);
        false
    })?;
    Ok(o)
}
