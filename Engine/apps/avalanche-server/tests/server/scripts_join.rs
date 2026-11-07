use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use avalanche_server::config::Config;
use avalanche_server::{ServerOptions, ServerThread};
use ssx_net_join::{JoinFailure, JoinParams, join_showing};
use ssx_net_proto::join::{CHALLENGE_SIZE, HeldAsk, JoinAnswer, JoinContent, JoinRequest, RefusalCode};
use ssx_net_proto::keys::{PlayerSecret, ReplySecret, ServerKey};
use ssx_net_proto::{API_LEVEL, BUILD_ID, PROTOCOL_VERSION};
use zeroize::Zeroizing;

use crate::common::{Player, get, post, temp_dir};

const GATE: &str = r#"
addEventHandler("onPlayerConnecting", root, function(name, keyId, join)
    if name == "Probe" then
        refuseJoin(join, "probe")
        return
    end
    deferJoin(join)
    if name == "Refused" then
        updateJoin(join, "checking the list")
        setTimer(function() refuseJoin(join, "not on the list") end, 1500, 1)
    elseif name == "Gone" then
        updateJoin(join, "forever")
    else
        updateJoin(join, "the board looks you up")
        setTimer(function() updateJoin(join, "almost there") end, 1000, 1)
        setTimer(function() acceptJoin(join) end, 4000, 1)
    end
end)
"#;

fn gate(root: &Path) {
    let dir = root.join("gate");
    std::fs::create_dir_all(dir.join("server")).unwrap();
    let manifest =
        "[resource]\nname = \"gate\"\nversion = \"1.0.0\"\napi = 1\n\n[scripts]\nserver = [\"server/*.luau\"]\n";
    std::fs::write(dir.join("resource.toml"), manifest).unwrap();
    std::fs::write(dir.join("server/main.luau"), GATE).unwrap();
}

fn server(folder: &Path) -> ServerThread {
    let mut config = Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.resources.folder = folder.to_path_buf();
    config.resources.start = vec!["gate".into()];
    let options = ServerOptions {
        config,
        data_dir: temp_dir("scripts-join-data"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
    };
    avalanche_server::spawn(options).expect("the server starts")
}

fn from(c: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 7, c))
}

fn content(server: SocketAddr, local: IpAddr) -> JoinContent {
    let (status, body) = get(server, local, "/resources");
    assert_eq!(status, 200, "the manifest");
    JoinContent { content_id: None, manifest: Some(ssx_content::ContentHash::of(&body).0) }
}

fn join_by_hand(server: SocketAddr, key: &ServerKey, local: IpAddr, name: &str) -> (JoinAnswer, ReplySecret) {
    let (status, challenge) = get(server, local, "/challenge");
    assert_eq!(status, 200);
    let challenge: [u8; CHALLENGE_SIZE] = challenge.try_into().expect("40 bytes");
    let (player, reply) = (PlayerSecret::generate(), ReplySecret::generate());
    let request = JoinRequest {
        protocol_version: PROTOCOL_VERSION,
        api_level: API_LEVEL,
        reply: reply.public(),
        challenge,
        player: player.public(),
        server_addr: server,
        name: name.into(),
        build: BUILD_ID.into(),
        password: Zeroizing::new(Vec::new()),
        content: content(server, local),
    };
    let (status, answer) = post(server, local, "/join", &request.seal(key, &player).expect("seal"));
    assert_eq!(status, 200, "a sealed answer: {}", String::from_utf8_lossy(&answer));
    (JoinAnswer::open(&answer, &reply).expect("the answer opens"), reply)
}

fn ask(
    server: SocketAddr,
    key: &ServerKey,
    local: IpAddr,
    a: &HeldAsk,
    reply: &ReplySecret,
) -> (u16, Option<JoinAnswer>) {
    let (status, body) = post(server, local, "/join/held", &a.seal(key).expect("seal"));
    (status, (status == 200).then(|| JoinAnswer::open(&body, reply).expect("the answer opens")))
}

