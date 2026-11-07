use core::marker::PhantomData;

use bevy_app::{App, Plugin};
use bevy_ecs::component::Component;
use bevy_replicon::prelude::Diffable;
use bytes::Bytes;
use lightyear::prelude::{
    AppChannelExt, AppComponentExt, AppMessageExt, ChannelMode, ChannelSettings, NetworkDirection, ReliableSettings,
};
use lightyear_serde::SerializationError;
use lightyear_serde::reader::Reader;
use lightyear_serde::registry::SerializeFns;
use lightyear_serde::writer::Writer;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ssx_net_proto::round::STANDING_SIZE;
use ssx_net_proto::{ClientChat, ClientControl, NetPose, RelayedPose, ServerChat, ServerControl, StandingRow};

use crate::roster::{
    InInstance, InstanceInfo, PlayerAppearance, PlayerKeyId, PlayerName, PlayerPing, PlayerState, RiderNumber,
};

pub struct ControlChannel;

pub struct ChatChannel;

pub struct ScriptChannel;

const _: () = match lightyear::transport::packet::fragment_size_for_min_mtu(lightyear::link::DEFAULT_MTU) {
    Some(size) => assert!(ssx_net_proto::scripts::MAX_WIRE + 3 <= size, "a script wire message would be fragmented"),
    None => panic!("the default MTU holds no transport fragment"),
};

pub struct ScriptUp;
pub struct ScriptDown;

pub struct PoseChannel;

pub struct Wire<T> {
    bytes: Bytes,
    _type: PhantomData<fn() -> T>,
}

impl<T> Clone for Wire<T> {
    fn clone(&self) -> Self {
        Self { bytes: self.bytes.clone(), _type: PhantomData }
    }
}

impl<T> Wire<T> {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn from_raw(bytes: impl Into<Bytes>) -> Self {
        Self { bytes: bytes.into(), _type: PhantomData }
    }
}

impl Wire<NetPose> {
    pub fn pose(pose: &NetPose) -> Self {
        Self::from_raw(Bytes::copy_from_slice(&pose.encode()))
    }
}

impl Wire<RelayedPose> {
    pub fn relayed(pose: &RelayedPose) -> Self {
        Self::from_raw(Bytes::copy_from_slice(&pose.encode()))
    }
}

impl Wire<ServerControl> {
    pub fn control(message: &ServerControl) -> Self {
        Self::from_raw(Bytes::from(message.encode()))
    }
}

impl Wire<ClientControl> {
    pub fn client_control(message: &ClientControl) -> Self {
        Self::from_raw(Bytes::from(message.encode()))
    }
}

impl Wire<ClientChat> {
    pub fn client_chat(message: &ClientChat) -> Self {
        Self::from_raw(Bytes::from(message.encode()))
    }
}

impl Wire<ServerChat> {
    pub fn server_chat(message: &ServerChat) -> Self {
        Self::from_raw(Bytes::from(message.encode()))
    }
}

fn wire_fns<T: 'static>() -> SerializeFns<Wire<T>> {
    fn serialize<T>(message: &Wire<T>, writer: &mut Writer) -> Result<(), SerializationError> {
        writer.extend_from_slice(&message.bytes);
        Ok(())
    }
    fn deserialize<T>(reader: &mut Reader) -> Result<Wire<T>, SerializationError> {
        let bytes = reader.split_len(reader.remaining())?;
        Ok(Wire::from_raw(bytes))
    }
    SerializeFns { serialize: serialize::<T>, deserialize: deserialize::<T> }
}

pub const MAX_STANDINGS: usize = crate::limits::MAX_CLIENTS;

#[derive(Component, Clone, Debug, Default, PartialEq, Eq)]
pub struct Standings {
    instance: u16,
    round: u16,
    rows: Vec<StandingRow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StandingsDiff {
    Round(u16),
    Set { slot: u8, row: StandingRow },
    Truncate(u8),
}

impl Standings {
    pub fn new(instance: u16, round: u16) -> Self {
        Self { instance, round, rows: Vec::new() }
    }

    pub fn instance(&self) -> u16 {
        self.instance
    }

    pub fn round(&self) -> u16 {
        self.round
    }

    pub fn rows(&self) -> &[StandingRow] {
        &self.rows
    }

    fn encode(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(5 + self.rows.len() * STANDING_SIZE);
        b.extend_from_slice(&self.instance.to_be_bytes());
        b.extend_from_slice(&self.round.to_be_bytes());
        b.push(self.rows.len().min(MAX_STANDINGS) as u8);
        for row in self.rows.iter().take(MAX_STANDINGS) {
            b.extend_from_slice(&row.encode());
        }
        b
    }

