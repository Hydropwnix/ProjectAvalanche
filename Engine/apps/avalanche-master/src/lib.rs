pub mod address;
pub mod blocklists;
pub mod challenge;
pub mod check;
pub mod config;
pub mod console;
pub mod forwarded;
pub mod health;
pub mod heartbeat;
pub mod http;
pub mod limits;
pub mod logging;
pub mod names;
pub mod registry;
pub mod route;
pub mod state;
pub mod tls;
pub mod trust;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ssx_net_proto::keys::to_hex;
use tokio::sync::Notify;
use tracing::{error, info, warn};

use crate::blocklists::Blocklists;
use crate::config::Config;
use crate::state::Master;

pub struct MasterOptions {
    pub config: Config,
    pub data_dir: PathBuf,
    pub console: bool,
    pub logging: bool,
    pub log_to_file: bool,
    pub signals: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("{0}")]
    Blocklists(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("the socket {addr} (TCP) does not open: {error}")]
    Bind { addr: SocketAddr, error: std::io::Error },
    #[error("{0}")]
    Other(String),
}

#[derive(Clone)]
pub struct Started {
    pub addr: SocketAddr,
    pub master: Arc<Master>,
    pub tls: Option<Arc<tls::Slot>>,
}

pub struct MasterThread {
    pub started: Started,
    shutdown: Arc<Notify>,
    handle: JoinHandle<Result<(), String>>,
}

impl MasterThread {
    pub fn stop(&self) {
        self.started.master.stopping.store(true, Ordering::Release);
        self.shutdown.notify_one();
    }

    pub fn join(self) -> Result<(), String> {
        self.handle.join().map_err(|_| "the master's thread panicked".to_string())?
    }
}

pub fn spawn(options: MasterOptions) -> Result<MasterThread, StartError> {
    spawn_with_resolver(options, Arc::new(names::SystemResolver))
}

pub fn spawn_with_resolver(
    options: MasterOptions,
    resolver: Arc<dyn names::Resolve>,
) -> Result<MasterThread, StartError> {
    let shutdown = Arc::new(Notify::new());
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::clone(&shutdown);
    let handle = std::thread::Builder::new()
        .name("master".into())
        .spawn(move || {
            let result = run_with(options, stop, resolver, |s| {
                let _ = tx.send(Ok(s.clone()));
            });
            if let Err(e) = &result {
                let _ = tx.send(Err(e.to_string()));
            }
            result.map_err(|e| e.to_string())
        })
        .map_err(|e| StartError::Other(e.to_string()))?;
    match rx.recv() {
        Ok(Ok(started)) => Ok(MasterThread { started, shutdown, handle }),
        Ok(Err(e)) => Err(StartError::Other(e)),
        Err(_) => Err(StartError::Other("the master's thread ended before it started".into())),
    }
}

pub fn run(options: MasterOptions) -> Result<(), StartError> {
    run_with(options, Arc::new(Notify::new()), Arc::new(names::SystemResolver), |_| {})
}

