use core::time::Duration;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ssx_net_proto::keys::from_hex;
use ssx_net_proto::master::{ListedServer, MAX_LIST_BYTES, MasterHost, MasterUrl, ServerList};
use ssx_net_proto::{BUILD_ID, PROTOCOL_VERSION};
use ureq::Agent;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use super::address::{Host, ServerAddress};
use super::resolve::Lookup;

pub type Roots = Option<Arc<Vec<ureq::tls::Certificate<'static>>>>;

// TODO(guess): 8 (the plan's federation: a project master and a few community ones); settle: the masters players add.
pub const MAX_MASTERS: usize = 8;
pub const AT_ONCE: usize = 2;
pub const STEP: Duration = Duration::from_millis(2500);
// TODO(guess): 2 s for the lookup, 3 s to connect, 15 s in all; settle: the lists of real masters over real links.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
pub const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_HEADERS: usize = 8 * 1024;
const MAX_ETAG: usize = 128;
pub const MAX_RUNNING: usize = 16;
static RUNNING: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn from_settings(value: Option<&serde_json::Value>) -> Result<Vec<MasterUrl>, String> {
    let Some(value) = value else { return official().map(|m| vec![m]) };
    let list = value.as_array().ok_or("`masters` is not a list of addresses")?;
    if list.len() > MAX_MASTERS {
        return Err(format!("`masters` names {} masters, at most {MAX_MASTERS}", list.len()));
    }
    let mut out: Vec<MasterUrl> = Vec::with_capacity(list.len());
    for v in list {
        let text = v.as_str().ok_or("`masters` holds something that is not an address")?;
        let url = MasterUrl::parse(text).map_err(|e| format!("`masters`: {e}"))?;
        if out.contains(&url) {
            return Err(format!("`masters` names {url} twice"));
        }
        out.push(url);
    }
    Ok(out)
}

pub fn official() -> Result<MasterUrl, String> {
    MasterUrl::parse(ssx_core::OFFICIAL_MASTER).map_err(|e| format!("the project's master: {e}"))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fetched {
    List { list: ServerList, etag: Option<String> },
    NotModified,
}

#[derive(Clone, Debug)]
pub struct MasterAnswer {
    pub url: MasterUrl,
    pub result: Result<Fetched, String>,
    pub took: Duration,
}

pub(crate) fn system_roots() -> Arc<Vec<ureq::tls::Certificate<'static>>> {
    static ROOTS: std::sync::OnceLock<Arc<Vec<ureq::tls::Certificate<'static>>>> = std::sync::OnceLock::new();
    Arc::clone(ROOTS.get_or_init(|| {
        let found = rustls_native_certs::load_native_certs();
        Arc::new(found.certs.iter().map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned()).collect())
    }))
}

#[derive(Debug)]
struct MasterResolver {
    url: MasterUrl,
    cancel: Option<Arc<AtomicBool>>,
    lookup: Lookup,
}

impl Resolver for MasterResolver {
    fn resolve(&self, uri: &Uri, _: &Config, _: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        if uri.authority().map(|a| a.as_str()) != Some(self.url.authority().as_str()) {
            return Err(ureq::Error::HostNotFound);
        }
        let found = match &self.url.host {
            MasterHost::Ip(ip) => SocketAddr::new(*ip, self.url.port),
            MasterHost::Name(name) => {
                super::resolve::resolve(name, self.url.port, RESOLVE_TIMEOUT, self.cancel.as_deref(), self.lookup)
                    .map_err(|e| ureq::Error::Io(std::io::Error::other(e)))?
            }
        };
        let mut out = self.empty();
        out.push(found);
        Ok(out)
    }
}

pub(crate) fn agent(url: &MasterUrl, roots: &Roots, cancel: Option<Arc<AtomicBool>>, lookup: Lookup) -> Agent {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let roots = roots.clone().unwrap_or_else(system_roots);
    let tls = TlsConfig::builder().root_certs(RootCerts::Specific(roots)).build();
    let config = Agent::config_builder()
        .timeout_global(Some(LIST_TIMEOUT))
        .timeout_connect(Some(CONNECT_TIMEOUT))
        .max_response_header_size(MAX_HEADERS)
        .http_status_as_error(false)
        .max_redirects(0)
        .proxy(None)
        .user_agent(BUILD_ID)
        .tls_config(tls)
        .build();
    Agent::with_parts(config, DefaultConnector::new(), MasterResolver { url: url.clone(), cancel, lookup })
}

