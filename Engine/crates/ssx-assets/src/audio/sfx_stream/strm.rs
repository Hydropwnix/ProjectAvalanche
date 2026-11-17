use std::collections::VecDeque;

pub(super) const LOW: i32 = -1000;
pub(super) const HIGH: i32 = 1000;
pub(super) const ON: i32 = HIGH - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Queued {
    pub priority: i32,
    pub order: u64,
}

impl Queued {
    pub fn before(&self, other: &Queued) -> bool {
        (self.priority, std::cmp::Reverse(self.order)) > (other.priority, std::cmp::Reverse(other.order))
    }
}

#[derive(Debug, Default)]
pub(super) struct Order(u64);

impl Order {
    fn next(&mut self) -> u64 {
        self.0 += 1;
        self.0
    }
}

#[derive(Debug)]
pub(super) struct Strm {
    pub read: Option<Queued>,
    pub stalled: bool,
    pub urgent: bool,
    threshold: u64,
    pub buffered: u64,
    fills: VecDeque<(u64, u64)>,
    keep: usize,
}

impl Strm {
    pub fn new(ring: usize, chunks: usize) -> Self {
        Self {
            read: None,
            stalled: false,
            urgent: false,
            threshold: ring as u64 / 3,
            buffered: 0,
            fills: VecDeque::new(),
            keep: chunks.max(1),
        }
    }

    pub fn wake(&self) -> i32 {
        if self.urgent { HIGH } else { LOW }
    }

    pub fn want(&mut self, priority: i32, room: bool, order: &mut Order) {
        if room {
            self.stalled = false;
            self.read = Some(Queued { priority, order: order.next() });
        } else {
            self.stalled = true;
            self.read = None;
        }
    }

    pub fn started(&mut self, tag: u64) {
        self.fills.push_back((tag, 0));
        while self.fills.len() > self.keep {
            self.fills.pop_front();
        }
    }

    pub fn fill(&mut self, tag: u64, n: u64) {
        if let Some(f) = self.fills.iter_mut().rev().find(|f| f.0 == tag) {
            f.1 += n;
        }
        let before = self.buffered;
        self.buffered = before.saturating_add(n);
        if before < self.threshold && self.buffered >= self.threshold {
            self.urgent = false;
        }
    }

    pub fn consume(&mut self, n: u64, order: &mut Order) {
        let before = self.buffered;
        self.buffered = before.saturating_sub(n);
        if before >= self.threshold && self.buffered < self.threshold {
            self.urgent = true;
            if let Some(q) = self.read.as_mut()
                && q.priority != HIGH
            {
                *q = Queued { priority: HIGH, order: order.next() };
            }
        }
    }

    pub fn cancel(&mut self, tag: u64, released: u64, order: &mut Order) {
        let Some(k) = self.fills.iter().position(|f| f.0 == tag) else { return };
        let (_, filled) = self.fills.remove(k).unwrap_or((tag, 0));
        self.consume(filled.saturating_sub(released), order);
    }

    pub fn flush(&mut self, order: &mut Order) {
        self.consume(self.buffered, order);
        self.fills.clear();
        self.read = None;
        self.stalled = false;
    }
}

