use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use avalanche_server::config::Config;
use avalanche_server::{ServerOptions, ServerThread};
use bevy::app::App;
use ssx_net::client::spawn_udp_client;
use ssx_net::harness::{Conditions, client_app};
use ssx_net::lightyear::prelude::{Connect, Connected, Link};
use ssx_net_join::{Granted, JoinFailure, JoinParams, KnownServers, PlayerIdentity, join};

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn temp_dir(what: &str) -> PathBuf {
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ssx-server-{what}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

pub fn server_with(edit: impl FnOnce(&mut Config), conditions: Option<Conditions>) -> ServerThread {
    let mut config = Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    edit(&mut config);
    let options = ServerOptions {
        config,
        data_dir: temp_dir("data"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions,
        relay_rates: None,
        extend: None,
        rules: avalanche_server::rules::RulesSource::Unchecked,
    };
    avalanche_server::spawn(options).expect("the server starts")
}

pub fn server() -> ServerThread {
    server_with(|_| {}, None)
}

pub fn raw(server: SocketAddr, local: IpAddr, request: &[u8], timeout: Duration) -> (u16, Vec<u8>) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let stream = runtime.block_on(async {
        let socket = tokio::net::TcpSocket::new_v4().expect("socket");
        socket.bind(SocketAddr::new(local, 0)).expect("bind");
        socket.connect(server).await.expect("connect").into_std().expect("std")
    });
    stream.set_nonblocking(false).expect("blocking");
    stream.set_read_timeout(Some(timeout)).expect("timeout");
    let mut stream = stream;
    let _ = stream.write_all(request);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    parse(&answer)
}

pub fn parse(answer: &[u8]) -> (u16, Vec<u8>) {
    let Some(end) = answer.windows(4).position(|w| w == b"\r\n\r\n") else { return (0, Vec::new()) };
    let head = String::from_utf8_lossy(&answer[..end]);
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, answer[end + 4..].to_vec())
}

pub fn get(server: SocketAddr, local: IpAddr, path: &str) -> (u16, Vec<u8>) {
    raw(server, local, format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes(), Duration::from_secs(5))
}

pub fn post(server: SocketAddr, local: IpAddr, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let mut request = format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    request.extend_from_slice(body);
    raw(server, local, &request, Duration::from_secs(5))
}

pub const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

pub struct Player {
    pub identity: PlayerIdentity,
    pub known: KnownServers,
}

impl Player {
    pub fn new() -> Self {
        let home = temp_dir("player");
        let identity = PlayerIdentity::load_or_create(&home).expect("identity");
        let known = KnownServers::load(&home).expect("known servers");
        Self { identity, known }
    }

    pub fn join(&mut self, server: SocketAddr, name: &str, password: &str) -> Result<Granted, JoinFailure> {
        let content = player_content(server);
        join(
            server,
            &self.identity,
            &mut self.known,
            &JoinParams { name, password: password.as_bytes(), cancel: None, content },
        )
    }
}

pub fn player_content(server: SocketAddr) -> ssx_net_proto::join::JoinContent {
    let never = std::sync::atomic::AtomicBool::new(false);
    ssx_net_join::resources::join_content(server, &never, |_| Err("a test player computes no levels' ids".into()))
        .expect("the server's manifest")
}

pub fn udp_client(
    token: &[u8; 2048],
    server: SocketAddr,
    local: IpAddr,
    conditions: Option<Conditions>,
) -> (App, bevy::prelude::Entity) {
    let auth = ssx_net::connect_token(token, server).expect("a token the client accepts");
    let mut app = client_app();
    let link = Link::default().with_conditioner(conditions.map(|c| c.conditioner().expect("conditions")));
    let entity = spawn_udp_client(app.world_mut(), SocketAddr::new(local, 0), server, auth, link).expect("client");
    app.world_mut().trigger(Connect { entity });
    (app, entity)
}

pub fn connected(app: &App, entity: bevy::prelude::Entity) -> bool {
    app.world().get::<Connected>(entity).is_some()
}

pub fn step_until(apps: &mut [&mut App], timeout: Duration, mut done: impl FnMut(&[&mut App]) -> bool) -> bool {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        for app in apps.iter_mut() {
            app.update();
        }
        if done(apps) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    false
}
