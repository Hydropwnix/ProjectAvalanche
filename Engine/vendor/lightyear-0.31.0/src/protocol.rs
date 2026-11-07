//! Module to verify that the protocols of the client and server match

use bevy_app::{App, Plugin};
use bevy_ecs::prelude::*;
use lightyear_connection::client::{Client, Connected, Disconnect};
use lightyear_connection::client_of::ClientOf;
use lightyear_connection::direction::NetworkDirection;
use lightyear_connection::host::HostClient;
use lightyear_core::id::RemoteId;
use lightyear_messages::prelude::{AppTriggerExt, EventSender, RemoteEvent};
use lightyear_messages::registry::MessageRegistry;
use lightyear_replication::metadata::MetadataChannel;
use lightyear_replication::registry::ComponentRegistry;
use lightyear_transport::prelude::{
    AppChannelExt, ChannelMode, ChannelRegistry, ChannelSettings, ReliableSettings,
};
use serde::{Deserialize, Serialize};
use tracing::{error, trace};

#[derive(Serialize, Deserialize, Debug, PartialEq, Default, Clone, Copy, Event)]
pub struct ProtocolCheck {
    messages: Option<u64>,
    components: Option<u64>,
    channels: Option<u64>,
}

pub struct ProtocolCheckPlugin;

#[derive(thiserror::Error, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolCheckError {
    #[error("the message protocol doesn't match")]
    Message,
    #[error("the component protocol doesn't match")]
    Component,
    #[error("the channel protocol doesn't match")]
    Channel,
}

/// Project Avalanche patch (`Engine/vendor/PATCHES.md`, T2): the remote peer's protocol differs from this one. The
/// client link that received the check is disconnected; upstream returned the mismatch as an error from an observer,
/// which Bevy's default error handler turns into a panic (a server with a different registration crashed every
/// client that joined it).
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct ProtocolMismatch {
    /// The client link that was disconnected.
    pub entity: Entity,
    pub error: ProtocolCheckError,
}

impl ProtocolCheckPlugin {
    /// On the server, send a message with the protocol checksum when a client connects
    fn send_verify_protocol(
        trigger: On<Add<Connected>>,
        mut sender: Query<&mut EventSender<ProtocolCheck>, (With<ClientOf>, Without<HostClient>)>,
        messages: Option<ResMut<MessageRegistry>>,
        components: Option<ResMut<ComponentRegistry>>,
        channels: Option<ResMut<ChannelRegistry>>,
    ) {
        let check_message = ProtocolCheck {
            messages: messages.map(|mut m| m.finish()),
            components: components.map(|mut c| c.finish()),
            channels: channels.map(|mut c| c.finish()),
        };
        if let Ok(mut s) = sender.get_mut(trigger.entity) {
            s.trigger::<MetadataChannel>(check_message);
        }
    }

    // Project Avalanche patch (T2): a mismatch disconnects the link it came from and triggers `ProtocolMismatch`
    fn receive_verify_protocol(
        trigger: On<RemoteEvent<ProtocolCheck>>,
        messages: Option<ResMut<MessageRegistry>>,
        components: Option<ResMut<ComponentRegistry>>,
        channels: Option<ResMut<ChannelRegistry>>,
        clients: Query<(Entity, &RemoteId), With<Client>>,
        mut commands: Commands,
    ) {
        let message = trigger.trigger;
        trace!("Received protocol check from server: {message:?}");
        let mismatch = if message.messages != messages.map(|mut m| m.finish()) {
            Some(ProtocolCheckError::Message)
        } else if message.components != components.map(|mut c| c.finish()) {
            Some(ProtocolCheckError::Component)
        } else if message.channels != channels.map(|mut c| c.finish()) {
            Some(ProtocolCheckError::Channel)
        } else {
            None
        };
        let Some(error) = mismatch else { return };
        for (entity, remote) in &clients {
            if remote.0 == trigger.from {
                error!(?entity, %error, "protocol check failed: disconnecting from the remote peer");
                commands.trigger(ProtocolMismatch { entity, error });
                commands.trigger(Disconnect { entity });
            }
        }
    }
}

impl Plugin for ProtocolCheckPlugin {
    fn build(&self, app: &mut App) {
        // TODO: add these observers only on the server/client
        // app.add_observer(Self::send_verify_protocol);
        // app.add_observer(Self::receive_verify_protocol);

        // try to re-add the Channel in case Replication is not enabled
        app.add_channel::<MetadataChannel>(ChannelSettings {
            mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
            priority: 10.0,
            ..Default::default()
        })
        .add_direction(NetworkDirection::Bidirectional);

        app.register_event::<ProtocolCheck>()
            .add_direction(NetworkDirection::ServerToClient);
    }

    fn finish(&self, app: &mut App) {
        // Replication can be enabled without any user-defined components. In that case,
        // make sure the registry exists on both sides so the protocol check compares
        // empty registries rather than `Some(...)` vs `None`.
        if !app.world().contains_resource::<ComponentRegistry>() {
            app.world_mut().init_resource::<ComponentRegistry>();
        }

        app.add_observer(Self::send_verify_protocol);
        app.add_observer(Self::receive_verify_protocol);
    }

    // fn finish(&self, app: &mut App) {
    //     // try to re-add the Channel in case Replication is not enabled
    //     app.add_channel::<MetadataChannel>(ChannelSettings {
    //         mode: ChannelMode::UnorderedReliable(ReliableSettings::default()),
    //         send_frequency: Duration::default(),
    //         priority: 10.0,
    //     })
    //     .add_direction(NetworkDirection::Bidirectional);
    //
    //     app.add_trigger::<ProtocolCheck>()
    //         .add_direction(NetworkDirection::ServerToClient);
    // }
}

#[cfg(test)]
mod tests {
    use super::ProtocolCheckPlugin;
    use bevy_app::App;
    use lightyear_replication::registry::ComponentRegistry;

    #[test]
    fn protocol_check_initializes_empty_component_registry() {
        let mut app = App::new();

        app.add_plugins(ProtocolCheckPlugin);
        app.finish();

        assert!(app.world().contains_resource::<ComponentRegistry>());
    }
}
