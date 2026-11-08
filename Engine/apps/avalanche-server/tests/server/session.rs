use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

use avalanche_server::{STOP_GRACE, ServerOptions};
use ssx_net::ServerSaid;
use ssx_net::lightyear::prelude::Disconnect;
use ssx_net_proto::GoodbyeReason;

use crate::common::*;

fn goodbye(app: &bevy::app::App) -> Option<GoodbyeReason> {
    app.world().resource::<ServerSaid>().goodbye.as_ref().map(|(r, _)| *r)
}

#[test]
fn a_token_is_used_once_and_from_the_joining_address() {
    let server = server();
    let addr = server.started.addr;
    let mut player = Player::new();
    let granted = player.join(addr, "Elise", "").expect("joined");
    let (mut first, e1) = udp_client(&granted.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut first], Duration::from_secs(5), |a| connected(a[0], e1)));
    first.world_mut().trigger(Disconnect { entity: e1 });
    step_until(&mut [&mut first], Duration::from_secs(1), |_| false);
    let (mut second, e2) = udp_client(&granted.token, addr, LOCAL, None);
    let told = step_until(&mut [&mut second], Duration::from_secs(6), |a| goodbye(a[0]) == Some(GoodbyeReason::Ticket));
    assert!(told, "the second use is refused and told why");
    assert!(step_until(&mut [&mut second], Duration::from_secs(4), |a| !connected(a[0], e2)));
    let granted = Player::new().join(addr, "Mac", "").expect("joined");
    let (mut elsewhere, e3) = udp_client(&granted.token, addr, IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), None);
    let told =
        step_until(&mut [&mut elsewhere], Duration::from_secs(6), |a| goodbye(a[0]) == Some(GoodbyeReason::Ticket));
    assert!(told, "a token from another address is refused");
    assert!(step_until(&mut [&mut elsewhere], Duration::from_secs(4), |a| !connected(a[0], e3)));
    server.stop();
    server.join().expect("stops");
}

#[test]
fn the_same_key_replaces_its_older_connection() {
    let server = server();
    let addr = server.started.addr;
    let mut player = Player::new();
    let g1 = player.join(addr, "Kaori", "").expect("joined");
    let (mut old, e1) = udp_client(&g1.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut old], Duration::from_secs(5), |a| connected(a[0], e1)));
    let g2 = player.join(addr, "Kaori", "").expect("joined again");
    let (mut new, e2) = udp_client(&g2.token, addr, LOCAL, None);
    let replaced = step_until(&mut [&mut old, &mut new], Duration::from_secs(8), |a| {
        goodbye(a[0]) == Some(GoodbyeReason::Replaced) && !connected(a[0], e1) && connected(a[1], e2)
    });
    assert!(replaced);
    server.stop();
    server.join().expect("stops");
}

#[test]
fn the_stop_tells_every_rider() {
    let server = server();
    let addr = server.started.addr;
    let granted = Player::new().join(addr, "Moby", "").expect("joined");
    let (mut app, entity) = udp_client(&granted.token, addr, LOCAL, None);
    assert!(step_until(&mut [&mut app], Duration::from_secs(5), |a| connected(a[0], entity)));
    let asked = Instant::now();
    server.stop();
    let told = step_until(&mut [&mut app], STOP_GRACE + Duration::from_secs(3), |a| {
        goodbye(a[0]) == Some(GoodbyeReason::ServerStopping) && !connected(a[0], entity)
    });
    assert!(told, "the rider is told and let go");
    server.join().expect("the server ends normally");
    assert!(asked.elapsed() < STOP_GRACE + Duration::from_secs(5), "{:?}", asked.elapsed());
}

#[test]
fn foreign_and_open_key_files_stop_the_start() {
    let data = temp_dir("keys");
    std::fs::write(data.join("identity.key"), "not a key\n").expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data.join("identity.key"), std::fs::Permissions::from_mode(0o600)).expect("chmod");
    }
    let mut config = crate::common::defaults();
    config.bind = LOCAL;
    config.port = 0;
    let options = |data: &std::path::Path| ServerOptions {
        config: config.clone(),
        data_dir: data.to_path_buf(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
    };
    let err = avalanche_server::spawn(options(&data)).err().expect("refused").to_string();
    assert!(err.contains("identity.key") && err.contains("move it away"), "{err}");
    assert_eq!(std::fs::read_to_string(data.join("identity.key")).expect("kept"), "not a key\n", "never replaced");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let open = temp_dir("keys-open");
        let started = avalanche_server::spawn(options(&open)).expect("starts and makes its keys");
        started.stop();
        started.join().expect("stops");
        std::fs::set_permissions(open.join("netcode.key"), std::fs::Permissions::from_mode(0o644)).expect("chmod");
        let err = avalanche_server::spawn(options(&open)).err().expect("refused").to_string();
        assert!(err.contains("chmod 600"), "{err}");
    }
}
