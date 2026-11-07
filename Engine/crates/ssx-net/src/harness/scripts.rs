use core::time::Duration;

use bevy_ecs::prelude::*;
use lightyear::prelude::MessageSender;
use ssx_net_proto::GoodbyeReason;
use ssx_net_proto::scripts::{MAX_ARGS, ScriptBody, ScriptItem};

use super::instance::{FRAME, run};
use super::{Io, Peers};
use crate::client::{RemotePoses, ScriptReceived, ScriptSend, ServerSaid};
use crate::protocol::{ScriptChannel, ScriptDown, ScriptUp, Wire};
use crate::server::{KickReason, ScriptInbox, ScriptLink, ScriptNetStats, ScriptSessions, ServerStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptCase {
    RoundTrip,
    Burst,
    Flood,
    Malformed,
    ServerKind,
    ServerMalformed,
    ClientKind,
    Backlog,
}

#[derive(Clone, Debug, Default)]
pub struct ScriptOutcome {
    pub kicks: Vec<KickReason>,
    pub refusal: Option<String>,
    pub goodbye: Option<GoodbyeReason>,
    pub client0_connected: bool,
    pub client1_connected: bool,
    pub server_got: Vec<(u32, ScriptItem)>,
    pub client_got: Vec<ScriptItem>,
    pub stats: ScriptNetStats,
    pub joined: Vec<(u32, String)>,
    pub left: Vec<u32>,
    pub notes: Vec<String>,
    pub server_fragment_bytes: usize,
    pub client_fragment_bytes: usize,
}

fn client_fragment_bytes(peers: &mut Peers) -> usize {
    let world = peers.clients[0].world_mut();
    let mut transports = world.query::<&mut lightyear::prelude::Transport>();
    transports
        .iter_mut(world)
        .map(|mut t| t.channel_receives_mut().map(|c| c.fragment_bytes_in_flight()).sum::<usize>())
        .max()
        .unwrap_or(0)
}

fn event(name: &str, args: Vec<u8>) -> ScriptItem {
    ScriptItem { package: Some("scoreboard".into()), body: ScriptBody::Event { name: name.into(), source: 1, args } }
}

fn client_raw(peers: &mut Peers, bytes: Vec<u8>) -> Result<(), String> {
    let entity = peers.client_entities[0];
    let world = peers.clients[0].world_mut();
    let mut sender = world.get_mut::<MessageSender<Wire<ScriptUp>>>(entity).ok_or("no script sender")?;
    sender.send::<ScriptChannel>(Wire::from_raw(bytes));
    Ok(())
}

fn server_raw(peers: &mut Peers, bytes: Vec<u8>) -> Result<(), String> {
    let link = peers.server_link(0).ok_or("no link of client 0")?;
    let mut sender =
        peers.server.world_mut().get_mut::<MessageSender<Wire<ScriptDown>>>(link).ok_or("no script sender")?;
    sender.send::<ScriptChannel>(Wire::from_raw(bytes));
    Ok(())
}

fn whole(item: &ScriptItem) -> Result<Vec<u8>, String> {
    let b = item.encode().map_err(|e| e.to_string())?;
    let mut out = vec![3u8];
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(&b);
    Ok(out)
}

fn server_queue(peers: &mut Peers, item: &ScriptItem) -> Result<(), String> {
    let link = peers.server_link(0).ok_or("no link of client 0")?;
    let bytes = item.encode().map_err(|e| e.to_string())?;
    let mut l = peers.server.world_mut().get_mut::<ScriptLink>(link).ok_or("no script link")?;
    let latent = match &item.body {
        ssx_net_proto::scripts::ScriptBody::LatentChunk { bytes, .. } => bytes.len(),
        _ => 0,
    };
    l.queue(bytes.into(), latent, Duration::ZERO).map_err(|e| format!("{e:?}"))
}

pub fn run_scripts(case: ScriptCase) -> Result<ScriptOutcome, String> {
    use ScriptCase as C;
    let mut peers = Peers::new(Io::Udp, None, FRAME)?;
    peers.add_client(1)?;
    peers.add_client(2)?;
    let mut none = None;
    let up = |p: &mut Peers| (0..2).all(|c| p.connected(c) && p.synced(c) && p.knows(c, 2));
    if !run(&mut peers, &mut none, true, Duration::from_secs(10), up)? {
        return Err("the two clients did not connect".into());
    }
    let mut o = ScriptOutcome::default();
    let mut seconds = 2.0;
    let send = |peers: &mut Peers, items: Vec<ScriptItem>| -> Result<(), String> {
        let mut s = peers.clients[0].world_mut().resource_mut::<ScriptSend>();
        items.iter().try_for_each(|i| s.push(i))
    };
    match case {
        C::RoundTrip => {
            let items = vec![event("a", vec![1]), event("big", vec![7; MAX_ARGS]), event("c", vec![])];
            send(&mut peers, items)?;
            for item in [event("x", vec![2]), event("y", vec![9; MAX_ARGS]), event("z", vec![])] {
                server_queue(&mut peers, &item)?;
            }
        }
        C::Burst => send(&mut peers, (0..260).map(|k| event("tick", vec![k as u8])).collect())?,
        C::Flood => send(&mut peers, (0..400).map(|k| event("tick", vec![k as u8])).collect())?,
        C::Malformed => client_raw(&mut peers, vec![3, 0, 200, 1, 2, 3])?,
        C::ServerKind => {
            let body = ScriptBody::Created { element: 5, kind: "rider".into(), name: String::new(), parent: 1 };
            client_raw(&mut peers, whole(&ScriptItem { package: None, body })?)?;
        }
        C::ServerMalformed => server_raw(&mut peers, vec![0, 0, 1, 9])?,
        C::ClientKind => {
            let body = ScriptBody::DataWrite { element: 5, key: "k".into(), value: None };
            server_raw(&mut peers, whole(&ScriptItem { package: None, body })?)?;
        }
        C::Backlog => {
            for k in 0..64u32 {
                server_queue(&mut peers, &event("chunk", vec![(k % 251) as u8; MAX_ARGS]))?;
            }
            seconds = 20.0;
        }
    }
    let target = match case {
        C::Backlog => 64,
        C::RoundTrip => 3,
        _ => usize::MAX,
    };
    let mut client_got = Vec::new();
    let (mut server_fragments, mut client_fragments) = (0, 0);
    let wait = |p: &mut Peers| {
        server_fragments = server_fragments.max(super::hostile::fragment_bytes_in_flight(p));
        client_fragments = client_fragments.max(client_fragment_bytes(p));
        let (items, _) = p.clients[0].world_mut().resource_mut::<ScriptReceived>().take();
        client_got.extend(items);
        p.server.world().resource::<ServerStats>().kicked > 0
            || p.clients[0].world().resource::<RemotePoses>().refused.is_some()
            || client_got.len() >= target
    };
    let ended = run(&mut peers, &mut none, true, Duration::from_secs_f64(seconds), wait)?;
    o.notes.push(format!("the case ended within {seconds} s: {ended}"));
    (o.server_fragment_bytes, o.client_fragment_bytes) = (server_fragments, client_fragments);
    run(&mut peers, &mut none, true, Duration::from_secs(1), |_| false)?;
    let (items, _) = peers.clients[0].world_mut().resource_mut::<ScriptReceived>().take();
    client_got.extend(items);
    o.client_got = client_got;
    let stats = peers.server.world().resource::<ServerStats>().clone();
    o.kicks = stats.kick_reasons.iter().copied().collect();
    o.refusal = peers.clients[0].world().resource::<RemotePoses>().refused.clone();
    o.goodbye = peers.clients[0].world().resource::<ServerSaid>().goodbye.as_ref().map(|(r, _)| *r);
    o.client0_connected = peers.connected(0);
    o.client1_connected = peers.connected(1);
    let w = peers.server.world_mut();
    o.server_got = std::mem::take(&mut w.resource_mut::<ScriptInbox>().0);
    o.stats = w.resource::<ScriptNetStats>().clone();
    let mut book = w.resource_mut::<ScriptSessions>();
    o.joined = book.joined.drain(..).map(|(_, c, n)| (c, n)).collect();
    o.left = book.left.drain(..).map(|(c, _)| c).collect();
    Ok(o)
}
