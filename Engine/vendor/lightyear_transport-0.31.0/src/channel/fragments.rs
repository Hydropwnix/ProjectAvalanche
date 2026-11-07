//! Transport message fragmentation.

mod ack;
mod receive;
mod send;

pub(crate) use ack::FragmentAckReceiver;
pub(crate) use receive::FragmentReceiver;
// Project Avalanche patch (T1): the reassembly limits are the application's to set (T13: and the reliable sends').
pub use receive::{FragmentLimits, ReliableLimits};
pub(crate) use send::FragmentSender;
