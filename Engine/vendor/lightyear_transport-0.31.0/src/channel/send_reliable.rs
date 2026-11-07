use alloc::vec::Vec;
use bevy_time::{Real, Time, Timer};
use bytes::Bytes;
use core::time::Duration;
use lightyear_link::LinkStats;
use lightyear_utils::collections::HashMap;
use tracing::trace;

use crate::channel::builder::ReliableSettings;
use crate::channel::fragments::FragmentSender;
use crate::channel::registry::{ChannelId, ChannelKind};
use crate::packet::compression::{CompressionConfig, CompressionScratch};
use crate::packet::message::{
    FragmentData, MessageAck, MessageId, SendCandidate, SendMessage, SendMessageKey, SingleData,
};

#[derive(Debug)]
struct FragmentAck {
    data: FragmentData,
    acked: bool,
    last_sent: Option<Duration>,
}

#[derive(Debug)]
enum UnackedMessage {
    Single {
        bytes: Bytes,
        last_sent: Option<Duration>,
    },
    Fragmented(Vec<FragmentAck>),
}

#[derive(Debug)]
struct PendingReliableMessage {
    message: UnackedMessage,
    base_priority: f32,
    accumulated_priority: f32,
    /// Stable tie-breaker assigned when the message is buffered.
    send_order: u64,
    /// Project Avalanche patch (T13): the message's bytes as buffered.
    len: usize,
}

/// State which differs specifically for reliable channel sends.
#[derive(Debug)]
pub(super) struct ReliableSendState {
    settings: ReliableSettings,
    /// Pending messages keyed by exact wrapping ID. Chronology lives in `send_order`.
    pending: HashMap<MessageId, PendingReliableMessage>,
    next_message_id: MessageId,
    next_send_order: u64,
    current_rtt: Duration,
    current_time: Duration,
    priority_multiplier: f32,
    /// Project Avalanche patch (T13): the bytes of the messages held until acknowledged (`pending`).
    pending_bytes: usize,
}

impl ReliableSendState {
    pub(super) fn new(settings: ReliableSettings) -> Self {
        Self {
            settings,
            pending: HashMap::default(),
            next_message_id: MessageId::default(),
            next_send_order: 0,
            current_rtt: Duration::default(),
            current_time: Duration::default(),
            priority_multiplier: 1.0,
            pending_bytes: 0,
        }
    }

    /// Project Avalanche patch (T13): the messages held until the peer acknowledges them, and their bytes.
    pub(super) fn held(&self) -> (usize, usize) {
        (self.pending.len(), self.pending_bytes)
    }

    pub(super) fn update(
        &mut self,
        real_time: &Time<Real>,
        link_stats: &LinkStats,
        timer: Option<&Timer>,
    ) {
        self.current_time = real_time.elapsed();
        self.current_rtt = link_stats.rtt;
        if let Some(timer) = timer {
            self.priority_multiplier =
                timer.duration().as_nanos() as f32 / real_time.delta().as_nanos() as f32;
            trace!(
                ?timer,
                priority_multiplier = self.priority_multiplier,
                "updated reliable channel-send priority multiplier"
            );
        }
    }

    pub(super) fn buffer_send(
        &mut self,
        fragmenter: &mut FragmentSender,
        message: Bytes,
        priority: f32,
        compression: CompressionConfig,
        compression_scratch: &mut CompressionScratch,
    ) -> MessageId {
        let message_id = self.next_message_id;
        let len = message.len();
        let message = if message.len() > fragmenter.fragment_size {
            UnackedMessage::Fragmented(
                fragmenter
                    .build_fragments_for_message(
                        message_id,
                        message,
                        compression,
                        compression_scratch,
                    )
                    .into_iter()
                    .map(|data| FragmentAck {
                        data,
                        acked: false,
                        last_sent: None,
                    })
                    .collect(),
            )
        } else {
            UnackedMessage::Single {
                bytes: message,
                last_sent: None,
            }
        };
        self.pending.insert(
            message_id,
            PendingReliableMessage {
                message,
                base_priority: priority,
                accumulated_priority: 0.0,
                send_order: self.next_send_order,
                len,
            },
        );
        self.pending_bytes += len;
        self.next_message_id += 1;
        self.next_send_order = self
            .next_send_order
            .checked_add(1)
            .expect("reliable message send order exhausted");
        message_id
    }

    pub(super) fn collect_candidates(
        &mut self,
        channel_kind: ChannelKind,
        channel_id: ChannelId,
        output: &mut Vec<SendCandidate>,
    ) {
        let resend_delay = self.settings.resend_delay(self.current_rtt);
        let current_time = self.current_time;
        let should_send = |last_sent: &Option<Duration>| match last_sent {
            None => true,
            Some(last_sent) => {
                resend_delay != Duration::default()
                    && current_time.saturating_sub(*last_sent) > resend_delay
            }
        };

        for (message_id, pending) in &mut self.pending {
            pending.accumulated_priority += pending.base_priority * self.priority_multiplier;
            match &mut pending.message {
                UnackedMessage::Single { bytes, last_sent } if should_send(last_sent) => {
                    output.push(SendCandidate::new_reliable(
                        channel_kind,
                        channel_id,
                        SendMessageKey::ReliableSingle(*message_id),
                        pending.send_order,
                        SendMessage {
                            data: SingleData::new(Some(*message_id), bytes.clone()).into(),
                            priority: pending.accumulated_priority,
                        },
                    ));
                }
                UnackedMessage::Single { .. } => {}
                UnackedMessage::Fragmented(fragments) => {
                    output.extend(
                        fragments
                            .iter_mut()
                            .filter(|fragment| !fragment.acked && should_send(&fragment.last_sent))
                            .map(|fragment| {
                                SendCandidate::new_reliable(
                                    channel_kind,
                                    channel_id,
                                    SendMessageKey::ReliableFragment(
                                        *message_id,
                                        fragment.data.fragment_id,
                                    ),
                                    pending.send_order,
                                    SendMessage {
                                        data: fragment.data.clone().into(),
                                        priority: pending.accumulated_priority,
                                    },
                                )
                            }),
                    );
                }
            }
        }
    }

