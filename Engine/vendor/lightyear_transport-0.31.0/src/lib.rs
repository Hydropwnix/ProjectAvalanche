/*! # Lightyear Packet

Packet handling for the lightyear networking library.
This crate builds up on top of lightyear-io, to add packet fragmentation, channels, and reliability.
*/
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod channel;

pub mod error;

#[cfg(feature = "client")]
mod client;
pub mod packet;
pub mod plugin;
#[cfg(feature = "server")]
mod server;

pub mod prelude {
    pub use crate::channel::Channel;
    pub use crate::channel::builder::{ChannelMode, ChannelSettings, ReliableSettings, Transport};
    // Project Avalanche patch (T1, T6, T13): the reassembly and reliable-send limits, the receive and send errors
    pub use crate::channel::fragments::{FragmentLimits, ReliableLimits};
    pub use crate::plugin::{TransportReceiveError, TransportSendError};
    pub use crate::channel::receive::ChannelReceive;
    pub use crate::channel::registry::AppChannelExt;
    pub use crate::channel::registry::ChannelRegistry;
    pub use crate::channel::send::ChannelSend;
    pub use crate::packet::compression::{CompressionAlgorithm, CompressionConfig};
    pub use crate::packet::nack::PacketNackSettings;
    pub use crate::packet::priority_manager::{PriorityConfig, PriorityManager};
}
