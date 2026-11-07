use std::collections::VecDeque;

use bevy_ecs::prelude::*;
use bevy_time::{Real, Time};
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::{MessageReceiver, MessageSender};
use ssx_gameplay::chat_rules::{ChatBudget, Verdict};
use ssx_net_proto::{ChatRefusal, ClientChat, ServerChat};

use super::ServerStats;
use super::guard::{KickReason, Kicked, Leaving, kick, leave};
use super::instance::PlacedIn;
use super::relay::RiderLink;
use super::session::PlayerSession;
use crate::protocol::{ChatChannel, Wire};

#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChatConfig {
    pub enabled: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Component, Clone, Debug)]
pub struct ChatLink(ChatBudget);

impl ChatLink {
    pub(super) fn new() -> Self {
        Self(ChatBudget::new())
    }
}

#[derive(Resource, Default)]
pub struct ChatOutbox(pub Vec<ServerChat>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatLogLine {
    pub rider: u16,
    pub key_id: [u8; 8],
    pub name: String,
    pub line: ClientChat,
}

const MAX_LOG_QUEUE: usize = 4096;

#[derive(Resource, Default)]
pub struct ChatLog {
    pub lines: VecDeque<ChatLogLine>,
    pub overflowed: u64,
}

impl ChatLog {
    fn push(&mut self, line: ChatLogLine) {
        if self.lines.len() == MAX_LOG_QUEUE {
            self.lines.pop_front();
            self.overflowed += 1;
        }
        self.lines.push_back(line);
    }
}

type Listening = (With<ClientOf>, Without<Kicked>, Without<Leaving>);
type Chatter<'a> = (
    Entity,
    &'a mut MessageReceiver<Wire<ClientChat>>,
    &'a RiderLink,
    &'a PlayerSession,
    &'a mut ChatLink,
    Option<&'a PlacedIn>,
);
type Listener<'a> = (Entity, &'a mut MessageSender<Wire<ServerChat>>, Option<&'a PlacedIn>);
type Placed = (With<ClientOf>, Without<Kicked>, Without<Leaving>, With<PlayerSession>);
type ChatWorld<'w> = (Res<'w, ChatConfig>, Res<'w, Time<Real>>);
type ChatBooks<'w> = (ResMut<'w, ChatOutbox>, ResMut<'w, ChatLog>, ResMut<'w, ServerStats>);

pub(super) fn receive_chat(
    mut chatters: Query<Chatter, Listening>,
    mut riders: Query<Listener, Placed>,
    (config, time): ChatWorld,
    (mut outbox, mut log, mut stats): ChatBooks,
    (gate, mut to_instance): (Option<Res<super::OperatorGate>>, ResMut<InstanceOutbox>),
    mut commands: Commands,
) {
    let now = time.elapsed();
    let mut passed: Vec<(ServerChat, Option<u16>)> = Vec::new();
    let mut refusals: Vec<(Entity, ChatRefusal)> = Vec::new();
    for (link, mut receiver, rider, session, mut budget, placed) in &mut chatters {
        for wire in receiver.receive() {
            stats.chat_received += 1;
            let line = match ClientChat::decode(wire.bytes()) {
                Ok(line) => line,
                Err(_) => {
                    kick(&mut commands, link, KickReason::MalformedChat);
                    break;
                }
            };
            match budget.0.take(now, config.enabled) {
                Verdict::Pass if let Some(mute) = gate.as_deref().and_then(|g| g.0.muted(&session.player)) => {
                    stats.chat_muted += 1;
                    let refusal = ChatRefusal::muted(mute.until, &mute.reason)
                        .unwrap_or(ChatRefusal::Muted { until: mute.until, reason: String::new() });
                    refusals.push((link, refusal));
                }
                Verdict::Pass => {
                    let from = rider.rider().0;
                    let message = match &line {
                        ClientChat::Text(text) => ServerChat::Line { rider: from, text: text.clone() },
                        ClientChat::Quick(index) => ServerChat::Quick { rider: from, index: *index },
                    };
                    passed.push((message, placed.map(|p| p.0)));
                    let mut key_id = [0u8; 8];
                    key_id.copy_from_slice(&session.id.0[..8]);
                    log.push(ChatLogLine { rider: from, key_id, name: session.name.clone(), line });
                }
                Verdict::Drop { notice, wait } => {
                    stats.chat_dropped += 1;
                    if notice {
                        let refusal = if config.enabled {
                            let ms = wait.as_millis().clamp(1, u128::from(ChatRefusal::MAX_WAIT_MS)) as u16;
                            ChatRefusal::Flood { wait_ms: ms }
                        } else {
                            ChatRefusal::Disabled
                        };
                        refusals.push((link, refusal));
                    }
                }
                Verdict::Flood => {
                    stats.chat_floods += 1;
                    leave(&mut commands, link, KickReason::ChatFlood, "too many chat lines", now);
                    break;
                }
            }
        }
    }
    passed.extend(outbox.0.drain(..).map(|line| (line, None)));
    passed.extend(to_instance.0.drain(..).map(|(instance, line)| (line, Some(instance))));
    for (line, instance) in &passed {
        let wire = Wire::server_chat(line);
        for (_, mut sender, placed) in &mut riders {
            if instance.is_some_and(|i| placed.is_none_or(|p| p.0 != i)) {
                continue;
            }
            sender.send::<ChatChannel>(wire.clone());
            stats.chat_sent += 1;
        }
    }
    for (to, refusal) in refusals {
        if let Ok((_, mut sender, _)) = riders.get_mut(to) {
            sender.send::<ChatChannel>(Wire::server_chat(&ServerChat::Refused(refusal)));
        }
    }
}

#[derive(Resource, Default)]
pub struct InstanceOutbox(pub Vec<(u16, ServerChat)>);

pub(super) fn drop_leaving_chat(mut leaving: Query<&mut MessageReceiver<Wire<ClientChat>>, With<Leaving>>) {
    for mut receiver in &mut leaving {
        receiver.receive().for_each(drop);
    }
}