pub fn fetch(url: &MasterUrl, etag: Option<&str>, roots: &Roots) -> Result<Fetched, String> {
    fetch_until(url, etag, roots, None, super::resolve::system)
}

fn fetch_until(
    url: &MasterUrl,
    etag: Option<&str>,
    roots: &Roots,
    cancel: Option<Arc<AtomicBool>>,
    lookup: Lookup,
) -> Result<Fetched, String> {
    let target = format!("{url}/v1/servers?protocol={PROTOCOL_VERSION}");
    let mut req = agent(url, roots, cancel, lookup).get(&target);
    if let Some(tag) = etag {
        req = req.header("if-none-match", tag);
    }
    let mut response = req.call().map_err(|e| match e {
        ureq::Error::Timeout(_) => format!("no answer within {} s", LIST_TIMEOUT.as_secs()),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => "nothing answers there".into(),
        other => format!("cannot be reached: {other}"),
    })?;
    let status = response.status().as_u16();
    if status == 304 {
        return Ok(Fetched::NotModified);
    }
    if status == 429 {
        return Err("asks to be asked less often (HTTP 429)".into());
    }
    if status != 200 {
        return Err(format!("not a Project Avalanche master of this version (HTTP {status})"));
    }
    let tag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|t| t.len() <= MAX_ETAG && t.bytes().all(|b| (0x21..=0x7E).contains(&b)))
        .map(str::to_string);
    let body =
        response.body_mut().with_config().limit(MAX_LIST_BYTES as u64 + 1).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => format!("its list is larger than {MAX_LIST_BYTES} bytes"),
            ureq::Error::Timeout(_) => format!("no whole list within {} s", LIST_TIMEOUT.as_secs()),
            e => format!("its list does not read: {e}"),
        })?;
    let list = ServerList::from_json(&body, PROTOCOL_VERSION)?;
    Ok(Fetched::List { list, etag: tag })
}

pub struct Masters {
    masters: Vec<MasterUrl>,
    roots: Roots,
    started: Option<Instant>,
    asked: usize,
    answered_ok: usize,
    running: usize,
    answers: Mutex<Receiver<MasterAnswer>>,
    send: Sender<MasterAnswer>,
    generation: Arc<AtomicBool>,
    pub kept: HashMap<MasterUrl, (ServerList, Option<String>)>,
    pub status: Vec<(MasterUrl, Result<usize, String>)>,
}

impl Masters {
    pub fn new(masters: Vec<MasterUrl>, roots: Option<Vec<Vec<u8>>>) -> Self {
        let (send, answers) = std::sync::mpsc::channel();
        let roots = roots
            .map(|r| Arc::new(r.into_iter().map(|der| ureq::tls::Certificate::from_der(&der).to_owned()).collect()));
        Self {
            masters,
            roots,
            started: None,
            asked: 0,
            answered_ok: 0,
            running: 0,
            answers: Mutex::new(answers),
            send,
            generation: Arc::new(AtomicBool::new(false)),
            kept: HashMap::new(),
            status: Vec::new(),
        }
    }

    pub fn masters(&self) -> &[MasterUrl] {
        &self.masters
    }

    pub fn refresh(&mut self, now: Instant) -> bool {
        if self.started.is_some_and(|t| now.duration_since(t) < STEP) {
            return false;
        }
        self.generation.store(true, Ordering::Release);
        self.generation = Arc::new(AtomicBool::new(false));
        let (send, answers) = std::sync::mpsc::channel();
        self.send = send;
        self.answers = Mutex::new(answers);
        self.started = None;
        self.asked = 0;
        self.answered_ok = 0;
        self.running = 0;
        self.status.clear();
        true
    }