fn run_with(
    options: MasterOptions,
    shutdown: Arc<Notify>,
    resolver: Arc<dyn names::Resolve>,
    on_started: impl FnOnce(&Started),
) -> Result<(), StartError> {
    let config = options.config;
    if options.logging {
        let file = options.log_to_file.then(|| options.data_dir.join("logs").join("master.log"));
        logging::init(file.as_deref());
    }
    let blocklists = Blocklists::load(&config).map_err(StartError::Blocklists)?;
    let tls = config.tls.as_ref().map(tls::acceptor).transpose().map_err(StartError::Tls)?;
    let tls = tls.map(|a| Arc::new(tls::Slot::new(a)));
    let lines = if options.console {
        Some(console::spawn().map_err(|e| StartError::Other(format!("console: {e}")))?)
    } else {
        None
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| StartError::Other(e.to_string()))?;
    runtime.block_on(async move {
        let addr = config.addr();
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|error| StartError::Bind { addr, error })?;
        let local = listener.local_addr().map_err(|error| StartError::Bind { addr, error })?;
        let scheme = if tls.is_some() { "HTTPS" } else { "HTTP" };
        let master = Arc::new(Master::new(config, blocklists));
        info!(
            "listening on {local} ({scheme}); at most {} servers, {} per address; {}; blocked keys {}, blocked \
             resources {}",
            master.config.max_servers,
            master.config.max_per_address,
            if master.config.allow_private {
                "private addresses allowed (a LAN or test master)"
            } else {
                "public addresses only"
            },
            master.blocklists.read().map_or(0, |b| b.servers.len()),
            master.blocklists.read().map_or(0, |b| b.resources.len()),
        );
        let names = (!master.config.trusted_hosts.is_empty()).then(|| trust::Lookups::new(resolver));
        let names = match names {
            Some(mut lookups) => {
                trust::refresh(&master.trust, &mut lookups).await;
                Some(tokio::spawn(trust::keep_fresh(Arc::clone(&master), lookups)))
            }
            None => None,
        };
        if master.config.behind_proxy() {
            let now = master.trust.now();
            info!("behind trusted proxies {}: {}", now.set.describe(now.at), forwarded_rule());
            match route::start_line(&route::default_gateways(), &now) {
                Some(route::StartLine::Warning(line)) => warn!("{line}"),
                Some(route::StartLine::NotRead { text, unsupported: true }) => info!("{text}"),
                Some(route::StartLine::NotRead { text, unsupported: false }) => warn!("{text}"),
                None => {}
            }
        }
        on_started(&Started { addr: local, master: Arc::clone(&master), tls: tls.clone() });
        let sweeper = tokio::spawn(sweep(Arc::clone(&master)));
        let console = lines.map(|l| tokio::spawn(commands(Arc::clone(&master), l, Arc::clone(&shutdown), tls.clone())));
        let signals = options.signals.then(|| tokio::spawn(watch_signals(Arc::clone(&master), Arc::clone(&shutdown))));
        #[cfg(unix)]
        let hangups = options.signals.then(|| tokio::spawn(watch_hangups(Arc::clone(&master), tls.clone())));
        #[cfg(not(unix))]
        let hangups: Option<tokio::task::JoinHandle<()>> = None;
        http::run(listener, Arc::clone(&master), tls, shutdown).await;
        sweeper.abort();
        for task in console.into_iter().chain(signals).chain(hangups).chain(names) {
            task.abort();
        }
        info!("stopped");
        Ok(())
    })
}

async fn sweep(master: Arc<Master>) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tick.tick().await;
        let gone = match master.registry.lock() {
            Ok(mut r) => r.expire(Instant::now()),
            Err(_) => continue,
        };
        for l in gone {
            state::Stats::count(&master.stats.expired);
            info!(
                "expired {:?} ({}:{}, key {}): no heartbeat for {} s",
                l.beat.name,
                l.ip,
                l.beat.port,
                &to_hex(&l.key)[..16],
                ssx_net_proto::master::EXPIRE_AFTER.as_secs()
            );
        }
    }
}

pub fn forwarded_rule() -> &'static str {
    "a request from them names its client in the right-most X-Forwarded-For entry; any other peer is its own client"
}

fn status(master: &Master) -> Vec<String> {
    let s = &master.stats;
    let n = |c: &std::sync::atomic::AtomicU64| c.load(Ordering::Relaxed);
    let listed = master.registry.lock().map_or(0, |r| r.len());
    let proxy = master.config.behind_proxy().then(|| {
        let now = master.trust.now();
        format!(
            "behind trusted proxies ({}): requests named by them {}, refused for their X-Forwarded-For {}; their \
             connections open {} (at most {}), closed at a full pool {}; addresses with open requests {}",
            now.set.describe(now.at),
            n(&s.proxied),
            n(&s.forwarded_refused),
            master.limits.connections.proxied_now(),
            state::MAX_PROXIED_CONNECTIONS,
            n(&s.proxy_pool_full),
            master.limits.connections.addresses_now(),
        )
    });
    let mut lines = vec![
        format!(
            "uptime {:.0} s; listed {listed}; checks running {}",
            master.started.elapsed().as_secs_f64(),
            state::CHECKS_AT_ONCE - master.checks.available_permits()
        ),
        format!(
            "requests {} limited {}; heartbeats {} refused {}; checks passed {} failed {}; expired {}; lists served {} \
             (not modified {}); connections open {}; challenges remembered {}; log lines suppressed {}",
            n(&s.requests),
            n(&s.limited),
            n(&s.heartbeats),
            n(&s.refused),
            n(&s.checks_passed),
            n(&s.checks_failed),
            n(&s.expired),
            n(&s.lists_served),
            n(&s.not_modified),
            master.limits.connections.open_now(),
            master.challenges.remembered(),
            logging::SUPPRESSED_LINES.load(Ordering::Relaxed),
        ),
    ];
    lines.extend(proxy);
    lines
}

