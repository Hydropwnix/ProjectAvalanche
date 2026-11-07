use crate::channel::ChannelKind;
use crate::channel::receive::ChannelReceiveError;
use crate::channel::registry::ChannelId;
use crate::packet::error::PacketError;
use bytes::Bytes;
use crossbeam_channel::TrySendError;

pub type Result<T> = core::result::Result<T, TransportError>;
#[derive(thiserror::Error, Debug)]
pub enum TransportError {
    #[error(transparent)]
    SerializationError(#[from] lightyear_serde::SerializationError),
    #[error(transparent)]
    PacketError(#[from] PacketError),
    #[error("channel {0:?} was not found")]
    ChannelNotFound(ChannelKind),
    #[error("channel {0:?} was not found")]
    ChannelIdNotFound(ChannelId),
    #[error("receiver channel error: {0}")]
    ChannelReceiveError(#[from] ChannelReceiveError),
    #[error("error sending data: {0}")]
    ChannelSendError(#[from] TrySendError<(ChannelKind, Bytes, f32)>),
    /// Project Avalanche patch (T13): the peer has not acknowledged this many of the channel's reliable messages and
    /// bytes; buffering one more of `adding` bytes would exceed `ReliableLimits`.
    #[error("channel {channel:?}: {messages} reliable messages ({bytes} bytes) unacknowledged, {adding} bytes more")]
    UnacknowledgedLimit { channel: ChannelKind, messages: usize, bytes: usize, adding: usize },
}
