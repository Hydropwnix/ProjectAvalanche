use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use avalanche_server::config::Config;
use avalanche_server::operation::keys;
use avalanche_server::operation::rcon_client::{RconOptions, run};
use avalanche_server::{ServerOptions, ServerThread};
use ssx_net_proto::join::CHALLENGE_SIZE;
use ssx_net_proto::keys::ReplySecret;
use ssx_net_proto::rcon::{OperatorSecret, RCON_VERSION, RconAnswer, RconRefusalCode, RconRequest};

use crate::common::*;

pub struct Ops {
    pub server: ServerThread,
    pub addr: SocketAddr,
    pub data: PathBuf,
    pub admin: PathBuf,
    pub helper: PathBuf,
    pub admin_secret: OperatorSecret,
}

pub const ACL: &str = r#"[[group]]
name = "admin"
objects = ["operator.{ADMIN}"]
allow = ["command"]
deny = []

[[group]]
name = "helper"
objects = ["operator.{LIMITED}"]
allow = ["command.status"]
deny = []
"#;

pub fn ops_with(edit: impl FnOnce(&mut Config), bans: &str) -> Ops {
    ops_with_data(edit, bans, |_, _| {}).expect("the server starts")
}

pub fn ops_with_data(
    edit: impl FnOnce(&mut Config),
    bans: &str,
    prepare: impl FnOnce(&Path, &Path),
) -> Result<Ops, avalanche_server::StartError> {
    let keys_dir = temp_dir("operator-keys");
    let (admin, helper) = (keys_dir.join("admin.key"), keys_dir.join("helper.key"));
    let admin_secret = keys::generate(&admin).expect("the admin's key");
    let helper_secret = keys::generate(&helper).expect("the helper's key");
    let data = temp_dir("data");
    let acl =
        ACL.replace("{ADMIN}", &admin_secret.public().to_hex()).replace("{LIMITED}", &helper_secret.public().to_hex());
    std::fs::write(data.join("acl.toml"), acl).expect("acl.toml");
    std::fs::write(data.join("bans.toml"), bans).expect("bans.toml");
    let conf = temp_dir("config");
    prepare(&data, &conf);
    let mut config = crate::common::defaults();
    config.path = Some(conf.join("server.toml"));
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    edit(&mut config);
    let server = avalanche_server::spawn(ServerOptions {
        config,
        data_dir: data.clone(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
    })?;
    let addr = server.started.addr;
    Ok(Ops { server, addr, data, admin, helper, admin_secret })
}

pub fn rcon(o: &Ops, key: &Path, command: &str) -> RconAnswer {
    let options = RconOptions {
        address: o.addr.to_string(),
        key: key.to_path_buf(),
        server_key: Some(o.server.started.identity_key.to_hex()),
        accept_new: false,
        command: command.into(),
    };
    run(&options).unwrap_or_else(|e| panic!("{command}: {e}")).1
}

pub fn ran(o: &Ops, command: &str) -> Vec<String> {
    match rcon(o, &o.admin, command) {
        RconAnswer::Ran { ok: true, lines } => lines,
        other => panic!("{command}: {other:?}"),
    }
}

fn refused(answer: &RconAnswer) -> Option<RconRefusalCode> {
    match answer {
        RconAnswer::Refused(r) => Some(r.code),
        RconAnswer::Ran { .. } => None,
    }
}

#[test]
fn the_remote_console_runs_the_consoles_commands_for_an_operator() {
    let o = ops_with(|_| {}, "");
    let status = ran(&o, "status");
    assert!(!status.is_empty(), "status answers lines");
    let instances = ran(&o, "instances");
    assert!(instances.iter().any(|l| l.contains("instance")), "{instances:?}");
    let said = ran(&o, "say the operator speaks");
    assert!(said.iter().any(|l| l.contains("the operator speaks")), "{said:?}");
    let granted = Player::new().join(o.addr, "Elise", "").expect("joined");
    let (mut rider, e) = udp_client(&granted.token, o.addr, LOCAL, None);
    assert!(step_until(&mut [&mut rider], Duration::from_secs(5), |a| connected(a[0], e)));
    let kicked = ran(&o, "kick Elise cheating is not riding");
    assert!(!kicked.is_empty(), "{kicked:?}");
    let told = step_until(&mut [&mut rider], Duration::from_secs(5), |a| {
        a[0].world().resource::<ssx_net::ServerSaid>().goodbye.as_ref().is_some_and(|(_, t)| t.contains("cheating"))
    });
    assert!(told, "the kicked rider is told the reason");
    let ban = ran(&o, "ban ip:192.0.2.7 2h a test ban");
    assert!(ban.iter().any(|l| l.contains("192.0.2.7")), "{ban:?}");
    assert!(ran(&o, "bans").iter().any(|l| l.contains("a test ban")));
    assert!(ran(&o, "unban ip:192.0.2.7").iter().any(|l| l.contains("192.0.2.7")));
    let mute = ran(&o, &format!("mute player:{} 30m too loud", "cd".repeat(32)));
    assert!(!mute.is_empty(), "{mute:?}");
    assert!(ran(&o, "mutes").iter().any(|l| l.contains("too loud")));
    let groups = ran(&o, "acl list");
    assert!(groups.iter().any(|l| l.contains("admin")) && groups.iter().any(|l| l.contains("helper")), "{groups:?}");
    assert!(ran(&o, "acl create moderators").iter().any(|l| l.contains("moderators")));
    assert!(std::fs::read_to_string(o.data.join("acl.toml")).expect("acl.toml").contains("moderators"));
    assert!(matches!(rcon(&o, &o.helper, "status"), RconAnswer::Ran { ok: true, .. }));
    assert_eq!(refused(&rcon(&o, &o.helper, "ban ip:192.0.2.8 1h")), Some(RconRefusalCode::Denied));
    assert_eq!(refused(&rcon(&o, &o.helper, "no-such-command")), Some(RconRefusalCode::Command));
    assert!(ran(&o, "stop").iter().any(|l| l.contains("stopping")));
    o.server.join().expect("stops after the remote stop");
}

fn sealed_request(o: &Ops, local: IpAddr, signer: &OperatorSecret, command: &str) -> (Vec<u8>, ReplySecret) {
    let (status, challenge) = get(o.addr, local, "/rcon/challenge");
    assert_eq!(status, 200, "a challenge");
    let challenge: [u8; CHALLENGE_SIZE] = challenge.try_into().expect("challenge size");
    let reply = ReplySecret::generate();
    let request = RconRequest {
        rcon_version: RCON_VERSION,
        reply: reply.public(),
        challenge,
        operator: signer.public(),
        command: command.into(),
    };
    (request.seal(&o.server.started.identity_key, signer).expect("sealed"), reply)
}

fn answer(o: &Ops, local: IpAddr, body: &[u8], reply: &ReplySecret) -> RconAnswer {
    let (status, body) = post(o.addr, local, "/rcon", body);
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    RconAnswer::open(&body, reply).expect("a sealed answer")
}

fn from(n: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 4, n))
}

