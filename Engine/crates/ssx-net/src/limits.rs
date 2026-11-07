use core::time::Duration;

use bevy_replicon::shared::receive_error::ReplicationLimits;
use lightyear::prelude::{FragmentLimits, ReliableLimits};
use ssx_net_proto::TICK_HZ;

// TODO(guess): 64 connections per server process; the plan's instance size is 16 riders (32 once measured), a server
// may run several instances; settle: phase 4's 32-player tick-time measurement and the operator's `server.toml`.
pub const MAX_CLIENTS: usize = 64;

// TODO(guess): 5 s; netcode's handshake is 2 round trips and the client resends every 0.1 s, so 5 s covers a 1 s RTT
// with loss; settle: the connect times of phase 2's sessions over real links.
pub const PENDING_LINK_TIMEOUT: Duration = Duration::from_secs(5);

// TODO(guess): 1 s, ten of a client's request intervals; settle: the first-request arrival times of phase 2's
// sessions over lossy links.
pub const UNAUTHENTICATED_LINK_TIMEOUT: Duration = Duration::from_secs(1);

// TODO(guess): 256 pending links (4× MAX_CLIENTS); settle: phase 4's join flood limit per IP (MTA: 4 per 30 s) in front
// of it, with a measured join burst.
pub const MAX_PENDING_LINKS: usize = 256;

pub const MAX_POSES_PER_SECOND: u32 = TICK_HZ;

// TODO(guess): one second of samples (30); settle: the bunching seen in phase 2's sessions at 100 ms RTT, 20 ms
// jitter, 2 % loss.
pub const POSE_BURST: u32 = TICK_HZ;

// TODO(guess): ±10 s (300 ticks); settle: the tick offsets seen in phase 2's sessions and the client's resync bound
// (lightyear's `max_error_margin`, 10 ticks).
pub const POSE_TICK_WINDOW: u32 = 10 * TICK_HZ;

// TODO(guess): 16 KiB, about three times the measured 5 567 bytes; settle: phase 3's standings and phase 4's
// 32-rider instances, measured the same way.
pub const MAX_FRAGMENTED_MESSAGE: usize = 16 * 1024;

pub const MAX_FRAGMENTS_IN_FLIGHT: usize = 4 * MAX_FRAGMENTED_MESSAGE;

pub const RELIABLE_FRAGMENT_STALE_AFTER: Duration = Duration::from_secs(10);

pub fn fragment_limits() -> FragmentLimits {
    FragmentLimits {
        max_message_size: MAX_FRAGMENTED_MESSAGE,
        max_in_flight_bytes: MAX_FRAGMENTS_IN_FLIGHT,
        reliable_stale_after: RELIABLE_FRAGMENT_STALE_AFTER,
    }
}

// TODO(guess): 256 messages and 256 KiB per channel; phase 2's reliable traffic is the control channel (a welcome, the
// look, loaded, ready, the operator's notices) and replicon's updates (a player joining or leaving, a look), a few
// messages a second at most, each acknowledged within a round trip; settle: the most held in phase 2's soak at
// 100 ms RTT and 2 % loss, and phase 3's standings at 32 riders.
pub const MAX_UNACKED_MESSAGES: usize = 256;
pub const MAX_UNACKED_BYTES: usize = 256 * 1024;

pub fn reliable_limits() -> ReliableLimits {
    ReliableLimits { max_unacked_messages: MAX_UNACKED_MESSAGES, max_unacked_bytes: MAX_UNACKED_BYTES }
}

// TODO(guess): 2 × MAX_CLIENTS entities (MAX_CLIENTS player entities, at most 8 instances and 8 standings entities,
// phase 3) and two mutate messages a tick for the connection's timeout, 600 (a tick's mutations — the standings' 64
// rows of 21 bytes, the players' pings and statuses — split into at most two packets); measured: round net3's window
// was ended at the former 64 while its level loaded (a frame of 3.5 s, about 105 ticks of resent mutations); settle:
// the mutate messages a tick of a 64-player round sends, measured.
pub const MAX_REPLICATED_ENTITIES: usize = 2 * MAX_CLIENTS;
pub const MAX_BUFFERED_REPLICATION: usize = 2 * CONNECTION_TIMEOUT_SECS as usize * TICK_HZ as usize;

pub fn replication_limits() -> ReplicationLimits {
    ReplicationLimits { max_entities: MAX_REPLICATED_ENTITIES, max_buffered_messages: MAX_BUFFERED_REPLICATION }
}

// TODO(guess): 10 s; netcode's own default is 30 s; the join's answer and the first connect request take one HTTP and one
// UDP round trip; settle: the join-to-connect times of phase 2's sessions over real links.
pub const TOKEN_EXPIRE_SECS: i32 = 10;

// TODO(guess): 10 s, the game's hitch and alt-tab tolerance; settle: the stalls of phase 2's game windows (loading a
// drop) measured on both systems.
pub const CONNECTION_TIMEOUT_SECS: i32 = 10;

pub const TICKET_LIFETIME: Duration = Duration::from_secs(TOKEN_EXPIRE_SECS as u64 + PENDING_LINK_TIMEOUT.as_secs());

// TODO(guess): 1 024 tickets; settle: a measured join burst of phase 4's 32-player servers.
pub const MAX_TICKETS: usize = 1024;

// TODO(guess): 4, the join flood limit's count; settle: households with several players behind one address (LAN
// parties) seen once servers are public.
pub const MAX_TICKETS_PER_ADDRESS: usize = 4;

// TODO(guess): 1 s, three resends of a reliable message at a 200 ms round trip; settle: the arrival of the reason in
// phase 2's sessions at 5 % loss.
pub const LEAVE_GRACE: Duration = Duration::from_secs(1);

// TODO(guess): 60 s each, six times what this server issues (TOKEN_EXPIRE_SECS 10, CONNECTION_TIMEOUT_SECS 10);
// settle: the tokens of other server builds once servers are public (phase 4).
pub const MAX_ACCEPTED_TOKEN_SECS: u64 = 60;
pub const MAX_ACCEPTED_TIMEOUT_SECS: i32 = 60;