fn wait_for_scripts(server: SocketAddr, key: &ServerKey) {
    let start = Instant::now();
    for n in 1u8.. {
        let (answer, _) = join_by_hand(server, key, from(100 + n), "Probe");
        if matches!(&answer, JoinAnswer::Refused(r) if r.code == RefusalCode::Scripts && r.text == "probe") {
            return;
        }
        assert!(start.elapsed() < Duration::from_secs(20), "the scripts never answered a join: {answer:?}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn refusal(answer: &Option<JoinAnswer>) -> Option<(RefusalCode, String)> {
    match answer {
        Some(JoinAnswer::Refused(r)) => Some((r.code, r.text.clone())),
        _ => None,
    }
}

#[test]
fn a_held_join_shows_its_text_and_ends_with_the_scripts_word() {
    let folder = temp_dir("scripts-join");
    gate(&folder);
    let s = server(&folder);
    let (addr, key) = (s.started.addr, s.started.identity_key);
    wait_for_scripts(addr, &key);
    let mut player = Player::new();
    let shown = Mutex::new(Vec::<String>::new());
    let show = |t: &str| {
        let mut v = shown.lock().unwrap();
        if v.last().map(String::as_str) != Some(t) {
            v.push(t.to_string());
        }
    };
    let params =
        JoinParams { name: "Elise", password: b"", cancel: None, content: content(addr, crate::common::LOCAL) };
    let started = Instant::now();
    let granted = join_showing(addr, &player.identity, &mut player.known, &params, &show).expect("let through");
    assert!(started.elapsed() >= Duration::from_secs(3), "held until the scripts' word: {:?}", started.elapsed());
    assert_eq!(granted.token.len(), ssx_net_proto::join::TOKEN_SIZE);
    assert_eq!(*shown.lock().unwrap(), ["the board looks you up", "almost there"]);
    shown.lock().unwrap().clear();
    std::thread::sleep(Duration::from_millis(1100));
    let params =
        JoinParams { name: "Refused", password: b"", cancel: None, content: content(addr, crate::common::LOCAL) };
    match join_showing(addr, &player.identity, &mut player.known, &params, &show) {
        Err(JoinFailure::Refused(r)) => {
            assert_eq!((r.code, r.text.as_str()), (RefusalCode::Scripts, "not on the list"))
        }
        other => panic!("refused by the scripts, not {:?}", other.map(|g| g.info.name)),
    }
    assert_eq!(*shown.lock().unwrap(), ["checking the list"]);
    s.stop();
}

#[test]
fn a_held_joins_ticket_is_its_clients_alone() {
    let folder = temp_dir("scripts-join-ticket");
    gate(&folder);
    let s = server(&folder);
    let (addr, key) = (s.started.addr, s.started.identity_key);
    wait_for_scripts(addr, &key);
    let home = from(1);
    let (answer, reply) = join_by_hand(addr, &key, home, "Gone");
    let JoinAnswer::Held(hold) = answer else { panic!("held: {answer:?}") };
    assert_eq!((hold.text.as_str(), hold.ask_after_ms), ("forever", 1000));
    let right = HeldAsk { ticket: hold.ticket, reply: reply.public() };
    let (status, answer) = ask(addr, &key, from(2), &right, &reply);
    assert_eq!(status, 200);
    assert!(matches!(refusal(&answer), Some((RefusalCode::Request, t)) if t.contains("unknown")), "elsewhere");
    let stranger = ReplySecret::generate();
    let wrong_key = HeldAsk { ticket: hold.ticket, reply: stranger.public() };
    let (_, answer) = ask(addr, &key, home, &wrong_key, &stranger);
    assert!(matches!(refusal(&answer), Some((RefusalCode::Request, _))), "another reply key");
    let (status, answer) = ask(addr, &key, home, &right, &reply);
    assert_eq!(status, 200);
    let Some(JoinAnswer::Held(again)) = answer else { panic!("held again: {answer:?}") };
    assert_eq!(again.text, "forever");
    assert_ne!(again.ticket, hold.ticket, "a new ticket");
    let (_, answer) = ask(addr, &key, home, &right, &reply);
    assert!(matches!(refusal(&answer), Some((RefusalCode::Request, _))), "the used ticket");
    let flood = from(3);
    let statuses: Vec<u16> = (0..16u8)
        .map(|i| ask(addr, &key, flood, &HeldAsk { ticket: [i; 32], reply: reply.public() }, &reply).0)
        .collect();
    assert!(statuses.iter().filter(|s| **s == 429).count() >= 4, "{statuses:?}");
    assert!(statuses.iter().all(|s| *s == 200 || *s == 429), "{statuses:?}");
    let (status, _) = post(addr, from(4), "/join/held", &[0u8; 40]);
    assert_eq!(status, 400);
    let (status, _) = get(addr, from(4), "/join/held");
    assert_eq!(status, 405);
    s.stop();
}
