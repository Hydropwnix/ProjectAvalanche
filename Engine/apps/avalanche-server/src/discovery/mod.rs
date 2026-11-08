pub mod heartbeat;
pub mod lan;
pub mod query;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::Resource;
use ssx_net_proto::keys::ServerSecret;
use ssx_net_proto::master::{HEARTBEAT_AFTER_CHANGE, HEARTBEAT_INTERVAL};
use ssx_net_proto::query::QUERY_PORT_OFFSET;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::StartError;
use crate::bridge::Shared;
use crate::config::Config;
use crate::http::HttpState;
use crate::http::limits::{Rate, Verdict};

#[derive(Resource, Clone, Default)]
pub struct Extras {
    pub roots: Vec<rustls::pki_types::CertificateDer<'static>>,
    pub interval: Option<Duration>,
    pub after_change: Option<Duration>,
}

#[derive(Default)]
pub struct Stats {
    pub query_received: AtomicU64,
    pub query_answered: AtomicU64,
    pub query_limited: AtomicU64,
    pub query_malformed: AtomicU64,
    pub query_bad_token: AtomicU64,
    pub lan_received: AtomicU64,
    pub lan_answered: AtomicU64,
    pub lan_limited: AtomicU64,
    pub lan_off_link: AtomicU64,
    pub lan_malformed: AtomicU64,
    pub heartbeats_sent: AtomicU64,
    pub heartbeats_listed: AtomicU64,
    pub heartbeats_refused: AtomicU64,
    pub heartbeats_failed: AtomicU64,
}

impl Stats {
    pub fn count(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Resource, Clone)]
pub struct Status {
    pub stats: Arc<Stats>,
    pub master: Arc<Mutex<Option<(String, String)>>>,
    pub query: bool,
    pub lan: bool,
    pub portmap: Option<Arc<Mutex<ssx_net_portmap::Status>>>,
}

impl Status {
    pub fn lines(&self) -> Vec<String> {
        let n = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let s = &self.stats;
        let mut out = vec![format!(
            "discovery: query {} (received {}, answered {}, over a rate {}, malformed {}, bad token {}); LAN {} \
             (received {}, answered {}, over a rate {}, off the links {}, malformed {}); heartbeats sent {}, \
             listed {}, not listed {}, failed {}",
            if self.query { "on" } else { "off" },
            n(&s.query_received),
            n(&s.query_answered),
            n(&s.query_limited),
            n(&s.query_malformed),
            n(&s.query_bad_token),
            if self.lan { "on" } else { "off" },
            n(&s.lan_received),
            n(&s.lan_answered),
            n(&s.lan_limited),
            n(&s.lan_off_link),
            n(&s.lan_malformed),
            n(&s.heartbeats_sent),
            n(&s.heartbeats_listed),
            n(&s.heartbeats_refused),
            n(&s.heartbeats_failed),
        )];
        if let Ok(m) = self.master.lock() {
            out.extend(m.iter().map(|(url, last)| format!("  master {url}: {last}")));
            if let Some(map) = &self.portmap {
                out.extend(crate::portmap::lines(map, m.as_ref()));
            }
        }
        out
    }
}

pub struct GlobalLimiter {
    rate: Rate,
    state: Mutex<(f64, Instant)>,
}

impl GlobalLimiter {
    pub fn new(rate: Rate) -> Self {
        Self { rate, state: Mutex::new((rate.burst, Instant::now())) }
    }

    pub fn check(&self, now: Instant) -> Verdict {
        let Ok(mut s) = self.state.lock() else { return Verdict::Full };
        s.0 = (s.0 + now.saturating_duration_since(s.1).as_secs_f64() * self.rate.per_second).min(self.rate.burst);
        s.1 = now;
        if s.0 >= 1.0 {
            s.0 -= 1.0;
            Verdict::Allowed
        } else {
            Verdict::Limited(Duration::from_secs_f64((1.0 - s.0) / self.rate.per_second))
        }
    }
}

pub struct Sockets {
    query: Option<std::net::UdpSocket>,
    lan: Option<std::net::UdpSocket>,
}

impl Sockets {
    pub fn take_query(&mut self) -> Option<std::net::UdpSocket> {
        self.query.take()
    }
}

pub fn bind(config: &Config, game: SocketAddr) -> Result<Sockets, StartError> {
    let d = &config.discovery;
    let query = if d.query {
        let port = game.port().checked_add(QUERY_PORT_OFFSET).ok_or_else(|| {
            StartError::Other(format!("the query's port would be {} + 1: choose a game port below 65535", game.port()))
        })?;
        let addr = SocketAddr::new(config.bind, port);
        let s = crate::bind::udp_socket(addr).map_err(|e| StartError::Udp { addr, why: format!("the query: {e}") })?;
        s.set_nonblocking(true).map_err(|e| StartError::Udp { addr, why: e.to_string() })?;
        Some(s)
    } else {
        None
    };
    let lan = match (d.lan, lan::listen_address(config.bind)) {
        (true, Some(addr)) => {
            Some(lan::bind(addr).map_err(|e| StartError::Udp { addr, why: format!("LAN discovery: {e}") })?)
        }
        (true, None) => {
            bevy::log::warn!("LAN discovery is IPv4 broadcast: off for the IPv6 address {}", config.bind);
            None
        }
        (false, _) => None,
    };
    Ok(Sockets { query, lan })
}

