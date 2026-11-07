//! Fragment validation, reassembly, and decompression for channel receives.

use alloc::{vec, vec::Vec};
use bevy_platform::collections::HashMap;

use crate::channel::receive::ChannelReceiveError;

type Result<T> = core::result::Result<T, ChannelReceiveError>;
#[cfg(feature = "compression_lz4")]
use crate::packet::compression::CompressionAlgorithm;
use crate::packet::compression::{CompressionConfig, decompress_payload};
use crate::packet::error::PacketError;
use crate::packet::message::{FragmentCompression, FragmentData, MessageId};
use crate::packet::packet::FRAGMENT_SIZE;
use bytes::Bytes;
use core::time::Duration;
use lightyear_core::tick::Tick;
use tracing::trace;

/// Project Avalanche patch (`Engine/vendor/PATCHES.md`, T1): the bounds on fragment reassembly, checked before a
/// fragmented message's buffer is allocated. Upstream 0.31.0 allocated the fragment count the peer declares times the
/// fragment size for the first fragment of every message id, unchecked (a 30-byte packet declaring 2^40 fragments
/// aborts the process), and kept incomplete messages of reliable channels forever.
///
/// Read by [`crate::plugin::TransportPlugin`] from this resource; an application sets its own with `insert_resource`.
#[derive(bevy_ecs::resource::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FragmentLimits {
    /// The largest reassembled message a peer may announce, in bytes (fragment count × fragment size).
    pub max_message_size: usize,
    /// The bytes of incomplete fragmented messages one channel of one connection may hold at once.
    pub max_in_flight_bytes: usize,
    /// How long an incomplete message of a reliable channel is kept without a new fragment (an unreliable channel
    /// keeps upstream's 3 s). Fragments a receiver has acknowledged are not sent again, so a reliable message
    /// dropped here does not complete: the bound is for a peer that has stopped sending.
    pub reliable_stale_after: Duration,
}

impl Default for FragmentLimits {
    // TODO(guess): 1 MiB per message, 4 MiB per channel and connection, 10 s for a reliable channel's partial message
    // (the longest netcode client timeout of a server-issued token); a default for applications that set none, the
    // game's server and client set their own from their protocol; settle: the largest message an application of
    // this copy fragments.
    fn default() -> Self {
        Self {
            max_message_size: 1 << 20,
            max_in_flight_bytes: 4 << 20,
            reliable_stale_after: Duration::from_secs(10),
        }
    }
}

/// Project Avalanche patch (`Engine/vendor/PATCHES.md`, T13): what one reliable channel of one connection may hold
/// for a peer that does not acknowledge it (a resource like [`FragmentLimits`]). Upstream kept every unacknowledged
/// reliable message for ever: a peer that never acknowledged made its sender's memory grow with every message, and
/// past 65 536 held the wrapping message ids would have overwritten held messages. A send that would exceed a limit
/// is not buffered and is reported (`TransportSendError`): the application ends that connection.
#[derive(bevy_ecs::resource::Resource, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableLimits {
    pub max_unacked_messages: usize,
    pub max_unacked_bytes: usize,
}

impl Default for ReliableLimits {
    // TODO(guess): 4 096 messages and 4 MiB per channel and connection (below the 32 768 the wrapping u16 message id
    // can tell apart); a default for applications that set none, the game's server and client set their own;
    // settle: the most a connection of the application holds under its worst loss.
    fn default() -> Self {
        Self { max_unacked_messages: 4096, max_unacked_bytes: 4 << 20 }
    }
}

/// `FragmentReceiver` is used to reconstruct fragmented messages
#[derive(Debug)]
pub struct FragmentReceiver {
    fragment_messages: HashMap<MessageId, FragmentConstructor>,
    fragment_size: usize,
    /// Project Avalanche patch (T1): the bytes reserved by the incomplete messages in `fragment_messages`.
    in_flight_bytes: usize,
}