#[test]
fn the_remote_console_refuses_hostile_requests() {
    let o = ops_with(|_| {}, "");
    let (body, reply) = sealed_request(&o, from(1), &o.admin_secret, "status");
    assert!(matches!(answer(&o, from(1), &body, &reply), RconAnswer::Ran { ok: true, .. }));
    assert_eq!(refused(&answer(&o, from(1), &body, &reply)), Some(RconRefusalCode::Challenge), "replayed");
    let (body, reply) = sealed_request(&o, from(2), &o.admin_secret, "status");
    assert_eq!(refused(&answer(&o, from(3), &body, &reply)), Some(RconRefusalCode::Challenge), "another address");
    let stranger = OperatorSecret::generate();
    let (body, reply) = sealed_request(&o, from(4), &stranger, "status");
    assert_eq!(refused(&answer(&o, from(4), &body, &reply)), Some(RconRefusalCode::UnknownKey));
    let (status, _) = post(o.addr, from(5), "/rcon", &[7u8; 300]);
    assert_eq!(status, 400, "random bytes do not open");
    let (mut body, _) = sealed_request(&o, from(6), &o.admin_secret, "status");
    let last = body.len() - 1;
    body[last] ^= 0x40;
    assert_eq!(post(o.addr, from(6), "/rcon", &body).0, 400, "an altered body does not open");
    let (body, _) = sealed_request(&o, from(7), &o.admin_secret, "status");
    assert_eq!(post(o.addr, from(7), "/rcon", &body[..body.len() / 2]).0, 400, "a cut body does not open");
    let big = vec![0u8; ssx_net_proto::rcon::MAX_REQUEST_BODY + 1];
    assert_eq!(post(o.addr, from(8), "/rcon", &big).0, 413);
    let head = format!(
        "POST /rcon HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
        ssx_net_proto::rcon::MAX_REQUEST_BODY + 1
    );
    let mut chunked = head.into_bytes();
    chunked.extend_from_slice(&big);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    assert_eq!(raw(o.addr, from(9), &chunked, Duration::from_secs(5)).0, 413, "a chunked body over the cap");
    let started = std::time::Instant::now();
    let (status, _) = raw(
        o.addr,
        from(10),
        b"POST /rcon HTTP/1.1\r\nHost: x\r\nContent-Length: 200\r\n\r\nabc",
        Duration::from_secs(30),
    );
    assert!(status == 0 || status == 408 || status == 413, "a slow body: {status}");
    assert!(started.elapsed() < Duration::from_secs(25), "the slow body was ended by the server's own bound");
    for _ in 0..6 {
        let _ = post(o.addr, from(11), "/rcon", &[1u8; 100]);
    }
    assert_eq!(post(o.addr, from(11), "/rcon", &[1u8; 100]).0, 429, "the address waits after its failures");
    o.server.stop();
    o.server.join().expect("stops");
}

