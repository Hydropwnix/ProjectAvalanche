use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};

use bevy_ecs::prelude::*;
use lightyear::netcode::ConnectToken;
use ssx_net_proto::join::TOKEN_SIZE;
use ssx_net_proto::keys::{PlayerId, PlayerKey, random_bytes};
use ssx_net_proto::netcode_protocol_id;
use ssx_net_proto::ticket::{SessionTicket, TICKET_SIZE};
use zeroize::Zeroizing;

use crate::address::{AddressKey, address_key, same_address};
use crate::limits::{
    CONNECTION_TIMEOUT_SECS, MAX_TICKETS, MAX_TICKETS_PER_ADDRESS, TICKET_LIFETIME, TOKEN_EXPIRE_SECS,
};

#[derive(Clone, Debug)]
pub struct TicketRequest {
    pub player: PlayerKey,
    pub name: String,
    pub address: IpAddr,
    pub public_addr: SocketAddr,
    pub manifest: Option<[u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueError {
    Full,
    Address,
    Token,
    NoServer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TicketRefusal {
    Missing,
    Unknown,
    Used,
    Mismatch,
    Address,
}

struct Ticket {
    client_id: u64,
    player: PlayerKey,
    name: String,
    address: IpAddr,
    expires_at: Duration,
    claimed: bool,
    manifest: Option<[u8; 32]>,
}

pub struct Issued {
    pub token: Box<[u8; TOKEN_SIZE]>,
    pub client_id: u64,
}

#[derive(Clone, Debug)]
pub struct Claimed {
    pub join_id: u64,
    pub player: PlayerKey,
    pub name: String,
    pub manifest: Option<[u8; 32]>,
}

#[derive(Resource, Default)]
pub struct Sessions {
    netcode_key: Option<Zeroizing<[u8; 32]>>,
    tickets: HashMap<u64, Ticket>,
    client_ids: HashSet<u64>,
}

impl Sessions {
    pub fn set_netcode_key(&mut self, key: [u8; 32]) {
        self.netcode_key = Some(Zeroizing::new(key));
    }

    pub fn expire(&mut self, now: Duration) {
        let client_ids = &mut self.client_ids;
        self.tickets.retain(|_, t| {
            let keep = t.expires_at > now;
            if !keep && !t.claimed {
                client_ids.remove(&t.client_id);
            }
            keep
        });
    }

    pub fn reserved(&self) -> usize {
        self.tickets.values().filter(|t| !t.claimed).count()
    }

    pub fn reserved_for(&self, player: &PlayerKey) -> bool {
        self.tickets.values().any(|t| !t.claimed && t.player == *player)
    }

    pub fn held(&self) -> usize {
        self.tickets.len()
    }

    pub fn issue(
        &mut self,
        now: Duration,
        server_addr: SocketAddr,
        request: &TicketRequest,
    ) -> Result<Issued, IssueError> {
        let Some(netcode_key) = self.netcode_key.as_deref().copied() else { return Err(IssueError::NoServer) };
        self.expire(now);
        let client_ids = &mut self.client_ids;
        self.tickets.retain(|_, t| {
            let revoke = !t.claimed && t.player == request.player;
            if revoke {
                client_ids.remove(&t.client_id);
            }
            !revoke
        });
        if self.tickets.len() >= MAX_TICKETS {
            return Err(IssueError::Full);
        }
        let key: AddressKey = address_key(request.address);
        let from_address = self.tickets.values().filter(|t| !t.claimed && address_key(t.address) == key).count();
        if from_address >= MAX_TICKETS_PER_ADDRESS {
            return Err(IssueError::Address);
        }
        let join_id = loop {
            let id = u64::from_be_bytes(random_bytes());
            if id != 0 && !self.tickets.contains_key(&id) {
                break id;
            }
        };
        let client_id = loop {
            let id = u64::from_be_bytes(random_bytes());
            if id != 0 && !self.client_ids.contains(&id) {
                break id;
            }
        };
        let ticket = SessionTicket { join_id, player: request.player };
        let token = ConnectToken::build(request.public_addr, netcode_protocol_id(), client_id, netcode_key)
            .internal_addresses(server_addr)
            .map_err(|_| IssueError::Token)?
            .expire_seconds(TOKEN_EXPIRE_SECS)
            .timeout_seconds(CONNECTION_TIMEOUT_SECS)
            .user_data(ticket.encode())
            .generate()
            .map_err(|_| IssueError::Token)?;
        let bytes = token.try_into_bytes().map_err(|_| IssueError::Token)?;
        self.client_ids.insert(client_id);
        self.tickets.insert(
            join_id,
            Ticket {
                client_id,
                player: request.player,
                name: request.name.clone(),
                address: request.address,
                expires_at: now + TICKET_LIFETIME,
                claimed: false,
                manifest: request.manifest,
            },
        );
        Ok(Issued { token: Box::new(bytes), client_id })
    }

    pub fn claim(
        &mut self,
        now: Duration,
        user_data: &[u8; TICKET_SIZE],
        client_id: u64,
        peer: IpAddr,
    ) -> Result<Claimed, TicketRefusal> {
        let ticket = SessionTicket::decode(user_data).map_err(|_| TicketRefusal::Missing)?;
        self.expire(now);
        let t = self.tickets.get_mut(&ticket.join_id).ok_or(TicketRefusal::Unknown)?;
        if t.claimed {
            return Err(TicketRefusal::Used);
        }
        if t.client_id != client_id || t.player != ticket.player {
            return Err(TicketRefusal::Mismatch);
        }
        if !same_address(t.address, peer) {
            return Err(TicketRefusal::Address);
        }
        t.claimed = true;
        Ok(Claimed { join_id: ticket.join_id, player: t.player, name: t.name.clone(), manifest: t.manifest })
    }

    pub(super) fn release(&mut self, client_id: u64) {
        self.client_ids.remove(&client_id);
    }

    pub fn tickets(&self) -> Vec<(u64, u64, PlayerKey, bool)> {
        self.tickets.iter().map(|(&id, t)| (id, t.client_id, t.player, t.claimed)).collect()
    }

    pub fn state_sizes(&self) -> (usize, usize) {
        (self.tickets.len(), self.client_ids.len())
    }
}

#[derive(Component, Clone, Debug)]
pub struct PlayerSession {
    pub player: PlayerKey,
    pub id: PlayerId,
    pub name: String,
    pub join_id: u64,
    pub client_id: u64,
    pub address: IpAddr,
}