impl FragmentReceiver {
    pub fn new() -> Self {
        Self {
            fragment_messages: HashMap::default(),
            fragment_size: FRAGMENT_SIZE,
            in_flight_bytes: 0,
        }
    }

    pub(crate) fn set_fragment_size(&mut self, fragment_size: usize) {
        debug_assert!(fragment_size > 0);
        self.fragment_size = fragment_size;
    }

    /// Discard all messages for which the latest fragment was received before the cleanup time
    /// (i.e. we probably lost some fragments and we will never complete the message)
    ///
    /// If we don't keep track of the last received time, we will never clean up the messages.
    pub fn cleanup(&mut self, cleanup_time: Duration) {
        let in_flight_bytes = &mut self.in_flight_bytes;
        self.fragment_messages.retain(|_, c| {
            let keep = c
                .last_received
                .map(|t| t > cleanup_time)
                .unwrap_or_else(|| true);
            if !keep {
                *in_flight_bytes = in_flight_bytes.saturating_sub(c.reserved);
            }
            keep
        })
    }

    /// Project Avalanche patch (T1): the bytes the incomplete messages hold.
    pub fn in_flight_bytes(&self) -> usize {
        self.in_flight_bytes
    }

    /// Receive a fragment of a FragmentData message.
    ///
    /// When we complete the final message by aggregating all fragments, we will return the
    /// `remote_sent_tick` associated with the first fragment received.
    pub fn receive_fragment(
        &mut self,
        fragment: FragmentData,
        remote_sent_tick: Tick,
        current_time: Option<Duration>,
        compression: CompressionConfig,
        limits: &FragmentLimits,
    ) -> Result<Option<(Tick, Bytes)>> {
        let num_fragments = fragment.num_fragments.0;
        let fragment_index = fragment.fragment_id.0;
        if num_fragments == 0 {
            return Err(ChannelReceiveError::InvalidFragmentCount { num_fragments });
        }
        if fragment_index >= num_fragments {
            return Err(ChannelReceiveError::InvalidFragmentIndex {
                fragment_index,
                num_fragments,
            });
        }
        let num_fragments = usize::try_from(num_fragments)
            .map_err(|_| ChannelReceiveError::FragmentedMessageSizeOverflow)?;
        let fragment_index = usize::try_from(fragment_index)
            .map_err(|_| ChannelReceiveError::FragmentedMessageSizeOverflow)?;

        let message_id = fragment.message_id;
        let fragment_compression = fragment.compression;
        if !self.fragment_messages.contains_key(&message_id) {
            // Project Avalanche patch (T1): the declared size and the channel's budget before any allocation
            let declared = num_fragments
                .checked_mul(self.fragment_size)
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            if declared > limits.max_message_size {
                return Err(ChannelReceiveError::FragmentedMessageTooLarge {
                    declared,
                    limit: limits.max_message_size,
                });
            }
            let in_flight = self.in_flight_bytes.saturating_add(declared);
            if in_flight > limits.max_in_flight_bytes {
                return Err(ChannelReceiveError::FragmentBudgetExceeded {
                    in_flight: self.in_flight_bytes,
                    declared,
                    limit: limits.max_in_flight_bytes,
                });
            }
            let constructor =
                FragmentConstructor::new(remote_sent_tick, num_fragments, self.fragment_size)?;
            self.fragment_messages.insert(message_id, constructor);
            self.in_flight_bytes = in_flight;
        }
        let fragment_message = self
            .fragment_messages
            .get_mut(&message_id)
            .expect("fragment constructor was just inserted");
        if fragment_message.num_fragments != num_fragments {
            return Err(ChannelReceiveError::FragmentCountMismatch {
                expected: fragment_message.num_fragments,
                actual: num_fragments,
            });
        }
        if let Some(fragment_compression) = fragment_compression {
            fragment_message.set_compression(fragment_compression)?;
        } else if fragment_index == 0 {
            return Err(ChannelReceiveError::MissingFragmentCompression);
        }

        // completed the fragmented message!
        if let Some(payload) = fragment_message.receive_fragment(
            fragment_index,
            fragment.bytes.as_ref(),
            current_time,
        )? {
            let fragment_compression = fragment_message
                .compression
                .ok_or(ChannelReceiveError::MissingFragmentCompression)?;
            if let Some(done) = self.fragment_messages.remove(&message_id) {
                self.in_flight_bytes = self.in_flight_bytes.saturating_sub(done.reserved);
            }
            let (tick, payload) = payload;
            return decompress_fragment_payload(fragment_compression, payload, compression)
                .map(|payload| Some((tick, payload)));
        }

        Ok(None)
    }
}

