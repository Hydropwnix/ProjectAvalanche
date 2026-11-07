use std::collections::VecDeque;

use super::{FIRST, LAST, MAX_ITEM, MAX_WIRE, PIECE_HEADER, err};
use crate::DecodeError;

#[derive(Debug, Default)]
pub struct Assembler {
    buf: Vec<u8>,
    open: bool,
}

impl Assembler {
    pub fn take(&mut self, wire: &[u8]) -> Result<Vec<Vec<u8>>, DecodeError> {
        if wire.is_empty() || wire.len() > MAX_WIRE {
            return Err(err(format!("a wire message of {} bytes (1 to {MAX_WIRE})", wire.len())));
        }
        let mut items = Vec::new();
        let mut rest = wire;
        while !rest.is_empty() {
            let Some((&[flags, l0, l1], tail)) = rest.split_first_chunk::<PIECE_HEADER>() else {
                return Err(err("a piece's header cut short"));
            };
            let len = usize::from(u16::from_be_bytes([l0, l1]));
            if flags & !(FIRST | LAST) != 0 || len == 0 || len > tail.len() {
                return Err(err(format!("a piece with flags {flags:#04x} and {len} bytes")));
            }
            if flags & FIRST != 0 {
                if self.open {
                    return Err(err("an item's first piece inside another item"));
                }
                self.buf.clear();
                self.open = true;
            } else if !self.open {
                return Err(err("a piece of no item"));
            }
            if self.buf.len() + len > MAX_ITEM {
                return Err(err(format!("an item beyond {MAX_ITEM} bytes")));
            }
            self.buf.extend_from_slice(&tail[..len]);
            if flags & LAST != 0 {
                items.push(std::mem::take(&mut self.buf));
                self.open = false;
            }
            rest = &tail[len..];
        }
        Ok(items)
    }

    pub fn held(&self) -> usize {
        self.buf.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueFull {
    pub held: usize,
    pub cap: usize,
}

#[derive(Debug)]
pub struct Outgoing<B = Vec<u8>> {
    queue: VecDeque<B>,
    paced: VecDeque<usize>,
    offset: usize,
    held: usize,
    cap: usize,
}

impl<B: AsRef<[u8]>> Outgoing<B> {
    pub fn new(cap: usize) -> Self {
        Self { queue: VecDeque::new(), paced: VecDeque::new(), offset: 0, held: 0, cap }
    }

    pub fn push(&mut self, item: B) -> Result<(), QueueFull> {
        self.push_paced(item, 0)
    }

    pub fn push_paced(&mut self, item: B, paced: usize) -> Result<(), QueueFull> {
        let len = item.as_ref().len();
        if len == 0 || len > MAX_ITEM || self.held + len > self.cap {
            return Err(QueueFull { held: self.held, cap: self.cap });
        }
        self.held += len;
        self.queue.push_back(item);
        self.paced.push_back(paced);
        Ok(())
    }

    pub fn held(&self) -> usize {
        self.held
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn clear(&mut self) {
        let partial = (self.offset > 0).then(|| self.queue.pop_front()).flatten();
        let paced = self.paced.pop_front().unwrap_or(0);
        self.queue.clear();
        self.paced.clear();
        self.held = 0;
        if let Some(item) = partial {
            self.held = item.as_ref().len() - self.offset;
            self.queue.push_back(item);
            self.paced.push_back(paced);
        }
    }

    pub fn take(&mut self, budget: usize) -> Vec<Vec<u8>> {
        self.take_paced(budget, &mut |_| true)
    }

    pub fn take_paced(&mut self, mut budget: usize, pace: &mut dyn FnMut(usize) -> bool) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut m: Vec<u8> = Vec::new();
        while let Some(item) = self.queue.front().map(|i| i.as_ref()) {
            let room = (MAX_WIRE - m.len()).min(budget);
            if room <= PIECE_HEADER {
                if m.is_empty() || budget <= PIECE_HEADER {
                    break;
                }
                out.push(std::mem::take(&mut m));
                continue;
            }
            if self.offset == 0
                && let Some(&paced) = self.paced.front()
                && paced > 0
                && !pace(paced)
            {
                break;
            }
            let rest = item.len() - self.offset;
            let n = rest.min(room - PIECE_HEADER).min(usize::from(u16::MAX));
            let flags = if self.offset == 0 { FIRST } else { 0 } | if n == rest { LAST } else { 0 };
            m.push(flags);
            m.extend_from_slice(&(n as u16).to_be_bytes());
            m.extend_from_slice(&item[self.offset..self.offset + n]);
            budget -= n + PIECE_HEADER;
            self.offset += n;
            self.held -= n;
            if self.offset == item.len() {
                self.queue.pop_front();
                self.paced.pop_front();
                self.offset = 0;
            }
        }
        if !m.is_empty() {
            out.push(m);
        }
        out
    }
}