fn servers(master: &Master) -> Vec<String> {
    let Ok(r) = master.registry.lock() else { return vec![] };
    let now = Instant::now();
    let mut all: Vec<_> = r.iter().collect();
    all.sort_by(|a, b| a.beat.name.cmp(&b.beat.name).then(a.key.cmp(&b.key)));
    let mut out: Vec<String> = all
        .iter()
        .take(200)
        .map(|l| {
            format!(
                "{}:{} {:?} {}/{} {} {} key {} last heartbeat {:.0} s ago contact {:?}",
                l.ip,
                l.beat.port,
                l.beat.name,
                l.beat.players,
                l.beat.max_players,
                l.beat.mode,
                l.beat.drop,
                &to_hex(&l.key)[..16],
                now.duration_since(l.last_seen).as_secs_f64(),
                l.beat.contact
            )
        })
        .collect();
    if all.len() > 200 {
        out.push(format!("and {} more", all.len() - 200));
    }
    out.insert(0, format!("{} servers:", all.len()));
    out
}

async fn commands(
    master: Arc<Master>,
    mut lines: tokio::sync::mpsc::Receiver<String>,
    shutdown: Arc<Notify>,
    tls: Option<Arc<tls::Slot>>,
) {
    while let Some(line) = lines.recv().await {
        match console::parse(&line) {
            None => {}
            Some(Err(e)) => info!("{e}"),
            Some(Ok(console::Command::Help)) => info!("{}", console::HELP),
            Some(Ok(console::Command::Status)) => status(&master).iter().for_each(|l| info!("{l}")),
            Some(Ok(console::Command::Servers)) => servers(&master).iter().for_each(|l| info!("{l}")),
            Some(Ok(console::Command::Reload)) => reload(&master, tls.as_deref()),
            Some(Ok(console::Command::Stop)) => {
                info!("stopping");
                master.stopping.store(true, Ordering::Release);
                shutdown.notify_one();
            }
        }
    }
}

pub fn reload(master: &Master, tls: Option<&tls::Slot>) {
    match Blocklists::load(&master.config) {
        Ok(lists) => {
            let blocked: Vec<[u8; 32]> = lists.servers.keys().copied().collect();
            let (keys, resources) = (lists.servers.len(), lists.resources.len());
            if let Ok(mut b) = master.blocklists.write() {
                *b = lists;
            }
            let taken =
                master.registry.lock().map_or(0, |mut r| blocked.iter().filter(|k| r.remove(k).is_some()).count());
            info!("blocklists read again: {keys} server keys, {resources} resources; {taken} listed servers taken off");
        }
        Err(e) => error!("the blocklists were not read again (the old ones stay): {e}"),
    }
    if let (Some(slot), Some(files)) = (tls, &master.config.tls) {
        match slot.reload(files) {
            Ok(()) => info!("the certificate read again from {}", files.certificate.display()),
            Err(e) => error!("the certificate was not read again (the one in use stays): {e}"),
        }
    }
}

#[cfg(unix)]
async fn watch_hangups(master: Arc<Master>, tls: Option<Arc<tls::Slot>>) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut hangup = match signal(SignalKind::hangup()) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                "SIGHUP's handler could not be installed ({e}): read the certificate again with the console's `reload`"
            );
            return;
        }
    };
    while hangup.recv().await.is_some() {
        info!("SIGHUP: reading the blocklists and the certificate again");
        reload(&master, tls.as_deref());
    }
}

async fn watch_signals(master: Arc<Master>, shutdown: Arc<Notify>) {
    let mut asked = false;
    loop {
        if wait_for_signal().await.is_err() {
            warn!("the signal handlers could not be installed: stop the master with the console's `stop`");
            return;
        }
        if asked {
            error!("a second stop signal: ending at once");
            std::process::exit(1);
        }
        asked = true;
        info!("stop signal: stopping (a second signal ends the master at once)");
        master.stopping.store(true, Ordering::Release);
        shutdown.notify_one();
    }
}

#[cfg(unix)]
async fn wait_for_signal() -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = terminate.recv() => {}
        _ = interrupt.recv() => {}
    }
    Ok(())
}

#[cfg(windows)]
async fn wait_for_signal() -> std::io::Result<()> {
    use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
    let (mut c, mut b, mut close, mut shutdown) = (ctrl_c()?, ctrl_break()?, ctrl_close()?, ctrl_shutdown()?);
    tokio::select! {
        _ = c.recv() => {}
        _ = b.recv() => {}
        _ = close.recv() => {}
        _ = shutdown.recv() => {}
    }
    Ok(())
}