    fn ask(&mut self, i: usize) {
        let Some(url) = self.masters.get(i).cloned() else { return };
        self.asked = self.asked.max(i + 1);
        if RUNNING.fetch_add(1, Ordering::AcqRel) >= MAX_RUNNING {
            RUNNING.fetch_sub(1, Ordering::AcqRel);
            self.status.push((url, Err(format!("not asked: {MAX_RUNNING} lists are under way already"))));
            return;
        }
        let etag = self.kept.get(&url).and_then(|(_, t)| t.clone());
        let (send, roots, dropped) = (self.send.clone(), self.roots.clone(), Arc::clone(&self.generation));
        let spawned = std::thread::Builder::new().name("master list".into()).spawn(move || {
            let t0 = Instant::now();
            let cancel = Some(Arc::clone(&dropped));
            let result = fetch_until(&url, etag.as_deref(), &roots, cancel, super::resolve::system);
            RUNNING.fetch_sub(1, Ordering::AcqRel);
            if !dropped.load(Ordering::Acquire) {
                let _ = send.send(MasterAnswer { url, result, took: t0.elapsed() });
            }
        });
        match spawned {
            Ok(_) => self.running += 1,
            Err(_) => {
                RUNNING.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }

    pub fn poll(&mut self, now: Instant) -> bool {
        let started = *self.started.get_or_insert(now);
        if self.asked == 0 {
            for i in 0..AT_ONCE.min(self.masters.len()) {
                self.ask(i);
            }
        }
        let steps = (now.duration_since(started).as_millis() / STEP.as_millis()) as usize;
        while self.asked < self.masters.len() && self.answered_ok < AT_ONCE && self.asked < AT_ONCE + steps {
            self.ask(self.asked);
        }
        let got: Vec<MasterAnswer> = match self.answers.lock() {
            Ok(rx) => rx.try_iter().collect(),
            Err(_) => Vec::new(),
        };
        let changed = !got.is_empty();
        for a in got {
            self.running = self.running.saturating_sub(1);
            let note = match a.result {
                Ok(Fetched::List { list, etag }) => {
                    let n = list.servers.len();
                    self.kept.insert(a.url.clone(), (list, etag));
                    Ok(n)
                }
                Ok(Fetched::NotModified) => Ok(self.kept.get(&a.url).map_or(0, |(l, _)| l.servers.len())),
                Err(e) => Err(e),
            };
            if note.is_ok() {
                self.answered_ok += 1;
            }
            self.status.retain(|(u, _)| *u != a.url);
            self.status.push((a.url, note));
        }
        changed
    }

    pub fn waiting(&self) -> usize {
        self.running
    }

    pub fn merged(&self) -> (Vec<Listed>, usize) {
        let lists: Vec<(&MasterUrl, &ServerList)> =
            self.masters.iter().filter_map(|m| self.kept.get(m).map(|(l, _)| (m, l))).collect();
        merge(&lists)
    }
}

impl Drop for Masters {
    fn drop(&mut self) {
        self.generation.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    pub address: ServerAddress,
    pub key: [u8; 32],
    pub server: ListedServer,
    pub masters: Vec<MasterUrl>,
}

pub fn merge(lists: &[(&MasterUrl, &ServerList)]) -> (Vec<Listed>, usize) {
    let mut out: Vec<Listed> = Vec::new();
    let mut by_key: HashMap<[u8; 32], usize> = HashMap::new();
    let mut places: HashMap<(IpAddr, u16), usize> = HashMap::new();
    let mut left_out = 0;
    for (url, list) in lists {
        for s in &list.servers {
            let (Ok(ip), Ok(raw)) = (s.ip(), from_hex(&s.key)) else {
                left_out += 1;
                continue;
            };
            if let Some(&i) = by_key.get(&raw) {
                if !out[i].masters.contains(url) {
                    out[i].masters.push((*url).clone());
                }
                continue;
            }
            if places.contains_key(&(ip, s.port)) {
                left_out += 1;
                continue;
            }
            by_key.insert(raw, out.len());
            places.insert((ip, s.port), out.len());
            out.push(Listed {
                address: ServerAddress { host: Host::Ip(ip), port: s.port },
                key: raw,
                server: s.clone(),
                masters: vec![(*url).clone()],
            });
        }
    }
    (out, left_out)
}

pub fn socket(l: &Listed) -> Option<SocketAddr> {
    match l.address.host {
        Host::Ip(ip) => Some(SocketAddr::new(ip, l.address.port)),
        Host::Name(_) => None,
    }
}

#[cfg(test)]
mod tests;
