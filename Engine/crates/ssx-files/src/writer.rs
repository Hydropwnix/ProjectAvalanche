use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum Asked<J> {
    Queued,
    Replaced(J),
    Ended(J),
}

struct Queue<J> {
    next: Option<J>,
    busy: bool,
    closing: bool,
    ended: bool,
}

struct Shared<J> {
    queue: Mutex<Queue<J>>,
    changed: Condvar,
}

impl<J> Shared<J> {
    fn lock(&self) -> MutexGuard<'_, Queue<J>> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct EndGuard<'a, J>(&'a Shared<J>);

impl<J> Drop for EndGuard<'_, J> {
    fn drop(&mut self) {
        let mut q = self.0.lock();
        q.ended = true;
        q.busy = false;
        drop(q);
        self.0.changed.notify_all();
    }
}

pub struct Writer<J: Send + 'static> {
    shared: Arc<Shared<J>>,
    worker: Option<JoinHandle<()>>,
}

impl<J: Send + 'static> Writer<J> {
    pub fn start(name: &str, write: impl FnMut(J) + Send + 'static) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue { next: None, busy: false, closing: false, ended: false }),
            changed: Condvar::new(),
        });
        let s = Arc::clone(&shared);
        let worker = std::thread::Builder::new().name(name.into()).spawn(move || work(&s, write))?;
        Ok(Writer { shared, worker: Some(worker) })
    }

    pub fn ask(&self, job: J) -> Asked<J> {
        let mut q = self.shared.lock();
        if q.ended {
            return Asked::Ended(job);
        }
        let old = q.next.replace(job);
        drop(q);
        self.shared.changed.notify_all();
        old.map_or(Asked::Queued, Asked::Replaced)
    }

    pub fn writing(&self) -> bool {
        let q = self.shared.lock();
        !q.ended && (q.next.is_some() || q.busy)
    }

    pub fn settle(&self) {
        let mut q = self.shared.lock();
        while !q.ended && (q.next.is_some() || q.busy) {
            q = self.shared.changed.wait(q).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn finish(&mut self) -> bool {
        self.shared.lock().closing = true;
        self.shared.changed.notify_all();
        self.worker.take().is_none_or(|w| w.join().is_ok())
    }
}

impl<J: Send + 'static> Drop for Writer<J> {
    fn drop(&mut self) {
        self.finish();
    }
}

fn work<J>(shared: &Shared<J>, mut write: impl FnMut(J)) {
    let _end = EndGuard(shared);
    let mut q = shared.lock();
    loop {
        if let Some(job) = q.next.take() {
            q.busy = true;
            drop(q);
            write(job);
            q = shared.lock();
            q.busy = false;
            shared.changed.notify_all();
        } else if q.closing {
            return;
        } else {
            q = shared.changed.wait(q).unwrap_or_else(PoisonError::into_inner);
        }
    }
}
