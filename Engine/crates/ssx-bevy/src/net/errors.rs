use std::sync::Mutex;

use bevy::ecs::error::{BevyError, ErrorContext};
use bevy::prelude::error;

static NETWORK_ERROR: Mutex<Option<String>> = Mutex::new(None);

const NETWORK: [&str; 7] =
    ["lightyear", "bevy_replicon", "aeronet", "ssx_net::", "ssx_net_join", "ssx_bevy::net::", "ssx_net_proto"];

pub fn handler(e: BevyError, ctx: ErrorContext) {
    let name = ctx.name().to_string();
    if NETWORK.iter().any(|p| name.contains(p)) {
        error!("network: {ctx}: {e}");
        if let Ok(mut slot) = NETWORK_ERROR.lock() {
            slot.get_or_insert_with(|| format!("{ctx}: {e}"));
        }
    } else {
        bevy::ecs::error::panic(e, ctx);
    }
}

pub fn take() -> Option<String> {
    NETWORK_ERROR.lock().ok().and_then(|mut slot| slot.take())
}
