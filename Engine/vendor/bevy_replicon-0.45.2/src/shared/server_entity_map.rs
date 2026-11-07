use bevy::{
    ecs::entity::{EntityHash, hash_map::EntityHashMap},
    platform::collections::hash_map::{self, Entry},
    prelude::*,
};
use log::{error, warn};

/// Maps server entities to client entities and vice versa.
///
/// Inserted as resource by [`ClientPlugin`](crate::client::ClientPlugin).
///
/// Automatically updated on replication receive. If the client predicts a despawn,
/// the mapping is kept until the server also despawns the entity.
#[derive(Resource, Default)]
pub struct ServerEntityMap {
    server_to_client: EntityHashMap<Entity>,
    client_to_server: EntityHashMap<Entity>,
    /// Project Avalanche patch (`Engine/vendor/PATCHES.md`, T14): the most server entities the client maps
    /// ([`crate::shared::receive_error::ReplicationLimits::max_entities`], set at each receive); a new mapping beyond it
    /// is refused before its entity is spawned. Upstream spawned an entity for every server entity a message named.
    limit: Option<usize>,
    /// A mapping was refused while a component's data was read ([`Self::take_refused`]).
    refused: bool,
}

impl ServerEntityMap {
    /// Project Avalanche patch (T14): the mapping limit ([`Self::at_limit`]).
    pub fn set_limit(&mut self, limit: Option<usize>) {
        self.limit = limit;
    }

    /// Project Avalanche patch (T14): no new server entity may be mapped.
    pub(crate) fn at_limit(&self) -> bool {
        self.limit.is_some_and(|limit| self.server_to_client.len() >= limit)
    }

    /// Project Avalanche patch (T14): an entity mapping inside a component's data was refused at the limit.
    pub(crate) fn refuse(&mut self) {
        self.refused = true;
    }

    /// Project Avalanche patch (T14): whether a mapping was refused since the last call.
    pub(crate) fn take_refused(&mut self) -> bool {
        core::mem::take(&mut self.refused)
    }

    /// Inserts a server-client pair into the map.
    pub(crate) fn insert(&mut self, server_entity: Entity, client_entity: Entity) {
        if let Some(existing_entity) = self.server_to_client.insert(server_entity, client_entity) {
            if client_entity != existing_entity {
                error!(
                    "mapping {server_entity:?} to {client_entity:?}, but it's already mapped to {existing_entity:?}"
                );
                self.client_to_server.remove(&existing_entity);
            } else {
                warn!("ignoring duplicate mapping from {server_entity:?} to {client_entity:?}");
            }
        }

        if let Some(existing_server_entity) =
            self.client_to_server.insert(client_entity, server_entity)
            && existing_server_entity != server_entity
        {
            self.server_to_client.remove(&existing_server_entity);
        }
    }

    /// Returns server to client mappings.
    #[inline]
    pub fn to_client(&self) -> &EntityHashMap<Entity> {
        &self.server_to_client
    }

    /// Returns client to server mappings.
    #[inline]
    pub fn to_server(&self) -> &EntityHashMap<Entity> {
        &self.client_to_server
    }

    /// Gets a server entry using the client entity.
    pub(crate) fn server_entry(&mut self, server_entity: Entity) -> EntityEntry<'_> {
        EntityEntry::new(
            self.server_to_client.entry(server_entity),
            &mut self.client_to_server,
        )
    }

    /// Removes a mapping by its client entity.
    pub(crate) fn remove_by_client(&mut self, client_entity: Entity) -> Option<Entity> {
        let server_entity = self.client_to_server.remove(&client_entity)?;
        self.server_to_client.remove(&server_entity);
        Some(server_entity)
    }

    /// Clears the map (the limit stays).
    pub(crate) fn clear(&mut self) {
        self.client_to_server.clear();
        self.server_to_client.clear();
        self.refused = false;
    }
}

