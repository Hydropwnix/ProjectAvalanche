use core::time::Duration;

use bevy_ecs::prelude::*;
use lightyear::prelude::{MessageSender, ReliableLimits};
use ssx_net_proto::chat::{MAX_CHAT_CHARS, MAX_CLIENT_CHAT_SIZE};
use ssx_net_proto::{ChatRefusal, ClientChat, GoodbyeReason, ServerChat};

use super::instance::{CHARACTER, FRAME, run};
use super::{Io, Peers};
use crate::client::{ChatBody, ChatFrom, ChatInbox, ChatSend, RemotePoses, ServerSaid};
use crate::protocol::{ChatChannel, Wire};
use crate::server::{ChatConfig, ChatLog, ChatLogLine, ChatOutbox, KickReason, ServerStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatCase {
    Conversation,
    ShortFlood,
    SustainedFlood,
    Disabled,
    InvalidUtf8,
    ControlCharacter,
    OverCharacters,
    OverWire,
    Huge,
    UnknownQuickLine,
    WrongDirection,
    ServerMalformed,
    ServerControlCharacters,
    ServerOversized,
    ServerUnknownRider,
    ServerWrongDirection,
    ServerFlood,
}

#[derive(Clone, Debug, Default)]
pub struct ChatOutcome {
    pub kicks: Vec<KickReason>,
    pub refusal: Option<String>,
    pub goodbye: Option<GoodbyeReason>,
    pub rider0_connected: bool,
    pub rider1_connected: bool,
    pub lines: [Vec<(ChatFrom, ChatBody)>; 2],
    pub refusals: Vec<ChatRefusal>,
    pub chat_flags: [Option<bool>; 2],
    pub log: Vec<ChatLogLine>,
    pub stats: ServerStats,
    pub inbox: (u64, u64, u64, usize),
    pub notes: Vec<String>,
}

fn riders(enabled: bool) -> Result<Peers, String> {
    let mut peers = Peers::new(Io::Udp, None, FRAME)?;
    peers.server.world_mut().insert_resource(ChatConfig { enabled });
    peers.add_client(1)?;
    peers.add_client(2)?;
    let mut none = None;
    let up = |p: &mut Peers| (0..2).all(|c| p.connected(c) && p.synced(c) && p.knows(c, 2));
    if !run(&mut peers, &mut none, true, Duration::from_secs(10), up)? {
        return Err("the two riders did not connect".into());
    }
    for c in 0..2 {
        peers.dress(c, CHARACTER)?;
    }
    if !run(&mut peers, &mut none, true, Duration::from_secs(10), |p| p.riding(0) && p.riding(1))? {
        return Err("the two riders did not get ready".into());
    }
    Ok(peers)
}

fn client_raw(peers: &mut Peers, bytes: Vec<u8>) -> Result<(), String> {
    let entity = peers.client_entities[0];
    let world = peers.clients[0].world_mut();
    let mut sender = world.get_mut::<MessageSender<Wire<ClientChat>>>(entity).ok_or("no chat sender")?;
    sender.send::<ChatChannel>(Wire::from_raw(bytes));
    Ok(())
}

fn server_raw(peers: &mut Peers, bytes: Vec<u8>) -> Result<(), String> {
    let link = peers.server_link(0).ok_or("no link of rider 0")?;
    let mut sender =
        peers.server.world_mut().get_mut::<MessageSender<Wire<ServerChat>>>(link).ok_or("no chat sender")?;
    sender.send::<ChatChannel>(Wire::from_raw(bytes));
    Ok(())
}

fn framed(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![kind];
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn run_chat(case: ChatCase) -> Result<ChatOutcome, String> {
    use ChatCase as C;
    let mut peers = riders(case != C::Disabled)?;
    let mut none = None;
    let mut o = ChatOutcome::default();
    let mut seconds = 3.0;
    let line = |t: &str| ClientChat::text(t).map_err(|e| e.to_string());
    match case {
        C::Conversation => {
            let lines = vec![line("hello <b>&</b> all")?, ClientChat::quick(3).map_err(|e| e.to_string())?];
            peers.clients[0].world_mut().resource_mut::<ChatSend>().0.extend(lines);
            let say = ServerChat::server("the server speaks").map_err(|e| e.to_string())?;
            peers.server.world_mut().resource_mut::<ChatOutbox>().0.push(say);
        }
        C::ShortFlood | C::SustainedFlood => {
            let n = if case == C::ShortFlood { 8 } else { 40 };
            let lines = (0..n).map(|k| line(&format!("line {k}"))).collect::<Result<Vec<_>, _>>()?;
            peers.clients[0].world_mut().resource_mut::<ChatSend>().0.extend(lines);
            if case == C::SustainedFlood {
                seconds = 4.0;
            }
        }
        C::Disabled => {
            let l = line("anyone?")?;
            peers.clients[0].world_mut().resource_mut::<ChatSend>().0.push(l);
        }
        C::InvalidUtf8 => client_raw(&mut peers, framed(1, &[b'a', 0xC3, 0x28]))?,
        C::ControlCharacter => client_raw(&mut peers, framed(1, b"one\ntwo"))?,
        C::OverCharacters => client_raw(&mut peers, framed(1, &[b'x'; MAX_CHAT_CHARS + 1]))?,
        C::OverWire => client_raw(&mut peers, framed(1, &vec![b'x'; MAX_CLIENT_CHAT_SIZE + 1 - 3]))?,
        C::Huge => client_raw(&mut peers, framed(1, &vec![b'x'; 64 * 1024 - 3]))?,
        C::UnknownQuickLine => client_raw(&mut peers, framed(2, &[ssx_net_proto::chat::QUICK_LINES]))?,
        C::WrongDirection => {
            let entity = peers.client_entities[0];
            let world = peers.clients[0].world_mut();
            world.entity_mut(entity).insert(MessageSender::<Wire<ServerChat>>::default());
            let wire = Wire::server_chat(&ServerChat::server("forged").map_err(|e| e.to_string())?);
            if let Some(mut s) = world.get_mut::<MessageSender<Wire<ServerChat>>>(entity) {
                s.send::<ChatChannel>(wire);
            }
        }
        C::ServerMalformed => server_raw(&mut peers, framed(1, &[0, 1, 0xFF, 0xFE]))?,
        C::ServerControlCharacters => server_raw(&mut peers, framed(3, "a\u{7}\u{1B}[31mred".as_bytes()))?,
        C::ServerOversized => server_raw(&mut peers, framed(3, &vec![b'x'; 4000]))?,
        C::ServerUnknownRider => {
            let wire = ServerChat::Line { rider: 999, text: "who am I".into() };
            server_raw(&mut peers, wire.encode())?;
        }
        C::ServerWrongDirection => {
            let link = peers.server_link(0).ok_or("no link of rider 0")?;
            let world = peers.server.world_mut();
            world.entity_mut(link).insert(MessageSender::<Wire<ClientChat>>::default());
            if let Some(mut s) = world.get_mut::<MessageSender<Wire<ClientChat>>>(link) {
                s.send::<ChatChannel>(Wire::client_chat(&line("forged")?));
            }
        }
        C::ServerFlood => {
            let huge = ReliableLimits { max_unacked_messages: 1 << 24, max_unacked_bytes: 1 << 34 };
            peers.server.world_mut().insert_resource(huge);
            let end = std::time::Instant::now() + Duration::from_secs(3);
            let (mut sent, mut frames) = (0u64, 0u32);
            let started = std::time::Instant::now();
            while std::time::Instant::now() < end {
                let due = (started.elapsed().as_secs_f64() * 10_000.0) as u64;
                let n = due.saturating_sub(sent);
                let mut out = peers.server.world_mut().resource_mut::<ChatOutbox>();
                for k in 0..n {
                    out.0.push(ServerChat::Server(format!("flood {}", sent + k)));
                }
                sent += n;
                frames += 1;
                run(&mut peers, &mut none, true, FRAME, |_| false)?;
            }
            o.notes.push(format!("the server sent {sent} lines in {frames} frames"));
            seconds = 2.0;
        }
    }
    let wait = |p: &mut Peers| {
        p.server.world().resource::<ServerStats>().kicked > 0
            || p.clients[0].world().resource::<RemotePoses>().refused.is_some()
    };
    let ended = run(&mut peers, &mut none, true, Duration::from_secs_f64(seconds), wait)?;
    o.notes.push(format!("the case ended within {seconds} s: {ended}"));
    run(&mut peers, &mut none, true, Duration::from_secs(1), |_| false)?;
    let stats = peers.server.world().resource::<ServerStats>().clone();
    o.kicks = stats.kick_reasons.iter().copied().collect();
    o.stats = stats;
    o.refusal = peers.clients[0].world().resource::<RemotePoses>().refused.clone();
    o.goodbye = peers.clients[0].world().resource::<ServerSaid>().goodbye.as_ref().map(|(r, _)| *r);
    o.rider0_connected = peers.connected(0);
    o.rider1_connected = peers.connected(1);
    for c in 0..2 {
        let inbox = peers.clients[c].world().resource::<ChatInbox>();
        o.lines[c] = inbox.lines.iter().map(|l| (l.from, l.body.clone())).collect();
        o.chat_flags[c] = peers.clients[c].world().resource::<ServerSaid>().chat;
    }
    let inbox = peers.clients[0].world().resource::<ChatInbox>();
    o.refusals = inbox.refusals.iter().map(|(r, _)| r.clone()).collect();
    o.inbox = (inbox.received, inbox.unknown_riders, inbox.overflowed, inbox.lines.len());
    o.log = peers.server.world().resource::<ChatLog>().lines.iter().cloned().collect();
    Ok(o)
}