#[derive(Debug, Clone)]
/// Data structure to reconstruct a single fragmented message from individual fragments
pub struct FragmentConstructor {
    num_fragments: usize,
    fragment_size: usize,
    num_received_fragments: usize,
    received: Vec<bool>,
    // bytes: Bytes,
    bytes: Vec<u8>,

    tick: Tick,
    compression: Option<FragmentCompression>,
    last_received: Option<Duration>,
    /// Project Avalanche patch (T1): the bytes this message counts against the channel's budget.
    reserved: usize,
}

impl FragmentConstructor {
    pub fn new(tick: Tick, num_fragments: usize, fragment_size: usize) -> Result<Self> {
        let max_message_size = num_fragments
            .checked_mul(fragment_size)
            .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
        Ok(Self {
            num_fragments,
            fragment_size,
            num_received_fragments: 0,
            received: vec![false; num_fragments],
            bytes: vec![0; max_message_size],
            tick,
            compression: None,
            last_received: None,
            reserved: max_message_size,
        })
    }

    pub fn set_compression(&mut self, compression: FragmentCompression) -> Result<()> {
        if let Some(expected) = self.compression {
            if expected != compression {
                return Err(ChannelReceiveError::FragmentCompressionMismatch {
                    expected: expected.as_str(),
                    actual: compression.as_str(),
                });
            }
        } else {
            self.compression = Some(compression);
        }
        Ok(())
    }

    pub fn receive_fragment(
        &mut self,
        fragment_index: usize,
        bytes: &[u8],
        received_time: Option<Duration>,
    ) -> Result<Option<(Tick, Bytes)>> {
        self.last_received = received_time;

        let is_last_fragment = fragment_index == self.num_fragments - 1;

        if !is_last_fragment && bytes.len() != self.fragment_size {
            return Err(ChannelReceiveError::InvalidNonFinalFragmentSize {
                actual: bytes.len(),
                expected: self.fragment_size,
            });
        }
        if is_last_fragment && bytes.len() > self.fragment_size {
            return Err(ChannelReceiveError::InvalidFinalFragmentSize {
                actual: bytes.len(),
                max: self.fragment_size,
            });
        }

        if !self.received[fragment_index] {
            self.received[fragment_index] = true;
            self.num_received_fragments += 1;

            if is_last_fragment {
                let len = (self.num_fragments - 1)
                    .checked_mul(self.fragment_size)
                    .and_then(|len| len.checked_add(bytes.len()))
                    .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
                self.bytes.resize(len, 0);
            }

            let start = fragment_index
                .checked_mul(self.fragment_size)
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            let end = start
                .checked_add(bytes.len())
                .ok_or(ChannelReceiveError::FragmentedMessageSizeOverflow)?;
            self.bytes[start..end].copy_from_slice(bytes);
        }

        if self.num_received_fragments == self.num_fragments {
            trace!("Received all fragments!");
            let payload = core::mem::take(&mut self.bytes);
            return Ok(Some((self.tick, payload.into())));
        }

        Ok(None)
    }
}