#[test]
fn the_remote_console_is_off_without_an_operator() {
    let server = server();
    let addr = server.started.addr;
    assert_eq!(get(addr, LOCAL, "/rcon/challenge").0, 404);
    assert_eq!(post(addr, LOCAL, "/rcon", &[0u8; 64]).0, 404);
    server.stop();
    server.join().expect("stops");
}

#[test]
fn operator_keys_are_never_replaced_and_a_changed_server_key_is_refused() {
    let dir = temp_dir("operator-key");
    let path = dir.join("op.key");
    keys::generate(&path).expect("made");
    assert!(keys::generate(&path).is_err(), "never replaced");
    let o = ops_with(|_| {}, "");
    let mut options = RconOptions {
        address: o.addr.to_string(),
        key: o.admin.clone(),
        server_key: None,
        accept_new: false,
        command: "status".into(),
    };
    let e = run(&options).expect_err("not pinned yet");
    assert!(e.contains("not pinned"), "{e}");
    options.accept_new = true;
    let (notes, answer) = run(&options).expect("pinned on first use");
    assert!(notes.iter().any(|n| n.contains("pinned")) && matches!(answer, RconAnswer::Ran { ok: true, .. }));
    let pins = o.admin.parent().expect("dir").join("rcon-servers.toml");
    let text = std::fs::read_to_string(&pins).expect("pins");
    std::fs::write(&pins, text.replace(&o.server.started.identity_key.to_hex(), &"ab".repeat(32))).expect("write");
    let e = run(&options).expect_err("a changed key");
    assert!(e.contains("refused"), "{e}");
    o.server.stop();
    o.server.join().expect("stops");
}
