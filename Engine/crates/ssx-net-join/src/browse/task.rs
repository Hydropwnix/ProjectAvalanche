use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ssx_net_proto::join::{JoinContent, RefusalCode};
use ssx_net_proto::keys::ServerKey;
use zeroize::Zeroizing;

use super::address::ServerAddress;
use crate::http::{Granted, JoinFailure, JoinParams};
use crate::http::{join_listed_showing, join_showing_then_pin};
use crate::identity::PlayerIdentity;
use crate::known::{KnownServers, PinWrite};
use crate::resources::{Fetched, Progress, ResourceOptions, Step, fetch_all, fetch_manifest};

pub struct JoinRequest {
    pub address: ServerAddress,
    pub name: String,
    pub password: Zeroizing<Vec<u8>>,
    pub app_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failed {
    pub text: String,
    pub refusal: Option<RefusalCode>,
    pub cancelled: bool,
}

pub struct Joined {
    pub server: SocketAddr,
    pub granted: Box<Granted>,
    pub identity_created: bool,
    pub resources: Option<Box<Fetched>>,
    pub pinned: Option<Receiver<Pinned>>,
}

#[derive(Debug)]
pub struct Pinned {
    pub path: PathBuf,
    pub after: Duration,
    pub result: Result<(), String>,
}

// PORT(choice): a direct connect's new key (trust on first use) is written to `known_servers.txt` after the join's
// token is handed on, so that the write (the file and its folder synced: 3.4 + 5.8 s once under another program's
// 3 GB write, btrfs, which expired the 10 s token: round io1) is not in the token's time; a failed write is reported
// and the session goes on (the next join to the server pins it anew). The console has no direct connects.

pub struct ResourceJoin {
    pub options: ResourceOptions,
    pub progress: Arc<Progress>,
    pub check: ContentCheck,
    pub release: Box<dyn FnMut() + Send>,
}

impl ResourceJoin {
    pub fn without_content(cache: PathBuf) -> Self {
        let check: ContentCheck = Box::new(|f: &Fetched| match (f.packages.is_empty(), f.manifest.content_id) {
            (true, None) => Ok(None),
            _ => Err("this client mounts no server resources and computes no levels' ids".into()),
        });
        let options = ResourceOptions {
            cache,
            quota: crate::resources::DEFAULT_QUOTA,
            limit: crate::resources::DEFAULT_LIMIT,
            blocklists: ssx_content::blocklist::Follow::Nothing,
        };
        Self { options, progress: Arc::new(Progress::default()), check, release: Box::new(|| {}) }
    }
}

pub type ContentCheck = Box<dyn FnMut(&Fetched) -> Result<Option<ssx_content::data::SimContentId>, String> + Send>;

fn resources(
    server: SocketAddr,
    r: &mut ResourceJoin,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<Box<Fetched>>, Failed> {
    r.progress.say(Step::Reading);
    let stopped = |e: String| {
        if cancel.load(Ordering::Acquire) {
            Failed { text: "cancelled".into(), refusal: None, cancelled: true }
        } else {
            failed(format!("the server's resources: {e}"))
        }
    };
    let Some(manifest) = fetch_manifest(server, cancel).map_err(stopped)? else { return Ok(None) };
    let mut fetched = fetch_all(server, manifest, &r.options, &r.progress, cancel).map_err(stopped)?;
    r.progress.say(Step::CheckingContent);
    fetched.content_id = (r.check)(&fetched).map_err(failed)?;
    Ok(Some(Box::new(fetched)))
}

pub struct JoinTask {
    cancel: Arc<AtomicBool>,
    answer: Mutex<Receiver<Result<Joined, Failed>>>,
    pub address: ServerAddress,
    held: Arc<Mutex<HeldText>>,
}

#[derive(Default)]
struct HeldText {
    text: Option<String>,
    fresh: bool,
}

fn failed(text: String) -> Failed {
    Failed { text, refusal: None, cancelled: false }
}

fn run(
    req: JoinRequest,
    listed: Option<ServerKey>,
    mut with: ResourceJoin,
    cancel: &Arc<AtomicBool>,
    held: &Mutex<HeldText>,
) -> Result<(Joined, Option<PinWrite>), Failed> {
    let identity = PlayerIdentity::load_or_create(&req.app_dir).map_err(|e| failed(e.to_string()))?;
    let server = req.address.resolve_until(Some(cancel)).map_err(failed)?;
    let mut fetched = resources(server, &mut with, cancel)?;
    let mut again = true;
    let result = loop {
        let content = fetched.as_ref().map_or_else(JoinContent::default, |f| JoinContent {
            content_id: f.content_id.map(|c| c.0),
            manifest: Some(f.manifest_hash.0),
        });
        let params = JoinParams { name: &req.name, password: &req.password, cancel: Some(cancel), content };
        let show = |text: &str| {
            if let Ok(mut h) = held.lock()
                && h.text.as_deref() != Some(text)
            {
                h.text = Some(text.to_string());
                h.fresh = true;
            }
        };
        let result = match &listed {
            Some(key) => join_listed_showing(server, &identity, key, &params, &show).map(|g| (g, None)),
            None => {
                let mut known = KnownServers::load(&req.app_dir).map_err(|e| failed(e.to_string()))?;
                join_showing_then_pin(server, &identity, &mut known, &params, &show)
            }
        };
        match result {
            Err(JoinFailure::Refused(r)) if r.code == RefusalCode::ResourcesChanged && again => {
                again = false;
                (with.release)();
                drop(fetched.take());
                fetched = resources(server, &mut with, cancel)?;
            }
            other => break other,
        }
    };
    match result {
        Ok((granted, pin)) => {
            let joined = Joined {
                server,
                granted: Box::new(granted),
                identity_created: identity.created(),
                resources: fetched,
                pinned: None,
            };
            Ok((joined, pin))
        }
        Err(JoinFailure::Cancelled) => Err(Failed { text: "cancelled".into(), refusal: None, cancelled: true }),
        Err(JoinFailure::Refused(r)) => {
            let mut text = JoinFailure::Refused(r.clone()).to_string();
            if let Some(until) = r.ban {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                text = format!("{text} ({})", ssx_net_proto::operator::ban_end(until, now));
            }
            Err(Failed { text, refusal: Some(r.code), cancelled: false })
        }
        Err(e) => Err(failed(e.to_string())),
    }
}

impl JoinTask {
    pub fn spawn(req: JoinRequest, listed: Option<ServerKey>, resources: ResourceJoin) -> std::io::Result<Self> {
        Self::start(req, listed, resources)
    }

    fn start(req: JoinRequest, listed: Option<ServerKey>, with: ResourceJoin) -> std::io::Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let address = req.address.clone();
        let flag = Arc::clone(&cancel);
        let held = Arc::new(Mutex::new(HeldText::default()));
        let shown = Arc::clone(&held);
        std::thread::Builder::new().name("network join".into()).spawn(move || {
            let (mut out, pin) = match run(req, listed, with, &flag, &shown) {
                Ok((joined, pin)) => (Ok(joined), pin),
                Err(f) => (Err(f), None),
            };
            let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
            if let (Ok(joined), Some(_)) = (&mut out, &pin) {
                joined.pinned = Some(pinned_rx);
            }
            if flag.load(Ordering::Acquire) {
                out = Err(Failed { text: "cancelled".into(), refusal: None, cancelled: true });
            }
            let _ = tx.send(out);
            if let Some(pin) = pin {
                let handed = Instant::now();
                let path = pin.path().to_path_buf();
                let result = pin.write().map_err(|e| e.to_string());
                let _ = pinned_tx.send(Pinned { path, after: handed.elapsed(), result });
            }
        })?;
        Ok(Self { cancel, answer: Mutex::new(rx), address, held })
    }

    pub fn held(&self) -> Option<String> {
        let mut h = self.held.lock().ok()?;
        if !std::mem::take(&mut h.fresh) {
            return None;
        }
        h.text.clone()
    }

    pub fn poll(&self) -> Option<Result<Joined, Failed>> {
        let rx = self.answer.lock().ok()?;
        match rx.try_recv() {
            Ok(a) => Some(a),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(failed("the join's thread ended without an answer".into()))),
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}
