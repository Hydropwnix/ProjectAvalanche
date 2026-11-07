use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use avalanche_master::config::Config;
use avalanche_master::{MasterOptions, MasterThread};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper_util::rt::TokioIo;
use ssx_net_proto::info::{INFO_VERSION, ServerInfo};
use ssx_net_proto::keys::{ServerSecret, random_bytes};
use ssx_net_proto::master::{HEARTBEAT_VERSION, Heartbeat, HeartbeatAnswer, MasterChallenge};
use ssx_net_proto::query::{Challenge, QueryAnswer, QueryRequest, info_hash, mode_code};

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn next_ip() -> IpAddr {
    let n = NEXT.fetch_add(1, Ordering::Relaxed) + 1;
    IpAddr::V4(Ipv4Addr::new(127, 0, 20 + (n / 250) as u8, (n % 250) as u8 + 1))
}

pub fn master_with(edit: impl FnOnce(&mut Config)) -> MasterThread {
    let mut config = Config::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    config.allow_private = true;
    edit(&mut config);
    let options = MasterOptions {
        config,
        data_dir: std::env::temp_dir(),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
    };
    avalanche_master::spawn(options).expect("the master starts")
}

pub async fn exchange(
    to: SocketAddr,
    source: IpAddr,
    method: &str,
    path: &str,
    body: Vec<u8>,
    headers: &[(&str, &str)],
) -> (u16, Option<String>, Vec<u8>) {
    let socket = tokio::net::TcpSocket::new_v4().expect("socket");
    socket.bind(SocketAddr::new(source, 0)).expect("bind the source");
    let Ok(stream) = socket.connect(to).await else { return (0, None, Vec::new()) };
    let Ok((mut sender, conn)) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream)).await
    else {
        return (0, None, Vec::new());
    };
    let driver = tokio::spawn(conn);
    let mut req = Request::builder().method(method).uri(path).header("host", "master");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let Ok(response) = sender.send_request(req.body(Full::new(Bytes::from(body))).expect("request")).await else {
        driver.abort();
        return (0, None, Vec::new());
    };
    let status = response.status().as_u16();
    let etag = response.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = response.into_body().collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
    driver.abort();
    (status, etag, body)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    Honest,
    Silent,
    WrongHash,
    WrongMax,
}

pub struct Fake {
    pub identity: ServerSecret,
    pub ip: IpAddr,
    pub port: u16,
    pub info: Arc<Mutex<ServerInfo>>,
    pub info_body: Arc<Mutex<Option<Vec<u8>>>>,
    pub query: Arc<Mutex<Query>>,
    pub contacted: Arc<AtomicUsize>,
    _runtime: std::thread::JoinHandle<()>,
}

pub fn info_of(identity: &ServerSecret, name: &str) -> ServerInfo {
    ServerInfo {
        info_version: INFO_VERSION,
        name: name.into(),
        description: String::new(),
        protocol_version: ssx_net_proto::PROTOCOL_VERSION,
        api_level: ssx_net_proto::API_LEVEL,
        build: ssx_net_proto::BUILD_ID.into(),
        players: 2,
        max_players: 16,
        password: false,
        mode: "freeride".into(),
        drop: "exp_alps_vorassay_drop_3".into(),
        map: None,
        rewinds: "on".into(),
        ghosts: false,
        ghost_style: "character".into(),
        identity_key: identity.public().to_hex(),
        resources: ssx_net_proto::resources::ResourceSummary {
            content_id: "4e".repeat(32),
            client_mods: "presentation".into(),
            client_scripts: false,
            count: 2,
            download: 20_635,
            manifest: "ab".repeat(32),
            manifest_bytes: 2_431,
        },
        region: "eu-central".into(),
        tags: vec!["race".into()],
        icon: String::new(),
    }
}