/// A view into an entry in [`ServerEntityMap`].
#[must_use]
pub enum EntityEntry<'a> {
    Occupied(OccupiedEntityEntry<'a>),
    Vacant(VacantEntityEntry<'a>),
}

impl<'a> EntityEntry<'a> {
    fn new(
        main_entry: Entry<'a, Entity, Entity, EntityHash>,
        reverse_map: &'a mut EntityHashMap<Entity>,
    ) -> Self {
        match main_entry {
            Entry::Occupied(main_entry) => Self::Occupied(OccupiedEntityEntry {
                main_entry,
                reverse_map,
            }),
            Entry::Vacant(main_entry) => Self::Vacant(VacantEntityEntry {
                main_entry,
                reverse_map,
            }),
        }
    }

    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.get()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Option<Entity> {
        match self {
            EntityEntry::Occupied(entry) => Some(entry.remove()),
            EntityEntry::Vacant(_) => None,
        }
    }

    /// Inserts a new mapping from the function if the entry is not mapped, and returns the mapped entity.
    pub fn or_insert_with<F: FnOnce() -> Entity>(self, f: F) -> Entity {
        match self {
            EntityEntry::Occupied(entry) => entry.get(),
            EntityEntry::Vacant(entry) => entry.insert(f()),
        }
    }
}

/// A view into an occupied entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct OccupiedEntityEntry<'a> {
    main_entry: hash_map::OccupiedEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl OccupiedEntityEntry<'_> {
    /// Returns the mappend entity for the entry.
    pub fn get(&self) -> Entity {
        *self.main_entry.get()
    }

    /// Removes the entry and returns the mapped entity.
    pub fn remove(self) -> Entity {
        let (_, value) = self.main_entry.remove_entry();
        self.reverse_map.remove(&value);
        value
    }
}

/// A view into a vacant entry in [`ServerEntityMap`].
///
/// It's part of [`EntityEntry`] enum.
pub struct VacantEntityEntry<'a> {
    main_entry: hash_map::VacantEntry<'a, Entity, Entity, EntityHash>,
    reverse_map: &'a mut EntityHashMap<Entity>,
}

impl VacantEntityEntry<'_> {
    /// Sets the mapped entity for the entry and returns it.
    pub fn insert(self, value: Entity) -> Entity {
        let key = *self.main_entry.key();
        self.main_entry.insert(value);
        self.reverse_map.insert(value, key);
        value
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    #[test]
    fn mapping() {
        const SERVER_ENTITY: Entity = Entity::from_raw_u32(0).unwrap();
        const CLIENT_ENTITY: Entity = Entity::from_raw_u32(1).unwrap();
        const REPLACEMENT_SERVER_ENTITY: Entity = Entity::from_raw_u32(2).unwrap();

        let mut map = ServerEntityMap::default();
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), None);

        map.insert(SERVER_ENTITY, Entity::PLACEHOLDER);
        assert_eq!(
            map.server_entry(SERVER_ENTITY).get(),
            Some(Entity::PLACEHOLDER)
        );

        map.insert(SERVER_ENTITY, CLIENT_ENTITY);
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));

        assert_eq!(
            map.server_entry(SERVER_ENTITY).remove(),
            Some(CLIENT_ENTITY)
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), None);

        assert_eq!(
            map.server_entry(SERVER_ENTITY)
                .or_insert_with(|| CLIENT_ENTITY),
            CLIENT_ENTITY
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));

        assert_eq!(
            map.server_entry(SERVER_ENTITY)
                .or_insert_with(|| Entity::PLACEHOLDER),
            CLIENT_ENTITY
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), Some(CLIENT_ENTITY));

        map.insert(REPLACEMENT_SERVER_ENTITY, CLIENT_ENTITY);
        assert!(!map.to_client().contains_key(&SERVER_ENTITY));
        assert_eq!(
            map.to_client().get(&REPLACEMENT_SERVER_ENTITY),
            Some(&CLIENT_ENTITY)
        );

        assert_eq!(
            map.remove_by_client(CLIENT_ENTITY),
            Some(REPLACEMENT_SERVER_ENTITY)
        );
        assert_eq!(map.server_entry(SERVER_ENTITY).get(), None);
        assert!(!map.to_server().contains_key(&CLIENT_ENTITY));
    }

    /// Project Avalanche patch (T14): the limit counts the mapped server entities; a clear keeps it.
    #[test]
    fn the_limit_counts_mapped_entities() {
        let e = |i: u32| Entity::from_raw_u32(i).unwrap();
        let mut map = ServerEntityMap::default();
        assert!(!map.at_limit(), "no limit by default");
        map.set_limit(Some(2));
        map.insert(e(0), e(10));
        assert!(!map.at_limit());
        map.insert(e(1), e(11));
        assert!(map.at_limit());
        map.refuse();
        assert!(map.take_refused() && !map.take_refused());
        map.remove_by_client(e(10));
        assert!(!map.at_limit());
        map.insert(e(2), e(12));
        map.refuse();
        map.clear();
        assert!(!map.at_limit() && !map.take_refused());
        map.insert(e(3), e(13));
        map.insert(e(4), e(14));
        assert!(map.at_limit(), "the limit survives a clear");
    }
}
