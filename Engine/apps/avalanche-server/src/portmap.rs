use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use ssx_net_portmap::{Config as MapConfig, Level, Mapper, Port, Protocol, Status, Targets};

use crate::config::Config;

pub const MAPPING_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Resource, Clone, Default)]
pub struct PortmapTargets(pub Targets);

pub fn ports(config: &Config, game: SocketAddr) -> Vec<Port> {
    let mut v =
        vec![Port { protocol: Protocol::Udp, port: game.port() }, Port { protocol: Protocol::Tcp, port: game.port() }];
    if config.discovery.query
        && let Some(q) = game.port().checked_add(ssx_net_proto::query::QUERY_PORT_OFFSET)
    {
        v.push(Port { protocol: Protocol::Udp, port: q });
    }
    v
}

pub fn start(config: &Config, game: SocketAddr, targets: Targets) -> Result<Option<Mapper>, String> {
    let m = &config.port_mapping;
    if !m.enabled {
        return Ok(None);
    }
    let local = match ssx_net::address::canonical(config.bind) {
        IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
        _ => None,
    };
    let name: String = config.name.chars().filter(|c| !c.is_control()).take(40).collect();
    let cfg = MapConfig {
        methods: m.methods.clone(),
        gateway: m.gateway,
        lifetime: m.lifetime,
        local,
        ports: ports(config, game),
        description: format!("Project Avalanche: {name}"),
    };
    let log: ssx_net_portmap::Log = Arc::new(|level, line: &str| match level {
        Level::Info => bevy::log::info!(target: "avalanche_server::portmap", "{line}"),
        Level::Warn => bevy::log::warn!(target: "avalanche_server::portmap", "{line}"),
    });
    Mapper::start(cfg, targets, log).map(Some).map_err(|e| format!("the port mapping's thread: {e}"))
}

pub fn lines(status: &Mutex<Status>, masters: &[(String, String)]) -> Vec<String> {
    let Ok(s) = status.lock() else { return Vec::new() };
    let mut out = vec![format!(
        "port mapping: {} (maps {}, renewals {}, failures {}, removed {})",
        s.summary, s.maps, s.renewals, s.failures, s.removals
    )];
    if s.mapped {
        for (url, last) in masters {
            let verdict = if last.starts_with("listed") {
                "mapped, and reachable from the master"
            } else {
                "mapped, but the master did not list the server"
            };
            out.push(format!("  {verdict} {url}: {last}"));
        }
    }
    out
}
