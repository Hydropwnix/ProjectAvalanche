use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use ssx_net::{ChatInbox, ChatSend, ServerSaid};
use ssx_net_join::JoinFailure;
use ssx_net_proto::join::RefusalCode;
use ssx_net_proto::operator::Expiry;
use ssx_net_proto::{ChatRefusal, ClientChat, GoodbyeReason};

use crate::common::*;
use crate::operation::{Ops, ops_with, ran};

fn join_paced(player: &mut Player, addr: SocketAddr) -> Result<ssx_net_join::Granted, JoinFailure> {
    for _ in 0..10 {
        let wait = match player.join(addr, "Zoe", "") {
            Err(JoinFailure::Http { status: 429, retry_after_secs, .. }) => retry_after_secs.unwrap_or(1).clamp(1, 5),
            Err(JoinFailure::Refused(r)) if r.code == RefusalCode::Flood => u64::from(r.retry_after_secs).clamp(1, 10),
            other => return other,
        };
        std::thread::sleep(Duration::from_secs(wait));
    }
    panic!("the address's limits never let the test in")
}

fn banned(player: &mut Player, addr: SocketAddr) -> (String, Option<Expiry>) {
    match join_paced(player, addr) {
        Err(JoinFailure::Refused(r)) if r.code == RefusalCode::Banned => (r.text, r.ban),
        Err(e) => panic!("another failure: {e}"),
        Ok(_) => panic!("joined although banned"),
    }
}

fn joins(player: &mut Player, addr: SocketAddr) -> bool {
    join_paced(player, addr).is_ok()
}

#[test]
fn a_ban_by_id_address_wildcard_refuses_the_join_with_its_reason_and_end() {
    let o = ops_with(|_| {}, "");
    let mut player = Player::new();
    assert!(joins(&mut player, o.addr), "not banned yet");
    let id = player.identity.id().to_hex();
    ran(&o, &format!("ban player:{id} 2h a ban by id"));
    let (text, until) = banned(&mut player, o.addr);
    assert!(text.contains("a ban by id"), "{text}");
    assert!(matches!(until, Some(Expiry::At(_))), "the end is told: {until:?}");
    assert!(joins(&mut Player::new(), o.addr), "another player is not banned");
    ran(&o, &format!("unban player:{id}"));
    assert!(joins(&mut player, o.addr), "unbanned");
    ran(&o, "ban ip:127.0.0.1 forever a ban by address");
    let (text, until) = banned(&mut Player::new(), o.addr);
    assert!(text.contains("a ban by address"), "{text}");
    assert_eq!(until, Some(Expiry::Never), "for good");
    ran(&o, "unban 1");
    ran(&o, "ban range:127.0.*.* 1d a wildcard");
    assert!(banned(&mut Player::new(), o.addr).0.contains("a wildcard"));
    ran(&o, "unban range:127.0.*.*");
    assert!(joins(&mut Player::new(), o.addr), "the wildcard's ban is gone");
    let text = std::fs::read_to_string(o.data.join("bans.toml")).expect("bans.toml");
    assert!(!text.contains("127.0.*.*"), "{text}");
    o.server.stop();
    o.server.join().expect("stops");
}

fn ops_on(bind: IpAddr, bans: &str) -> Option<Ops> {
    std::net::UdpSocket::bind((Ipv6Addr::LOCALHOST, 0)).ok()?;
    Some(ops_with(|c| c.bind = bind, bans))
}

#[test]
fn an_ipv6_prefix_and_a_mapped_ipv4_peer_are_banned() {
    let at = Expiry::from_wire(4_102_444_800).to_wire();
    let bans =
        format!("[[ban]]\nrange = \"::/120\"\nreason = \"a v6 prefix\"\nby = \"console\"\nat = 1\nuntil = {at}\n");
    let Some(o) = ops_on(IpAddr::V6(Ipv6Addr::LOCALHOST), &bans) else { return };
    let (text, until) = banned(&mut Player::new(), o.addr);
    assert!(text.contains("a v6 prefix"), "{text}");
    assert_eq!(until.map(Expiry::to_wire), Some(at));
    o.server.stop();
    o.server.join().expect("stops");
    let bans = "[[ban]]\nip = \"127.0.0.1\"\nreason = \"mapped\"\nby = \"console\"\nat = 1\nuntil = 0\n";
    let mapped = IpAddr::V6(Ipv4Addr::LOCALHOST.to_ipv6_mapped());
    let Some(o) = ops_on(mapped, bans) else { return };
    let v4 = SocketAddr::new(LOCAL, o.addr.port());
    assert!(banned(&mut Player::new(), v4).0.contains("mapped"));
    o.server.stop();
    o.server.join().expect("stops");
}

