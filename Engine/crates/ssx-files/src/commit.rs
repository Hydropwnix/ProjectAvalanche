use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Mutex, PoisonError};

use crate::error::Error;
use crate::write::Prepared;

struct Shared {
    stop: AtomicBool,
    first_error: Mutex<Option<Error>>,
}

pub struct Committer<'a> {
    queue: Option<SyncSender<Prepared>>,
    shared: &'a Shared,
}

impl Committer<'_> {
    pub fn commit(&self, file: Prepared) -> Result<(), Error> {
        if self.shared.stop.load(Ordering::Acquire) {
            let target = file.target();
            drop(file);
            return Err(Error::new(target, io::Error::other("not written: the commit of another file failed")));
        }
        match &self.queue {
            Some(queue) => queue.send(file).map_err(|unsent| {
                Error::new(unsent.0.target(), io::Error::other("not written: the committing threads have ended"))
            }),
            None => file.commit(),
        }
    }
}

pub fn committing<R, E: From<Error>>(
    threads: usize,
    body: impl FnOnce(&Committer<'_>) -> Result<R, E>,
) -> Result<R, E> {
    let shared = Shared { stop: AtomicBool::new(false), first_error: Mutex::new(None) };
    let (queue, files) = mpsc::sync_channel::<Prepared>(threads.max(1));
    let files = Mutex::new(files);
    let out = std::thread::scope(|scope| {
        let mut started = 0;
        for _ in 0..threads {
            if std::thread::Builder::new()
                .name("ssx-files commit".into())
                .spawn_scoped(scope, || work(&files, &shared))
                .is_ok()
            {
                started += 1;
            }
        }
        let committer = Committer { queue: (started > 0).then_some(queue), shared: &shared };
        let out = body(&committer);
        if out.is_err() {
            shared.stop.store(true, Ordering::Release);
        }
        drop(committer);
        out
    });
    match shared.first_error.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some(e) => Err(e.into()),
        None => out,
    }
}

fn work(files: &Mutex<Receiver<Prepared>>, shared: &Shared) {
    loop {
        let next = files.lock().unwrap_or_else(PoisonError::into_inner).recv();
        let Ok(file) = next else { return };
        if shared.stop.load(Ordering::Acquire) {
            drop(file);
            continue;
        }
        if let Err(e) = file.commit() {
            shared.stop.store(true, Ordering::Release);
            shared.first_error.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(e);
        }
    }
}
