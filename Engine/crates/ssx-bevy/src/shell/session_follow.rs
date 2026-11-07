use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ssx_net::ServerSaid;
use ssx_net_join::resources::{Fetched, Progress, fetch_all, fetch_manifest};

use super::live::{LiveEnd, LiveSession};
use super::session_content::SessionContent;

type Answer = Result<Option<Box<Fetched>>, String>;

pub struct Following {
    want: [u8; 32],
    cancel: Arc<AtomicBool>,
    answer: Mutex<Receiver<Answer>>,
}

impl Drop for Following {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Resource, Default)]
pub struct Follow {
    worker: Option<Following>,
    next: Option<[u8; 32]>,
    started: Option<[u8; 32]>,
    pub starts: u64,
}

fn start(held: &SessionContent, want: [u8; 32], root: &std::path::Path) -> std::io::Result<Following> {
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel();
    let (server, options, flag) = (held.server, held.options.clone(), Arc::clone(&cancel));
    let current = held.fetched.manifest_hash.0;
    let data = root.parent().unwrap_or(root).to_path_buf();
    let mut check = held.slot.checker(data);
    std::thread::Builder::new().name("session resources".into()).spawn(move || {
        let mut run = || -> Answer {
            let manifest = fetch_manifest(server, &flag)?.ok_or("the server has no resources any more")?;
            if manifest.hash.0 == current {
                return Ok(None);
            }
            let progress = Progress::default();
            let fetched = fetch_all(server, manifest, &options, &progress, &flag);
            for line in progress.blocklists.into_inner().unwrap_or_else(std::sync::PoisonError::into_inner) {
                info!("session: {line}");
            }
            let mut fetched = fetched?;
            if flag.load(Ordering::Acquire) {
                return Err("cancelled".into());
            }
            fetched.content_id = check(&fetched)?;
            Ok(Some(Box::new(fetched)))
        };
        let _ = tx.send(run());
    })?;
    Ok(Following { want, cancel, answer: Mutex::new(rx) })
}

pub fn follow_changes(
    said: Option<Res<ServerSaid>>,
    held: Option<ResMut<SessionContent>>,
    live: Option<ResMut<LiveSession>>,
    root: Option<Res<super::session_content::SessionRoot>>,
    mut follow: ResMut<Follow>,
) {
    let (Some(said), Some(mut held), Some(mut live), Some(root)) = (said, held, live, root) else {
        if follow.worker.is_some() || follow.next.is_some() || follow.started.is_some() || follow.starts > 0 {
            *follow = Follow::default();
        }
        return;
    };
    if let Some(want) = said.resources_changed
        && want != held.fetched.manifest_hash.0
        && follow.started != Some(want)
        && follow.next != Some(want)
    {
        info!("session: the server's resources changed (manifest {}): reading them again", short(&want));
        match &follow.worker {
            Some(w) => {
                w.cancel.store(true, Ordering::Release);
                follow.next = Some(want);
            }
            None => {
                if let Err(e) = begin(&mut follow, &held, want, &root.0) {
                    return end(&mut live, e);
                }
            }
        }
    }
    let answer = match follow.worker.as_ref().map(|f| f.answer.lock().map(|rx| rx.try_recv())) {
        Some(Ok(Ok(a))) => a,
        Some(Ok(Err(TryRecvError::Empty))) => return,
        Some(_) => Err("the reading thread ended without its answer".into()),
        None => return,
    };
    let superseded = follow.worker.as_ref().is_some_and(|w| w.cancel.load(Ordering::Acquire));
    let done = follow.worker.take().map(|w| w.want);
    match answer {
        Ok(Some(fetched)) => {
            info!(
                "session: the server's resources followed: {} resources, {} bytes fetched, {} from the cache",
                fetched.manifest.resources.len(),
                fetched.fetched_bytes,
                fetched.cached_bytes
            );
            #[cfg(feature = "scripts")]
            {
                let app_dir = ssx_assets::setup::app_dir();
                let allowed = live.content.scripts_allowed;
                let generation = live.content.generation + 1;
                match super::session_content::live_content(&fetched, &held.server_key, &app_dir, allowed) {
                    Ok(c) => {
                        let markers = live.content.markers;
                        live.content = super::live::LiveContent { generation, markers, ..c };
                    }
                    Err(e) => return end(&mut live, e),
                }
            }
            held.fetched = fetched;
        }
        Ok(None) => info!(
            "session: the server's manifest is the session's own (told {}): nothing to follow",
            done.as_ref().map_or_else(String::new, short)
        ),
        Err(e) if superseded => info!("session: a newer change superseded the one being followed ({e})"),
        Err(e) => return end(&mut live, e),
    }
    if let Some(next) = follow.next.take()
        && next != held.fetched.manifest_hash.0
        && let Err(e) = begin(&mut follow, &held, next, &root.0)
    {
        end(&mut live, e);
    }
}

fn begin(follow: &mut Follow, held: &SessionContent, want: [u8; 32], root: &std::path::Path) -> Result<(), String> {
    let worker = start(held, want, root).map_err(|e| format!("the reading thread does not start: {e}"))?;
    follow.started = Some(want);
    follow.worker = Some(worker);
    follow.starts += 1;
    Ok(())
}

fn end(live: &mut LiveSession, why: String) {
    warn!("session: the server's resources changed and this game cannot follow: {why}");
    if live.end.is_none() {
        let reason = Some(format!("the server's resources changed and this game cannot follow: {why}"));
        live.end = Some(LiveEnd { reason, refusal: None });
    }
}

fn short(hash: &[u8; 32]) -> String {
    hash[..8].iter().map(|b| format!("{b:02x}")).collect()
}
