use core::time::Duration;

use ssx_net_proto::info::{INFO_VERSION, InfoHead, ServerInfo};
use ssx_net_proto::{API_LEVEL, BUILD_ID, PROTOCOL_VERSION};

use super::address::{Host, ServerAddress};
use super::list::{Entry, State};

pub const MAX_FAKE: usize = 10_000;

const FIRST: [&str; 16] = [
    "Alps",
    "Night",
    "Powder",
    "Deadly",
    "Rocky",
    "Frozen",
    "Summit",
    "Avalanche",
    "Glacier",
    "Peak",
    "Storm",
    "Thin Air",
    "Backcountry",
    "Wingsuit",
    "Big Air",
    "Zoë's",
];
const SECOND: [&str; 12] = [
    "Riders",
    "Descent",
    "Crew",
    "Freeride",
    "Trick Masters",
    "Lines",
    "Club",
    "Session",
    "Drop",
    "Racers",
    "Lounge",
    "EU",
];
const PLAYERS: [&str; 12] =
    ["Elise", "Mac", "Kaori", "Zoe", "Psymon", "Moby", "Tane", "Alex", "Griff", "Ty", "Maddy", "Marisol"];
const MODES: [&str; 4] = ["freeride", "race", "trick", "survive"];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

pub fn fake_servers(count: usize, drops: &[String]) -> Vec<Entry> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    (0..count.min(MAX_FAKE))
        .map(|i| {
            let address = ServerAddress {
                host: Host::Ip(std::net::IpAddr::V4(std::net::Ipv4Addr::new(192, 0, 2, (i % 254 + 1) as u8))),
                port: 27450 + (i / 254) as u16,
            };
            let name = format!("{} {} {}", FIRST[rng.below(FIRST.len())], SECOND[rng.below(SECOND.len())], i + 1);
            let max_players = [4u32, 8, 16, 32, 64][rng.below(5)];
            let players = match rng.below(4) {
                0 => 0,
                1 => max_players,
                _ => rng.below(max_players as usize + 1) as u32,
            };
            let drop = drops.get(rng.below(drops.len())).cloned().unwrap_or_else(|| "exp_alps_vorassay_drop_3".into());
            let mut entry = Entry::new(address, Some(name.clone()));
            entry.fake = true;
            entry.ping = Some(Duration::from_millis(5 + rng.below(300) as u64));
            entry.players =
                Some((0..players).map(|k| format!("{} {}", PLAYERS[(i + k as usize) % 12], k + 1)).collect());
            entry.state = if i % 25 == 24 {
                let head =
                    InfoHead { info_version: INFO_VERSION, protocol_version: PROTOCOL_VERSION + 1, name: Some(name) };
                State::Incompatible {
                    head,
                    why: format!("the server speaks protocol {}, this game {PROTOCOL_VERSION}", PROTOCOL_VERSION + 1),
                }
            } else if i % 40 == 39 {
                let head =
                    InfoHead { info_version: INFO_VERSION + 1, protocol_version: PROTOCOL_VERSION, name: Some(name) };
                State::Incompatible {
                    head,
                    why: format!("the server's info is version {}, this game reads {INFO_VERSION}", INFO_VERSION + 1),
                }
            } else {
                State::Info(Box::new(ServerInfo {
                    info_version: INFO_VERSION,
                    name,
                    description: format!("A generated server of the fake list (number {}).", i + 1),
                    protocol_version: PROTOCOL_VERSION,
                    api_level: API_LEVEL,
                    build: BUILD_ID.into(),
                    players,
                    max_players,
                    password: rng.below(5) == 0,
                    mode: MODES[rng.below(MODES.len())].into(),
                    drop,
                    map: None,
                    rewinds: ["on", "off", "3"][rng.below(3)].into(),
                    ghosts: rng.below(2) == 0,
                    ghost_style: ["character", "hologram"][rng.below(2)].into(),
                    identity_key: "0".repeat(64),
                    resources: {
                        let count = if rng.below(4) == 0 { 0 } else { 1 + rng.below(6) as u32 };
                        let download = u64::from(count) * (rng.below(512) as u64 + 1) * (1 << 20);
                        ssx_net_proto::resources::ResourceSummary {
                            content_id: format!("{:064x}", i + 1),
                            client_mods: ["presentation", "any", "none"][rng.below(3)].into(),
                            client_scripts: rng.below(2) == 0,
                            count,
                            download,
                            manifest: format!("{:064x}", i + 1),
                            manifest_bytes: 400 + 300 * u64::from(count),
                        }
                    },
                    region: String::new(),
                    tags: Vec::new(),
                    icon: String::new(),
                }))
            };
            entry.need = entry.info().filter(|s| s.resources.count > 0).map(|s| match i % 3 {
                0 => crate::resources::Need::Exactly(0),
                _ => crate::resources::Need::Between { least: s.resources.download, most: s.resources.download },
            });
            entry
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_list_is_the_same_for_a_count_and_keeps_the_rules() {
        let drops = vec!["exp_alps_vorassay_drop_3".to_string(), "exp_rockies_robson_drop_1".to_string()];
        let a = fake_servers(500, &drops);
        assert_eq!(a, fake_servers(500, &drops));
        assert_eq!(a.len(), 500);
        let mut seen = std::collections::HashSet::new();
        for e in &a {
            assert!(seen.insert(e.address.clone()), "{} twice", e.address);
            assert!(ssx_net_proto::name::server_name(&e.name()).is_ok_and(|n| n == e.name()), "{}", e.name());
            if let Some(i) = e.info() {
                assert!(i.players <= i.max_players && e.players.as_ref().map(Vec::len) == Some(i.players as usize));
            }
        }
        assert_eq!(a.iter().filter(|e| matches!(e.state, State::Incompatible { .. })).count(), 20 + 12 - 2);
        assert_eq!(fake_servers(MAX_FAKE + 5, &drops).len(), MAX_FAKE);
    }
}
