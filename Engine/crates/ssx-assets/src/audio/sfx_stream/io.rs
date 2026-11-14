use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, OnceLock};

use super::StreamFile;
use super::parse::BlockKind;

pub(super) struct Request {
    pub tag: u64,
    pub file: Arc<StreamFile>,
    pub offset: u64,
    pub kind: BlockKind,
}

pub(super) enum Msg {
    Open { slot: u64, chunk: usize, chunks: usize, max_block: usize, to: Sender<Delivery> },
    Queue { slot: u64, epoch: u32, req: Request },
    Flush { slot: u64, epoch: u32 },
    Cancel { slot: u64, epoch: u32, tag: u64 },
    Spent { slot: u64, buf: Vec<u8> },
    Close { slot: u64 },
    Spawn { name: &'static str, work: Box<dyn FnOnce() + Send> },
}

pub(super) enum Delivery {
    Data { epoch: u32, tag: u64, buf: Vec<u8>, last: bool },
    Failed { epoch: u32, tag: u64, error: String },
}

pub(super) fn sender() -> Option<&'static Sender<Msg>> {
    static IO: OnceLock<Option<Sender<Msg>>> = OnceLock::new();
    IO.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("ssx_stream_io".into()).spawn(move || run(rx)).ok().map(|_| tx)
    })
    .as_ref()
}

struct SlotIo {
    to: Sender<Delivery>,
    chunk: usize,
    chunks: usize,
    max_block: usize,
    made: usize,
    free: Vec<Vec<u8>>,
    epoch: u32,
    queue: VecDeque<Request>,
    cur: Option<Reading>,
}

struct Reading {
    req: Request,
    pos: u64,
    head: u64,
    end: Option<u64>,
}

fn run(rx: Receiver<Msg>) {
    let mut slots: HashMap<u64, SlotIo> = HashMap::new();
    let mut order: Vec<u64> = Vec::new();
    loop {
        let mut busy = false;
        order.clear();
        order.extend(slots.keys().copied());
        order.sort_unstable();
        for id in &order {
            let Some(s) = slots.get_mut(id) else { continue };
            match s.read_one() {
                Step::Read => busy = true,
                Step::Idle => {}
                Step::Gone => {
                    slots.remove(id);
                }
            }
        }
        let msg = if busy {
            match rx.try_recv() {
                Ok(m) => Some(m),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            }
        };
        let mut next = msg;
        while let Some(m) = next {
            handle(&mut slots, m);
            next = rx.try_recv().ok();
        }
    }
}

fn handle(slots: &mut HashMap<u64, SlotIo>, m: Msg) {
    match m {
        Msg::Open { slot, chunk, chunks, max_block, to } => {
            let s = SlotIo {
                to,
                chunk,
                chunks,
                max_block,
                made: 0,
                free: Vec::new(),
                epoch: 0,
                queue: VecDeque::new(),
                cur: None,
            };
            slots.insert(slot, s);
        }
        Msg::Queue { slot, epoch, req } => {
            if let Some(s) = slots.get_mut(&slot).filter(|s| s.epoch == epoch) {
                s.queue.push_back(req);
            }
        }
        Msg::Flush { slot, epoch } => {
            if let Some(s) = slots.get_mut(&slot) {
                s.epoch = epoch;
                s.queue.clear();
                s.cur = None;
            }
        }
        Msg::Cancel { slot, epoch, tag } => {
            if let Some(s) = slots.get_mut(&slot).filter(|s| s.epoch == epoch) {
                if s.cur.as_ref().is_some_and(|r| r.req.tag == tag) {
                    s.cur = None;
                }
                s.queue.retain(|r| r.tag != tag);
            }
        }
        Msg::Spent { slot, mut buf } => {
            if let Some(s) = slots.get_mut(&slot) {
                buf.clear();
                s.free.push(buf);
            }
        }
        Msg::Close { slot } => {
            slots.remove(&slot);
        }
        Msg::Spawn { name, work } => {
            let _ = std::thread::Builder::new().name(name.into()).spawn(work);
        }
    }
}

enum Step {
    Read,
    Idle,
    Gone,
}

impl SlotIo {
    fn read_one(&mut self) -> Step {
        if self.cur.is_none() {
            let Some(req) = self.queue.pop_front() else { return Step::Idle };
            self.cur = Some(Reading { pos: req.offset, head: req.offset, end: None, req });
        }
        let mut buf = match self.free.pop() {
            Some(b) => b,
            None if self.made < self.chunks => {
                self.made += 1;
                Vec::with_capacity(self.chunk)
            }
            None => return Step::Idle,
        };
        let Some(r) = self.cur.as_mut() else { return Step::Idle };
        let (epoch, tag) = (self.epoch, r.req.tag);
        let delivery = match r.read(&mut buf, self.chunk, self.max_block) {
            Ok(last) => {
                if last {
                    self.cur = None;
                }
                Delivery::Data { epoch, tag, buf, last }
            }
            Err(error) => {
                self.cur = None;
                self.free.push(buf);
                Delivery::Failed { epoch, tag, error }
            }
        };
        if self.to.send(delivery).is_err() {
            return Step::Gone;
        }
        Step::Read
    }
}

impl Reading {
    fn read(&mut self, buf: &mut Vec<u8>, chunk: usize, max_block: usize) -> Result<bool, String> {
        let file = &self.req.file;
        let stop = self.end.unwrap_or(u64::MAX).min(file.len());
        let left = stop.saturating_sub(self.pos);
        if left == 0 {
            return Err(format!("{}: the file ends at {:#x} before the stream's last block", file.name(), self.pos));
        }
        let n = usize::try_from(left).unwrap_or(usize::MAX).min(chunk);
        buf.resize(n, 0);
        let got =
            file.read_at(buf, self.pos).map_err(|e| format!("{}: reading at {:#x}: {e}", file.name(), self.pos))?;
        if got < n {
            return Err(format!("{}: a short read at {:#x}", file.name(), self.pos + got as u64));
        }
        let end = self.pos + n as u64;
        let kind = self.req.kind;
        let need = kind.head_bytes();
        while self.end.is_none() && self.head + need as u64 <= end {
            let mut word = [0u8; super::parse::HEAD_BYTES];
            let at = self.head;
            if at >= self.pos {
                let k = (at - self.pos) as usize;
                word[..need].copy_from_slice(&buf[k..k + need]);
            } else {
                let got = file.read_at(&mut word[..need], at).map_err(|e| format!("{}: {e}", file.name()))?;
                if got < need {
                    return Err(format!("{}: a block head at {at:#x} past the file's end", file.name()));
                }
            }
            let head = kind.head(&word[..need], max_block).map_err(|e| format!("{} at {at:#x}: {e:#}", file.name()))?;
            self.head += head.size as u64;
            if head.last {
                self.end = Some(self.head);
            }
        }
        match self.end {
            Some(e) if e <= end => {
                buf.truncate((e - self.pos) as usize);
                self.pos = e;
                Ok(true)
            }
            _ => {
                self.pos = end;
                Ok(false)
            }
        }
    }
}
