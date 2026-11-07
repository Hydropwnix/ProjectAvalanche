//! Project Avalanche patch (`Engine/vendor/PATCHES.md`, T12): replication data from the peer that does not apply
//! (it does not read, it names a component function the receiver has not registered, a size exceeds the message).
//! Upstream logged these (or panicked, for the last two); the application learns of them here and can end the
//! connection: a correct peer never sends them.

use alloc::string::String;

use bevy::prelude::*;

/// Written where replication data from the peer was refused.
#[derive(Message, Clone, Debug)]
pub struct ReplicationReceiveError {
    /// On the server, the client that sent it; on a client, `None` (the server sent it).
    pub client: Option<Entity>,
    pub error: String,
}

/// Project Avalanche patch (`Engine/vendor/PATCHES.md`, T14): what a client keeps of a server's replication at most —
/// the server entities it maps (and so spawns) and the update and mutate messages it buffers until they apply.
/// Upstream bounded none of them: a server could make a client spawn entities or buffer messages without end. Beyond a
/// limit the message is refused and reported ([`ReplicationReceiveError`]).
#[derive(Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplicationLimits {
    pub max_entities: usize,
    pub max_buffered_messages: usize,
}

impl Default for ReplicationLimits {
    // TODO(guess): 65 536 entities and 4 096 buffered messages; a default for applications that set none (the game's
    // client sets its own from its protocol); settle: the most an application's server replicates at once.
    fn default() -> Self {
        Self { max_entities: 65_536, max_buffered_messages: 4096 }
    }
}