    fn decode(b: &[u8]) -> Result<Self, String> {
        let Some((&[i0, i1, r0, r1, count], rest)) = b.split_first_chunk::<5>() else {
            return Err(format!("standings of {} bytes", b.len()));
        };
        let count = usize::from(count);
        if count > MAX_STANDINGS || rest.len() != count * STANDING_SIZE {
            return Err(format!("standings of {count} rows in {} bytes", rest.len()));
        }
        let mut rows = Vec::with_capacity(count);
        let (chunks, _) = rest.as_chunks::<STANDING_SIZE>();
        for chunk in chunks {
            let row = StandingRow::decode(chunk).map_err(|e| e.to_string())?;
            if rows.iter().any(|r: &StandingRow| r.rider == row.rider) {
                return Err(format!("standings name rider {} twice", row.rider));
            }
            rows.push(row);
        }
        Ok(Self { instance: u16::from_be_bytes([i0, i1]), round: u16::from_be_bytes([r0, r1]), rows })
    }
}

impl Serialize for Standings {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.encode())
    }
}

impl<'de> Deserialize<'de> for Standings {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::roster::bytes_of(d, 5 + MAX_STANDINGS * STANDING_SIZE, "standings", Standings::decode)
    }
}

impl StandingsDiff {
    const ROUND: u8 = 1;
    const SET: u8 = 2;
    const TRUNCATE: u8 = 3;
    const MAX_SIZE: usize = 2 + STANDING_SIZE;

    fn encode(&self) -> Vec<u8> {
        match *self {
            Self::Round(round) => [&[Self::ROUND][..], &round.to_be_bytes()].concat(),
            Self::Set { slot, row } => [&[Self::SET, slot][..], &row.encode()].concat(),
            Self::Truncate(len) => vec![Self::TRUNCATE, len],
        }
    }

    fn decode(b: &[u8]) -> Result<Self, String> {
        match *b {
            [Self::ROUND, r0, r1] => Ok(Self::Round(u16::from_be_bytes([r0, r1]))),
            [Self::SET, slot, ref row @ ..] => {
                Ok(Self::Set { slot, row: StandingRow::decode(row).map_err(|e| e.to_string())? })
            }
            [Self::TRUNCATE, len] => Ok(Self::Truncate(len)),
            _ => Err(format!("a standings diff of {} bytes", b.len())),
        }
    }
}

impl Serialize for StandingsDiff {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.encode())
    }
}

impl<'de> Deserialize<'de> for StandingsDiff {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::roster::bytes_of(d, Self::MAX_SIZE, "a standings diff", StandingsDiff::decode)
    }
}

impl Diffable for Standings {
    type Diff = StandingsDiff;

    fn apply_diff(&mut self, diff: &StandingsDiff) -> bevy_ecs::error::Result<()> {
        match *diff {
            StandingsDiff::Round(round) => {
                self.round = round;
                self.rows.clear();
            }
            StandingsDiff::Set { slot, row } => {
                let slot = usize::from(slot);
                if slot >= MAX_STANDINGS || slot > self.rows.len() {
                    return Err(
                        format!("standings: row {slot} of {} (at most {MAX_STANDINGS})", self.rows.len()).into()
                    );
                }
                if slot == self.rows.len() {
                    self.rows.push(row);
                } else {
                    self.rows[slot] = row;
                }
            }
            StandingsDiff::Truncate(len) => self.rows.truncate(usize::from(len)),
        }
        Ok(())
    }
}

pub struct ProtocolPlugin;

impl Plugin for ProtocolPlugin {
    fn build(&self, app: &mut App) {
        app.add_channel::<ControlChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..ChannelSettings::default()
        })
        .add_direction(NetworkDirection::Bidirectional);
        app.add_channel::<PoseChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedUnreliable,
            ..ChannelSettings::default()
        })
        .add_direction(NetworkDirection::Bidirectional);
        app.add_channel::<ChatChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..ChannelSettings::default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.add_channel::<ScriptChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..ChannelSettings::default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.register_message_custom_serde::<Wire<NetPose>>(wire_fns()).add_direction(NetworkDirection::ClientToServer);
        app.register_message_custom_serde::<Wire<RelayedPose>>(wire_fns())
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message_custom_serde::<Wire<ServerControl>>(wire_fns())
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message_custom_serde::<Wire<ClientControl>>(wire_fns())
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message_custom_serde::<Wire<ClientChat>>(wire_fns())
            .add_direction(NetworkDirection::ClientToServer);
        app.register_message_custom_serde::<Wire<ServerChat>>(wire_fns())
            .add_direction(NetworkDirection::ServerToClient);
        app.register_message_custom_serde::<Wire<ScriptUp>>(wire_fns()).add_direction(NetworkDirection::ClientToServer);
        app.register_message_custom_serde::<Wire<ScriptDown>>(wire_fns())
            .add_direction(NetworkDirection::ServerToClient);

        app.component::<Standings>().replicate_diff();
        app.component::<InstanceInfo>().replicate();
        app.component::<RiderNumber>().replicate();
        app.component::<PlayerName>().replicate();
        app.component::<PlayerKeyId>().replicate();
        app.component::<InInstance>().replicate();
        app.component::<PlayerAppearance>().replicate();
        app.component::<PlayerPing>().replicate();
        app.component::<PlayerState>().replicate();
        crate::tick::configure_sets(app);
        app.insert_resource(crate::limits::fragment_limits());
        app.insert_resource(crate::limits::reliable_limits());
        app.insert_resource(crate::limits::replication_limits());
    }
}
