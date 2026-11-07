use bevy::prelude::*;
use ssx_net::client::{ScriptReceived, ScriptSend};
use ssx_script::host::outbox::Message;
use ssx_script::host::wire::{from_item, to_item};

use super::{ScriptBridge, worker};

pub(super) fn inbound(mut bridge: ResMut<ScriptBridge>, mut got: ResMut<ScriptReceived>) {
    if got.items.is_empty() || bridge.gone.is_some() {
        return;
    }
    let (items, bytes) = got.take();
    let messages = items.into_iter().map(from_item).collect();
    if let Err(worker::Job::Net(back)) = bridge.send_or_return(worker::Job::Net(messages)) {
        let back = back.into_iter().map(|(p, b)| to_item(p.as_deref(), &b)).collect();
        got.put_back(back, bytes);
    }
}

pub(super) fn outbound(send: Option<&mut ScriptSend>, net: Vec<Message>) {
    let Some(send) = send else { return };
    for m in net {
        if let Err(e) = send.push(&to_item(m.package.as_deref(), &m.body)) {
            warn!("scripts: a message to the server not sent: {}", ssx_core::text::shown(&e));
        }
    }
}