    pub(super) fn commit_send(&mut self, key: SendMessageKey, sent_at: Duration) {
        match key {
            SendMessageKey::ReliableSingle(message_id) => {
                let Some(pending) = self.pending.get_mut(&message_id) else {
                    debug_assert!(false, "missing reliable message during send commit");
                    return;
                };
                let UnackedMessage::Single { last_sent, .. } = &mut pending.message else {
                    debug_assert!(false, "reliable send key did not match message shape");
                    return;
                };
                *last_sent = Some(sent_at);
            }
            SendMessageKey::ReliableFragment(message_id, fragment_id) => {
                let Some(pending) = self.pending.get_mut(&message_id) else {
                    debug_assert!(false, "missing reliable message during fragment commit");
                    return;
                };
                let UnackedMessage::Fragmented(fragments) = &mut pending.message else {
                    debug_assert!(false, "reliable fragment key did not match message shape");
                    return;
                };
                let Some(fragment) = fragments.get_mut(fragment_id.0 as usize) else {
                    debug_assert!(false, "missing reliable fragment during send commit");
                    return;
                };
                fragment.last_sent = Some(sent_at);
            }
            _ => debug_assert!(false, "unreliable key committed to reliable channel send"),
        }
    }

    /// Applies an acknowledgement and returns whether it completed the logical message.
    pub(super) fn receive_ack(&mut self, ack: &MessageAck) -> bool {
        let Some(pending) = self.pending.get_mut(&ack.message_id) else {
            return false;
        };
        match &mut pending.message {
            UnackedMessage::Single { .. } => {
                assert!(
                    ack.fragment_id.is_none(),
                    "received a fragment ack for a single reliable message"
                );
                if let Some(done) = self.pending.remove(&ack.message_id) {
                    self.pending_bytes -= done.len;
                }
                true
            }
            UnackedMessage::Fragmented(fragments) => {
                let fragment_id = ack
                    .fragment_id
                    .expect("received a single-message ack for a fragmented reliable message");
                let fragment = &mut fragments[fragment_id.0 as usize];
                if fragment.acked {
                    return false;
                }
                fragment.acked = true;
                let completed = fragments.iter().all(|fragment| fragment.acked);
                if completed && let Some(done) = self.pending.remove(&ack.message_id) {
                    self.pending_bytes -= done.len;
                }
                completed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestChannel;

    fn pending(bytes: &'static [u8], send_order: u64) -> PendingReliableMessage {
        PendingReliableMessage {
            message: UnackedMessage::Single {
                bytes: Bytes::from_static(bytes),
                last_sent: None,
            },
            base_priority: 1.0,
            accumulated_priority: 0.0,
            send_order,
            len: bytes.len(),
        }
    }

    #[test]
    fn pending_send_order_and_lookup_survive_message_id_rollover() {
        let mut state = ReliableSendState::new(ReliableSettings::default());
        state
            .pending
            .insert(MessageId(u16::MAX), pending(b"max", 41));
        state.pending.insert(MessageId(0), pending(b"zero", 42));
        // Project Avalanche patch (T13): the held bytes kept with the messages the test inserts itself
        state.pending_bytes = b"max".len() + b"zero".len();

        let mut candidates = Vec::new();
        state.collect_candidates(ChannelKind::of::<TestChannel>(), 0, &mut candidates);
        let mut order = candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.message.data.message_id().unwrap(),
                    candidate.send_order,
                )
            })
            .collect::<Vec<_>>();
        order.sort_by_key(|(_, send_order)| *send_order);
        assert_eq!(order, [(MessageId(u16::MAX), 41), (MessageId(0), 42)]);

        assert!(state.receive_ack(&MessageAck {
            message_id: MessageId(u16::MAX),
            fragment_id: None,
        }));
        assert!(state.pending.contains_key(&MessageId(0)));
        assert_eq!(state.held(), (1, b"zero".len()));

        candidates.clear();
        state.collect_candidates(ChannelKind::of::<TestChannel>(), 0, &mut candidates);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].send_order, 42);
    }

    /// Project Avalanche patch (T13): the held messages and bytes follow buffering and acknowledgement, single and
    /// fragmented messages alike.
    #[test]
    fn held_messages_and_bytes_follow_buffering_and_acks() {
        let mut state = ReliableSendState::new(ReliableSettings::default());
        let mut fragmenter = FragmentSender::new();
        fragmenter.set_fragment_size(100);
        let mut scratch = CompressionScratch::default();
        let compression = CompressionConfig::default();
        let single = state.buffer_send(&mut fragmenter, Bytes::from(alloc::vec![1u8; 60]), 1.0, compression, &mut scratch);
        let fragmented =
            state.buffer_send(&mut fragmenter, Bytes::from(alloc::vec![2u8; 250]), 1.0, compression, &mut scratch);
        assert_eq!(state.held(), (2, 310));
        assert!(state.receive_ack(&MessageAck { message_id: single, fragment_id: None }));
        assert_eq!(state.held(), (1, 250));
        let fragments = 3u8;
        for k in 0..fragments {
            let done = state.receive_ack(&MessageAck {
                message_id: fragmented,
                fragment_id: Some(crate::packet::message::FragmentIndex(u64::from(k))),
            });
            assert_eq!(done, k + 1 == fragments);
        }
        assert_eq!(state.held(), (0, 0));
    }
}
