use std::collections::VecDeque;

use bevy_ecs::prelude::*;
use lightyear::prelude::{Disconnect, MessageReceiver, MessageSender, Transport};
use lightyear::transport::channel::ChannelKind;
use ssx_net_proto::scripts::{Assembler, Direction, Outgoing, QueueFull, ScriptItem};

use super::{Connection, RemotePoses};
use crate::protocol::{ScriptChannel, ScriptDown, ScriptUp, Wire};
use crate::server::{HELD_BYTES, HELD_MESSAGES, QUEUE_BYTES};

#[derive(Resource, Default)]
pub struct ScriptReceived {
    pub items: VecDeque<ScriptItem>,
    held: usize,
    asm: Assembler,
    pub wires: u64,
    pub received: u64,
}

impl ScriptReceived {
    pub fn take(&mut self) -> (Vec<ScriptItem>, usize) {
        (self.items.drain(..).collect(), std::mem::take(&mut self.held))
    }

    pub fn put_back(&mut self, items: Vec<ScriptItem>, bytes: usize) {
        for item in items.into_iter().rev() {
            self.items.push_front(item);
        }
        self.held += bytes;
    }
}

#[derive(Resource)]
pub struct ScriptSend {
    out: Outgoing,
    pub wires: u64,
    pub bytes: u64,
}

impl Default for ScriptSend {
    fn default() -> Self {
        Self { out: Outgoing::new(QUEUE_BYTES), wires: 0, bytes: 0 }
    }
}

impl ScriptSend {
    pub fn push(&mut self, item: &ScriptItem) -> Result<(), String> {
        if !item.body.from_client() {
            return Err("a kind the client does not send".into());
        }
        let bytes = item.encode().map_err(|e| e.to_string())?;
        self.out.push(bytes).map_err(|QueueFull { held, cap }| format!("{held} bytes wait to be sent (at most {cap})"))
    }

    pub fn held(&self) -> usize {
        self.out.held()
    }
}

pub(super) fn receive(
    mut links: Query<(Entity, &mut MessageReceiver<Wire<ScriptDown>>), Connection>,
    mut wrong: Query<(Entity, &mut MessageReceiver<Wire<ScriptUp>>), Connection>,
    mut got: ResMut<ScriptReceived>,
    mut poses: ResMut<RemotePoses>,
    mut commands: Commands,
) {
    for (entity, mut w) in &mut wrong {
        if w.receive().count() > 0 {
            poses.refused.get_or_insert_with(|| "the server sent a client-to-server script message".into());
            commands.trigger(Disconnect { entity });
        }
    }
    for (entity, mut receiver) in &mut links {
        let got = &mut *got;
        let mut refused = None;
        'wires: for wire in receiver.receive() {
            got.wires += 1;
            let items = match got.asm.take(wire.bytes()) {
                Ok(items) => items,
                Err(e) => {
                    refused = Some(format!("the server sent a malformed script message: {e}"));
                    break;
                }
            };
            for bytes in items {
                match ScriptItem::decode(&bytes, Direction::ToClient) {
                    Ok(item) => {
                        got.held += bytes.len();
                        if got.held > QUEUE_BYTES {
                            refused = Some(format!("the server's script messages wait beyond {QUEUE_BYTES} bytes"));
                            break 'wires;
                        }
                        got.received += 1;
                        got.items.push_back(item);
                    }
                    Err(e) => {
                        refused = Some(format!("the server sent a malformed script item: {e}"));
                        break 'wires;
                    }
                }
            }
        }
        if let Some(r) = refused
            && poses.refused.is_none()
        {
            poses.refused = Some(r);
            commands.trigger(Disconnect { entity });
        }
    }
}

type Sending<'a> = (&'a mut MessageSender<Wire<ScriptUp>>, &'a Transport);

pub(super) fn send(mut out: ResMut<ScriptSend>, mut links: Query<Sending, Connection>) {
    if out.out.is_empty() {
        return;
    }
    let Some((mut sender, transport)) = links.iter_mut().next() else { return };
    let kind = ChannelKind::of::<ScriptChannel>();
    let (n, b) = transport.channel_send(kind).and_then(|c| c.reliable_held()).unwrap_or((0, 0));
    if n >= HELD_MESSAGES {
        return;
    }
    for w in out.out.take(HELD_BYTES.saturating_sub(b)) {
        out.wires += 1;
        out.bytes += w.len() as u64;
        sender.send::<ScriptChannel>(Wire::from_raw(w));
    }
}