pub(super) fn pick<'a, K: Copy + 'a>(reads: impl IntoIterator<Item = (K, &'a Strm)>) -> Option<K> {
    let mut best: Option<(K, Queued)> = None;
    for (k, s) in reads {
        if let Some(q) = s.read
            && best.is_none_or(|(_, b)| q.before(&b))
        {
            best = Some((k, q));
        }
    }
    best.map(|(k, _)| k)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strm() -> Strm {
        Strm::new(600, 6)
    }

    #[test]
    fn the_constructor_s_priorities_and_threshold() {
        let s = strm();
        assert_eq!((LOW, HIGH, ON), (-1000, 1000, 999));
        assert_eq!((s.threshold, s.urgent, s.buffered, s.read, s.stalled), (200, false, 0, None, false));
    }

    #[test]
    fn an_idle_stream_reads_first_at_the_low_priority_and_urgent_at_the_high() {
        let mut order = Order::default();
        let mut s = strm();
        s.want(s.wake(), true, &mut order);
        assert_eq!(s.read.map(|q| q.priority), Some(LOW));
        s.fill(1, 300);
        s.consume(150, &mut order);
        assert!(s.urgent, "300 → 150 crosses below a third");
        assert_eq!(s.wake(), HIGH);
    }

    #[test]
    fn a_read_without_room_waits_and_restarts_by_the_urgent_flag() {
        let mut order = Order::default();
        let mut s = strm();
        s.want(ON, false, &mut order);
        assert!(s.stalled && s.read.is_none());
        s.want(s.wake(), true, &mut order);
        assert!(!s.stalled);
        assert_eq!(s.read.map(|q| q.priority), Some(LOW));
    }

    #[test]
    fn falling_below_a_third_raises_the_queued_read_behind_its_equals() {
        let mut order = Order::default();
        let (mut a, mut b) = (strm(), strm());
        a.fill(1, 250);
        a.want(ON, true, &mut order);
        b.want(HIGH, true, &mut order);
        assert_eq!(pick([(0, &a), (1, &b)]), Some(1));
        a.consume(100, &mut order);
        assert!(a.urgent);
        assert_eq!(a.read.map(|q| q.priority), Some(HIGH));
        assert_eq!(pick([(0, &a), (1, &b)]), Some(1), "re-queued after b, its equal");
        let mut c = strm();
        c.fill(2, 250);
        c.want(HIGH, true, &mut order);
        let before = c.read;
        c.consume(100, &mut order);
        assert_eq!(c.read, before);
    }

    #[test]
    fn filling_to_a_third_clears_the_urgent_flag() {
        let mut order = Order::default();
        let mut s = strm();
        s.fill(1, 300);
        s.consume(250, &mut order);
        assert!(s.urgent);
        s.fill(1, 100);
        assert!(s.urgent, "150 is below a third");
        s.fill(1, 50);
        assert!(!s.urgent, "200 reaches it");
        s.urgent = true;
        s.fill(1, 10);
        assert!(s.urgent);
    }

    #[test]
    fn the_queue_serves_the_higher_priority_then_the_earlier() {
        let mut order = Order::default();
        let (mut a, mut b, mut c) = (strm(), strm(), strm());
        a.want(LOW, true, &mut order);
        b.want(ON, true, &mut order);
        c.want(ON, true, &mut order);
        assert_eq!(pick([(0, &a), (1, &b), (2, &c)]), Some(1));
        b.read = None;
        assert_eq!(pick([(0, &a), (1, &b), (2, &c)]), Some(2));
        c.read = None;
        assert_eq!(pick([(0, &a), (1, &b), (2, &c)]), Some(0));
        a.read = None;
        assert_eq!(pick([(0, &a), (1, &b), (2, &c)]), None);
    }

    #[test]
    fn a_stop_consumes_the_whole_count() {
        let mut order = Order::default();
        let mut s = strm();
        s.started(1);
        s.fill(1, 400);
        s.want(ON, false, &mut order);
        s.flush(&mut order);
        assert_eq!((s.buffered, s.urgent, s.stalled, s.read), (0, true, false, None));
        let mut t = strm();
        t.fill(1, 100);
        t.flush(&mut order);
        assert!(!t.urgent);
    }

    #[test]
    fn a_request_left_early_gives_up_its_blocks_still_in_the_ring() {
        let mut order = Order::default();
        let mut s = strm();
        s.started(1);
        s.fill(1, 300);
        s.started(2);
        s.fill(2, 200);
        s.consume(100, &mut order);
        s.cancel(1, 100, &mut order);
        assert_eq!(s.buffered, 200, "request 1's other 200 bytes leave");
        assert!(!s.urgent, "500 → 400 → 200 never fell below a third");
        s.cancel(7, 0, &mut order);
        assert_eq!(s.buffered, 200, "a request with nothing read takes nothing");
    }
}