impl Fake {
    pub fn start(name: &str, query: Query, tcp: bool) -> Self {
        let identity = ServerSecret::generate();
        let ip = next_ip();
        let tcp_listener = std::net::TcpListener::bind(SocketAddr::new(ip, 0)).expect("tcp");
        let port = tcp_listener.local_addr().expect("addr").port();
        let udp = std::net::UdpSocket::bind(SocketAddr::new(ip, port + 1)).expect("udp");
        let info = Arc::new(Mutex::new(info_of(&identity, name)));
        let info_body = Arc::new(Mutex::new(None));
        let query = Arc::new(Mutex::new(query));
        let contacted = Arc::new(AtomicUsize::new(0));
        let (i, b, q, c) = (Arc::clone(&info), Arc::clone(&info_body), Arc::clone(&query), Arc::clone(&contacted));
        let tcp_listener = tcp.then_some(tcp_listener);
        let runtime = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
            rt.block_on(async move {
                udp.set_nonblocking(true).expect("nonblocking");
                let udp = tokio::net::UdpSocket::from_std(udp).expect("udp");
                tokio::spawn(answer_queries(udp, Arc::clone(&i), q, Arc::clone(&c)));
                if let Some(tcp_listener) = tcp_listener {
                    tcp_listener.set_nonblocking(true).expect("nonblocking");
                    let l = tokio::net::TcpListener::from_std(tcp_listener).expect("tcp");
                    loop {
                        let Ok((s, _)) = l.accept().await else { continue };
                        c.fetch_add(1, Ordering::Relaxed);
                        let (i, b) = (Arc::clone(&i), Arc::clone(&b));
                        tokio::spawn(async move {
                            let svc = hyper::service::service_fn(move |_req: Request<hyper::body::Incoming>| {
                                let body = b
                                    .lock()
                                    .unwrap()
                                    .clone()
                                    .unwrap_or_else(|| i.lock().unwrap().to_json().into_bytes());
                                async move {
                                    Ok::<_, std::convert::Infallible>(hyper::Response::new(Full::new(Bytes::from(
                                        body,
                                    ))))
                                }
                            });
                            let _ =
                                hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(s), svc).await;
                        });
                    }
                } else {
                    std::future::pending::<()>().await;
                }
            });
        });
        Self { identity, ip, port, info, info_body, query, contacted, _runtime: runtime }
    }

    pub fn beat(&self) -> Heartbeat {
        let i = self.info.lock().unwrap().clone();
        Heartbeat {
            heartbeat_version: HEARTBEAT_VERSION,
            port: self.port,
            protocol_version: i.protocol_version,
            info_version: i.info_version,
            api_level: i.api_level,
            build: i.build,
            name: i.name,
            mode: i.mode,
            drop: i.drop,
            map: i.map,
            players: i.players,
            max_players: i.max_players,
            password: i.password,
            region: i.region,
            tags: i.tags,
            contact: "admin@example.org".into(),
            leaving: false,
            resources: i.resources,
        }
    }
}

async fn answer_queries(
    udp: tokio::net::UdpSocket,
    info: Arc<Mutex<ServerInfo>>,
    query: Arc<Mutex<Query>>,
    contacted: Arc<AtomicUsize>,
) {
    let mut buf = [0u8; 256];
    loop {
        let Ok((n, peer)) = udp.recv_from(&mut buf).await else { continue };
        contacted.fetch_add(1, Ordering::Relaxed);
        let behaviour = *query.lock().unwrap();
        if behaviour == Query::Silent {
            continue;
        }
        let answer = match QueryRequest::decode(&buf[..n]) {
            Ok(QueryRequest::Challenge) => Challenge { token: [5; 16] }.encode(),
            Ok(QueryRequest::Query { sequence, .. }) => {
                let i = info.lock().unwrap().clone();
                let mut a = QueryAnswer {
                    sequence,
                    protocol_version: i.protocol_version,
                    players: i.players as u16,
                    max_players: i.max_players as u16,
                    mode: mode_code(&i.mode).unwrap_or(1),
                    password: i.password,
                    info_hash: info_hash(&i),
                };
                match behaviour {
                    Query::WrongHash => a.info_hash[0] ^= 1,
                    Query::WrongMax => a.max_players -= 1,
                    _ => {}
                }
                a.encode().expect("valid")
            }
            Err(_) => continue,
        };
        let _ = udp.send_to(&answer, peer).await;
    }
}

#[derive(Debug)]
pub enum Got {
    Answer(HeartbeatAnswer),
    Plain(u16, String),
}

pub async fn challenge(master: SocketAddr, source: IpAddr) -> MasterChallenge {
    let (status, _, body) = exchange(master, source, "GET", "/v1/challenge", Vec::new(), &[]).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    MasterChallenge::from_bytes(&body).expect("a challenge")
}

pub async fn post(
    master: SocketAddr,
    source: IpAddr,
    body: Vec<u8>,
    identity: &ServerSecret,
    c: &MasterChallenge,
) -> Got {
    let (status, _, body) = exchange(master, source, "POST", "/v1/heartbeat", body, &[]).await;
    if status == 200 {
        Got::Answer(HeartbeatAnswer::open(&body, identity, c).expect("the master's boxed answer"))
    } else {
        Got::Plain(status, String::from_utf8_lossy(&body).into_owned())
    }
}

pub async fn heartbeat(master: SocketAddr, fake: &Fake, edit: impl FnOnce(&mut Heartbeat)) -> Got {
    let c = challenge(master, fake.ip).await;
    let mut beat = fake.beat();
    edit(&mut beat);
    let body = beat.seal(&fake.identity, &c).expect("sealed");
    post(master, fake.ip, body, &fake.identity, &c).await
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime")
}

pub fn random_u32() -> u32 {
    u32::from_be_bytes(random_bytes())
}

pub const SECOND: Duration = Duration::from_secs(1);