fn said(app: &bevy::app::App) -> &ServerSaid {
    app.world().resource::<ServerSaid>()
}

#[test]
fn a_connected_rider_banned_is_sent_away_and_a_passed_ban_holds_no_one() {
    let o = ops_with(|_| {}, "");
    let mut player = Player::new();
    let granted = player.join(o.addr, "Moby", "").expect("joined");
    let (mut rider, e) = udp_client(&granted.token, o.addr, LOCAL, None);
    assert!(step_until(&mut [&mut rider], Duration::from_secs(5), |a| connected(a[0], e)));
    let lines = ran(&o, "ban Moby 3s no shortcuts");
    assert!(lines.iter().any(|l| l.contains("Moby")), "{lines:?}");
    let told = step_until(&mut [&mut rider], Duration::from_secs(5), |a| {
        said(a[0]).goodbye.as_ref().is_some_and(|(r, t)| *r == GoodbyeReason::Banned && t.contains("no shortcuts"))
            && matches!(said(a[0]).ban, Some(Expiry::At(_)))
    });
    assert!(told, "the goodbye Banned with its reason and its end");
    assert!(step_until(&mut [&mut rider], Duration::from_secs(4), |a| !connected(a[0], e)));
    assert!(banned(&mut player, o.addr).0.contains("no shortcuts"), "the id and the address are banned");
    std::thread::sleep(Duration::from_secs(4));
    assert!(joins(&mut player, o.addr), "the ban's end passed");
    o.server.stop();
    o.server.join().expect("stops");
}

fn send(app: &mut bevy::app::App, text: &str) {
    app.world_mut().resource_mut::<ChatSend>().0.push(ClientChat::text(text).expect("a chat line"));
}

fn heard(app: &bevy::app::App, text: &str) -> bool {
    app.world().resource::<ChatInbox>().lines.iter().any(|l| format!("{:?}", l.body).contains(text))
}

#[test]
fn a_muted_riders_lines_are_refused_to_it_and_reach_no_one() {
    let o = ops_with(|_| {}, "");
    let a = Player::new().join(o.addr, "Kaori", "").expect("joined");
    let b = Player::new().join(o.addr, "Mac", "").expect("joined");
    let (mut ka, ea) = udp_client(&a.token, o.addr, LOCAL, None);
    let (mut mac, eb) = udp_client(&b.token, o.addr, LOCAL, None);
    let both = |apps: &[&mut bevy::app::App]| {
        connected(apps[0], ea)
            && connected(apps[1], eb)
            && said(apps[0]).welcome.is_some()
            && said(apps[1]).welcome.is_some()
    };
    assert!(step_until(&mut [&mut ka, &mut mac], Duration::from_secs(8), both));
    send(&mut ka, "before the mute");
    assert!(step_until(&mut [&mut ka, &mut mac], Duration::from_secs(5), |a| heard(a[1], "before the mute")));
    ran(&o, "mute Kaori 10m too many capitals");
    send(&mut ka, "after the mute");
    let refused = step_until(&mut [&mut ka, &mut mac], Duration::from_secs(5), |a| {
        a[0].world().resource::<ChatInbox>().refusals.iter().any(|(r, _)| {
            matches!(r, ChatRefusal::Muted { until, reason } if *until != Expiry::Never && reason == "too many capitals")
        })
    });
    assert!(refused, "the muted rider is told, with the end and the reason");
    step_until(&mut [&mut ka, &mut mac], Duration::from_secs(2), |_| false);
    assert!(!heard(&mac, "after the mute"), "no one else hears the muted line");
    send(&mut mac, "the others still talk");
    assert!(step_until(&mut [&mut ka, &mut mac], Duration::from_secs(5), |a| heard(a[0], "the others still talk")));
    ran(&o, "unmute Kaori");
    send(&mut ka, "after the unmute");
    assert!(step_until(&mut [&mut ka, &mut mac], Duration::from_secs(5), |a| heard(a[1], "after the unmute")));
    o.server.stop();
    o.server.join().expect("stops");
}