fn decompress_fragment_payload(
    compression: FragmentCompression,
    payload: Bytes,
    config: CompressionConfig,
) -> Result<Bytes> {
    match compression {
        FragmentCompression::None => Ok(payload),
        FragmentCompression::Lz4 => {
            let config = config_for_fragment_compression(compression, config)?;
            decompress_payload(payload.as_ref(), config)
                .map(Bytes::from)
                .map_err(map_fragment_decompression_error)
        }
    }
}

fn config_for_fragment_compression(
    compression: FragmentCompression,
    config: CompressionConfig,
) -> Result<CompressionConfig> {
    match compression {
        FragmentCompression::None => Ok(config),
        FragmentCompression::Lz4 => {
            #[cfg(feature = "compression_lz4")]
            {
                if config.algorithm == Some(CompressionAlgorithm::Lz4) {
                    Ok(config)
                } else {
                    Err(ChannelReceiveError::UnsupportedFragmentCompression {
                        compression: compression.as_str(),
                    })
                }
            }
            #[cfg(not(feature = "compression_lz4"))]
            {
                Err(ChannelReceiveError::UnsupportedFragmentCompression {
                    compression: compression.as_str(),
                })
            }
        }
    }
}

fn map_fragment_decompression_error(error: PacketError) -> ChannelReceiveError {
    match error {
        PacketError::UnsupportedCompression => {
            ChannelReceiveError::UnsupportedFragmentCompression {
                compression: FragmentCompression::Lz4.as_str(),
            }
        }
        PacketError::DecompressionFailed => ChannelReceiveError::FragmentDecompressionFailed,
        PacketError::DecompressedPayloadTooLarge { actual, limit } => {
            ChannelReceiveError::FragmentDecompressedPayloadTooLarge { actual, limit }
        }
        _ => ChannelReceiveError::FragmentDecompressionFailed,
    }
}

#[cfg(test)]
mod tests {
    use crate::channel::fragments::FragmentSender;
    #[cfg(feature = "compression_lz4")]
    use crate::packet::compression::CompressionScratch;

    use super::*;

    #[test]
    fn test_receiver() -> Result<()> {
        let mut receiver = FragmentReceiver::new();
        let num_bytes = (FRAGMENT_SIZE as f32 * 1.5) as usize;
        let message_bytes = Bytes::from(vec![1u8; num_bytes]);
        let fragments = FragmentSender::new().build_fragments(MessageId(0), message_bytes.clone());

        assert_eq!(
            receiver.receive_fragment(
                fragments[1].clone(),
                Tick(1),
                None,
                CompressionConfig::DISABLED,
                &FragmentLimits::default(),
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[0].clone(),
                Tick(0),
                None,
                CompressionConfig::DISABLED,
                &FragmentLimits::default(),
            )?,
            Some((Tick(1), message_bytes.clone()))
        );
        Ok(())
    }

    #[test]
    fn reassembles_out_of_order_fragments_with_non_default_size() -> Result<()> {
        let mut sender = FragmentSender::new();
        sender.set_fragment_size(37);
        let message_bytes = Bytes::from(vec![3u8; 100]);
        let fragments = sender.build_fragments(MessageId(4), message_bytes.clone());
        let mut receiver = FragmentReceiver::new();
        receiver.set_fragment_size(37);

        assert_eq!(fragments.len(), 3);
        assert_eq!(
            receiver.receive_fragment(
                fragments[2].clone(),
                Tick(2),
                None,
                CompressionConfig::DISABLED,
                &FragmentLimits::default(),
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[0].clone(),
                Tick(0),
                None,
                CompressionConfig::DISABLED,
                &FragmentLimits::default(),
            )?,
            None
        );
        assert_eq!(
            receiver.receive_fragment(
                fragments[1].clone(),
                Tick(1),
                None,
                CompressionConfig::DISABLED,
                &FragmentLimits::default(),
            )?,
            Some((Tick(2), message_bytes))
        );
        Ok(())
    }