pub struct Discovery {
    beat: Option<Arc<heartbeat::Beat>>,
    responders: Option<Responders>,
    status: Status,
}

const RESPONDERS_WAIT: Duration = Duration::from_secs(2);

struct Responders {
    stop: watch::Sender<bool>,
    ended: Option<tokio::sync::oneshot::Receiver<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Responders {
    fn start(
        sockets: Sockets,
        http: Arc<HttpState>,
        game_port: u16,
        stats: Arc<Stats>,
    ) -> Result<Option<Self>, String> {
        let Sockets { query, lan } = sockets;
        if query.is_none() && lan.is_none() {
            return Ok(None);
        }
        let (stop, mut stopped) = watch::channel(false);
        let (done, ended) = tokio::sync::oneshot::channel();
        let (ready, started) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
        let thread = std::thread::Builder::new()
            .name("discovery".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build() {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ready.send(Err(format!("no runtime: {e}")));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let take = |s: Option<std::net::UdpSocket>| s.map(tokio::net::UdpSocket::from_std).transpose();
                    let (query, lan) = match (take(query), take(lan)) {
                        (Ok(q), Ok(l)) => (q, l),
                        (Err(e), _) | (_, Err(e)) => {
                            let _ = ready.send(Err(e.to_string()));
                            return;
                        }
                    };
                    let _ = ready.send(Ok(()));
                    let query_stats = Arc::clone(&stats);
                    let query = async move {
                        match query {
                            Some(s) => query::run(s, http, query_stats).await,
                            None => std::future::pending().await,
                        }
                    };
                    let lan = async move {
                        match lan {
                            Some(s) => lan::run(s, game_port, stats).await,
                            None => std::future::pending().await,
                        }
                    };
                    tokio::select! {
                        _ = query => {}
                        _ = lan => {}
                        _ = stopped.wait_for(|s| *s) => {}
                    }
                });
                drop(runtime);
                let _ = done.send(());
            })
            .map_err(|e| format!("no thread: {e}"))?;
        match started.recv_timeout(RESPONDERS_WAIT) {
            Ok(Ok(())) => Ok(Some(Self { stop, ended: Some(ended), thread: Some(thread) })),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(format!("not started within {} s", RESPONDERS_WAIT.as_secs())),
        }
    }

    async fn stop(mut self) {
        let _ = self.stop.send(true);
        if let Some(ended) = self.ended.take()
            && tokio::time::timeout(RESPONDERS_WAIT, ended).await.is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for Responders {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

impl Discovery {
    pub fn new(
        config: &Config,
        identity: ServerSecret,
        game: SocketAddr,
        http: Arc<HttpState>,
        shared: Arc<Shared>,
        extras: Extras,
        sockets: Sockets,
    ) -> Result<Self, StartError> {
        let d = &config.discovery;
        let stats = Arc::new(Stats::default());
        let last = Arc::new(Mutex::new(None));
        let beat = if let (true, Some(master)) = (d.public, &d.master) {
            let tls = heartbeat::tls_config(&extras.roots).map_err(StartError::Other)?;
            let source: Option<IpAddr> = (!config.bind.is_unspecified()).then_some(config.bind);
            Some(Arc::new(heartbeat::Beat {
                master: master.clone(),
                identity,
                http: Arc::clone(&http),
                shared,
                game_port: game.port(),
                source,
                region: d.region.clone(),
                tags: d.tags.clone(),
                contact: d.contact.clone(),
                tls: Arc::new(tls),
                interval: extras.interval.unwrap_or(HEARTBEAT_INTERVAL),
                after_change: extras.after_change.unwrap_or(HEARTBEAT_AFTER_CHANGE),
                stats: Arc::clone(&stats),
                last: Arc::clone(&last),
            }))
        } else {
            None
        };
        let responders = Responders::start(sockets, http, game.port(), Arc::clone(&stats))
            .map_err(|e| StartError::Other(format!("the query and LAN responders: {e}")))?;
        let status = Status { stats, master: last, query: d.query, lan: d.lan, portmap: None };
        Ok(Self { beat, responders, status })
    }

    pub fn status(&self) -> Status {
        self.status.clone()
    }

    pub fn start(self) -> Running {
        let (stop, stopped) = watch::channel(false);
        let beat = self.beat.map(|b| tokio::spawn(heartbeat::run(b, stopped)));
        Running { responders: self.responders, beat, stop }
    }
}

pub struct Running {
    responders: Option<Responders>,
    beat: Option<JoinHandle<()>>,
    stop: watch::Sender<bool>,
}

impl Running {
    pub fn abort(self) {
        if let Some(t) = &self.beat {
            t.abort();
        }
        drop(self.responders);
    }

    pub async fn stop(self) {
        if let Some(r) = self.responders {
            r.stop().await;
        }
        let _ = self.stop.send(true);
        if let Some(b) = self.beat {
            let abort = b.abort_handle();
            let wait = heartbeat::LEAVE_TIMEOUT + Duration::from_secs(1);
            if tokio::time::timeout(wait, b).await.is_err() {
                abort.abort();
            }
        }
    }
}
