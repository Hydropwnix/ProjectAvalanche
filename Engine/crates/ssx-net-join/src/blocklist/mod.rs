pub mod fetch;
pub mod join;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use ssx_content::blocklist::{Follow, MAX_LISTS, REFRESH_INTERVAL};
use ssx_core::text::{PATH_CHARS, shown_max};
use ssx_net_proto::master::MasterUrl;

use crate::browse::masters::Roots;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refreshed {
    Kept(usize),
    Current(usize),
    NotDue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub master: String,
    pub result: Result<Refreshed, String>,
}

impl Outcome {
    pub fn line(&self) -> String {
        let m = shown_max(&self.master, PATH_CHARS);
        match &self.result {
            Ok(Refreshed::Kept(n)) => format!("blocklist of {m}: {n} blocked files (a new list)"),
            Ok(Refreshed::Current(n)) => format!("blocklist of {m}: {n} blocked files (unchanged)"),
            Ok(Refreshed::NotDue) => {
                format!("blocklist of {m}: asked within the last {} s", REFRESH_INTERVAL.as_secs())
            }
            Err(e) => format!("blocklist of {m}: not refreshed ({e}); the list kept still applies while young enough"),
        }
    }
}

fn attempts() -> &'static Mutex<HashMap<String, Instant>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    ATTEMPTS.get_or_init(Mutex::default)
}

fn claim(master: &str, now: Instant, force: bool) -> bool {
    let mut map = attempts().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !force && map.get(master).is_some_and(|t| now.duration_since(*t) < REFRESH_INTERVAL) {
        return false;
    }
    map.insert(master.to_string(), now);
    true
}

pub fn refresh(follow: &Follow, roots: &Roots, force: bool, now: u64) -> Result<Vec<Outcome>, String> {
    refresh_until(follow, roots, force, now, None)
}

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

pub fn refresh_until(
    follow: &Follow,
    roots: &Roots,
    force: bool,
    now: u64,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Vec<Outcome>, String> {
    let (dir, masters) = follow.masters()?;
    if masters.is_empty() {
        return Ok(Vec::new());
    }
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let started = Instant::now();
    let mut handles = Vec::new();
    let mut out = Vec::new();
    for master in masters.into_iter().take(MAX_LISTS) {
        let url = match MasterUrl::parse(&master) {
            Ok(u) => u,
            Err(e) => {
                out.push(Outcome { master, result: Err(format!("not a master's address: {e}")) });
                continue;
            }
        };
        let due = force || ssx_content::blocklist::due(&dir, &master, now);
        if !due || !claim(&master, started, force) {
            out.push(Outcome { master, result: Ok(Refreshed::NotDue) });
            continue;
        }
        let (dir, roots, label, cancel) = (dir.clone(), roots.clone(), master.clone(), cancel.clone());
        let spawned = std::thread::Builder::new().name("blocklist".into()).spawn(move || {
            let etag = ssx_content::blocklist::etag_of(&dir, &master);
            let result = fetch::fetch(&url, etag.as_deref(), &roots, cancel, crate::browse::resolve::system).and_then(
                |answer| {
                    let current = answer == ssx_content::blocklist::Answer::NotModified;
                    let n = ssx_content::blocklist::record(&dir, &master, answer, now)?;
                    Ok(if current { Refreshed::Current(n) } else { Refreshed::Kept(n) })
                },
            );
            Outcome { master, result }
        });
        match spawned {
            Ok(h) => handles.push(h),
            Err(e) => out.push(Outcome { master: label, result: Err(format!("no thread: {e}")) }),
        }
    }
    for h in handles {
        match h.join() {
            Ok(o) => out.push(o),
            Err(_) => out.push(Outcome { master: String::new(), result: Err("its thread panicked".into()) }),
        }
    }
    Ok(out)
}

static RUNNING: AtomicBool = AtomicBool::new(false);

pub struct Refresh {
    answer: Mutex<Receiver<Result<Vec<Outcome>, String>>>,
}

impl Refresh {
    pub fn start(follow: Follow, roots: Roots) -> Option<Refresh> {
        if RUNNING.swap(true, Ordering::AcqRel) {
            return None;
        }
        let (send, answer) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("blocklists".into()).spawn(move || {
            let result = refresh(&follow, &roots, false, ssx_content::blocklist::now_secs());
            RUNNING.store(false, Ordering::Release);
            let _ = send.send(result);
        });
        if spawned.is_err() {
            RUNNING.store(false, Ordering::Release);
            return None;
        }
        Some(Refresh { answer: Mutex::new(answer) })
    }

    pub fn take(&self) -> Option<Result<Vec<Outcome>, String>> {
        self.answer.lock().ok()?.try_recv().ok()
    }
}