    #[cfg(feature = "compression_lz4")]
    #[test]
    fn compressed_fragments_are_decompressed_after_reassembly() -> Result<()> {
        let compression = CompressionConfig {
            min_payload_size: 0,
            max_decompressed_payload_size: FRAGMENT_SIZE * 4,
            ..CompressionConfig::LZ4
        };
        let mut receiver = FragmentReceiver::new();
        let message_bytes = Bytes::from(vec![7u8; FRAGMENT_SIZE * 3]);
        let fragments = FragmentSender::new().build_fragments_for_message(
            MessageId(1),
            message_bytes.clone(),
            compression,
            &mut CompressionScratch::default(),
        );

        assert!(fragments.len() < 3);
        assert_eq!(fragments[0].compression, Some(FragmentCompression::Lz4));
        assert!(
            fragments
                .iter()
                .skip(1)
                .all(|fragment| fragment.compression.is_none())
        );

        let mut result = None;
        for (index, fragment) in fragments.into_iter().enumerate() {
            result = receiver.receive_fragment(
                fragment,
                Tick(index as u32),
                None,
                compression,
                &FragmentLimits::default(),
            )?;
        }

        assert_eq!(result, Some((Tick(0), message_bytes)));
        Ok(())
    }

    /// Project Avalanche patch (T1): a declared size above the limit is refused before anything is allocated.
    #[test]
    fn declared_size_above_the_limit_is_refused() {
        let mut receiver = FragmentReceiver::new();
        let limits = FragmentLimits {
            max_message_size: 4 * FRAGMENT_SIZE,
            ..FragmentLimits::default()
        };
        let bomb = FragmentData {
            message_id: MessageId(7),
            fragment_id: crate::packet::message::FragmentIndex(1),
            num_fragments: crate::packet::message::FragmentIndex(1 << 40),
            compression: None,
            bytes: Bytes::from_static(&[0xAB]),
        };
        let error = receiver
            .receive_fragment(bomb, Tick(0), Some(Duration::ZERO), CompressionConfig::DISABLED, &limits)
            .unwrap_err();
        assert!(matches!(error, ChannelReceiveError::FragmentedMessageTooLarge { .. }));
        assert_eq!(receiver.in_flight_bytes(), 0);
    }

    /// Project Avalanche patch (T1): partial messages count against the channel's budget until they complete or go
    /// stale.
    #[test]
    fn partial_messages_are_bounded_and_freed() {
        let mut receiver = FragmentReceiver::new();
        let limits = FragmentLimits {
            max_message_size: 4 * FRAGMENT_SIZE,
            max_in_flight_bytes: 8 * FRAGMENT_SIZE,
            ..FragmentLimits::default()
        };
        let partial = |id: u16| FragmentData {
            message_id: MessageId(id),
            fragment_id: crate::packet::message::FragmentIndex(1),
            num_fragments: crate::packet::message::FragmentIndex(4),
            compression: None,
            bytes: Bytes::from(vec![0u8; FRAGMENT_SIZE]),
        };
        let at = Some(Duration::from_secs(1));
        for id in 0..2 {
            assert_eq!(
                receiver.receive_fragment(partial(id), Tick(0), at, CompressionConfig::DISABLED, &limits).unwrap(),
                None
            );
        }
        assert_eq!(receiver.in_flight_bytes(), 8 * FRAGMENT_SIZE);
        let error = receiver
            .receive_fragment(partial(2), Tick(0), at, CompressionConfig::DISABLED, &limits)
            .unwrap_err();
        assert!(matches!(error, ChannelReceiveError::FragmentBudgetExceeded { .. }));
        receiver.cleanup(Duration::from_secs(2));
        assert_eq!(receiver.in_flight_bytes(), 0);
        assert!(
            receiver
                .receive_fragment(partial(2), Tick(0), at, CompressionConfig::DISABLED, &limits)
                .is_ok()
        );
    }
}
