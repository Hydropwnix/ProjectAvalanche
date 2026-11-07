use bevy_ecs::prelude::*;
use lightyear::netcode::DisconnectCause;
use lightyear::prelude::Disconnected;

use super::{Kicked, Leaving, PlayerSession, ServerStats};

pub type NotEndedByServer = (With<PlayerSession>, Without<Kicked>, Without<Leaving>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkEnd {
    Left,
    TimedOut,
    Other,
}

impl LinkEnd {
    pub fn of(cause: Option<&DisconnectCause>) -> Self {
        match cause {
            Some(DisconnectCause::ByClient) => Self::Left,
            Some(DisconnectCause::TimedOut) => Self::TimedOut,
            Some(DisconnectCause::ByServer) | None => Self::Other,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Left => "the client left (its goodbye: netcode's disconnect packets)",
            Self::TimedOut => "timed out (no packet from the client within its timeout)",
            Self::Other => "ended by netcode",
        }
    }
}

pub(super) fn count_ends(
    ended: On<Add<Disconnected>>,
    links: Query<Option<&DisconnectCause>, NotEndedByServer>,
    mut stats: ResMut<ServerStats>,
) {
    let Ok(cause) = links.get(ended.entity) else { return };
    match LinkEnd::of(cause) {
        LinkEnd::Left => stats.left += 1,
        LinkEnd::TimedOut => stats.timed_out += 1,
        LinkEnd::Other => {}
    }
}
