use core::time::Duration;

use bevy_app::{App, PostUpdate, PreUpdate};
use bevy_ecs::schedule::{IntoScheduleConfigs, SystemSet};
use lightyear::prelude::client::InputDelayConfig;
use lightyear::prelude::{InputTimelineConfig, MessageSystems, SyncConfig};
use ssx_net_proto::TICK_HZ;

pub fn tick_duration() -> Duration {
    Duration::from_secs_f64(1.0 / f64::from(TICK_HZ))
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NetSystems {
    Receive,
    Send,
}

pub(crate) fn configure_sets(app: &mut App) {
    app.configure_sets(PreUpdate, NetSystems::Receive.after(MessageSystems::Receive));
    app.configure_sets(PostUpdate, NetSystems::Send.before(MessageSystems::Send));
}

pub fn client_sync_config() -> InputTimelineConfig {
    InputTimelineConfig::new(
        SyncConfig { speedup_factor: 1.0, ..SyncConfig::default() },
        InputDelayConfig::no_input_delay(),
    )
}
