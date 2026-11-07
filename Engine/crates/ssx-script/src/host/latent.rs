use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use super::outbox::{Body, Destination, Message, Outbox, Peer};
use crate::value::ElementId;

const MIN_CHUNK: usize = 1024;
pub const MAX_CHUNK: usize = 64 << 10;

pub struct Transfer {
    pub handle: u64,
    pub package: String,
    pub to: Destination,
    pub name: String,
    pub source: ElementId,
    pub payload: Vec<u8>,
    pub sent: usize,
    pub rate: u32,
    credit: f64,
    started: bool,
}

#[derive(Default)]
pub struct LatentOut {
    transfers: Vec<Transfer>,
    next: u64,
    total_credit: f64,
    first: usize,
}

impl LatentOut {
    pub fn start(
        &mut self,
        package: &str,
        to: Destination,
        name: &str,
        source: ElementId,
        payload: Vec<u8>,
        rate: u32,
    ) -> u64 {
        self.next += 1;
        self.transfers.push(Transfer {
            handle: self.next,
            package: package.to_string(),
            to,
            name: name.to_string(),
            source,
            payload,
            sent: 0,
            rate,
            credit: 0.0,
            started: false,
        });
        self.next
    }

    pub fn count(&self, package: &str) -> usize {
        self.transfers.iter().filter(|t| t.package == package).count()
    }

    pub fn handles(&self, package: &str) -> Vec<u64> {
        self.transfers.iter().filter(|t| t.package == package).map(|t| t.handle).collect()
    }

    pub fn status(&self, package: &str, handle: u64) -> Option<(usize, usize)> {
        self.transfers.iter().find(|t| t.handle == handle && t.package == package).map(|t| (t.payload.len(), t.sent))
    }

    pub fn cancel(&mut self, package: &str, handle: u64, outbox: &mut Outbox) -> bool {
        let Some(at) = self.transfers.iter().position(|t| t.handle == handle && t.package == package) else {
            return false;
        };
        let t = self.transfers.remove(at);
        if t.started {
            let body = Body::LatentCancel { transfer: t.handle };
            let _ = outbox.push(Message { package: Some(t.package), to: t.to, body });
        }
        true
    }

    pub fn remove_package(&mut self, package: &str) {
        self.transfers.retain(|t| t.package != package);
    }

    pub fn step(&mut self, dt: Duration, chunk: usize, total: Option<u32>, outbox: &mut Outbox) {
        let chunk = chunk.clamp(MIN_CHUNK, MAX_CHUNK);
        if let Some(r) = total {
            let r = f64::from(r);
            self.total_credit = (self.total_credit + r * dt.as_secs_f64()).min(r.max(chunk as f64));
        }
        let n = self.transfers.len();
        self.first = if n == 0 { 0 } else { (self.first + 1) % n };
        for k in 0..n {
            let t = &mut self.transfers[(self.first + k) % n];
            let rate = f64::from(t.rate);
            t.credit = (t.credit + rate * dt.as_secs_f64()).min(rate.max(chunk as f64));
            if !t.started {
                let total = u32::try_from(t.payload.len()).unwrap_or(u32::MAX);
                let body = Body::LatentStart { transfer: t.handle, name: t.name.clone(), source: t.source, total };
                let m = Message { package: Some(t.package.clone()), to: t.to.clone(), body };
                if outbox.push(m).is_err() {
                    continue;
                }
                t.started = true;
            }
            while t.sent < t.payload.len() {
                let left = t.payload.len() - t.sent;
                let mut size = left.min(chunk).min(t.credit.max(0.0) as usize);
                if total.is_some() {
                    size = size.min(self.total_credit.max(0.0) as usize);
                }
                if size < left.min(MIN_CHUNK) {
                    break;
                }
                let offset = u32::try_from(t.sent).unwrap_or(u32::MAX);
                let bytes = t.payload[t.sent..t.sent + size].to_vec();
                let body = Body::LatentChunk { transfer: t.handle, offset, bytes };
                if outbox.push(Message { package: Some(t.package.clone()), to: t.to.clone(), body }).is_err() {
                    break;
                }
                t.sent += size;
                t.credit -= size as f64;
                if total.is_some() {
                    self.total_credit -= size as f64;
                }
            }
        }
        self.transfers.retain(|t| t.sent < t.payload.len() || !t.started);
    }
}

pub struct Partial {
    pub package: String,
    pub name: String,
    pub source: ElementId,
    pub total: usize,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
pub struct ReceiveCaps {
    pub total: usize,
    pub transfers: usize,
    pub buffered: usize,
}

#[derive(Default)]
pub struct LatentIn {
    peers: HashMap<Peer, BTreeMap<u64, Partial>>,
}

impl LatentIn {
    pub fn start(&mut self, peer: Peer, caps: ReceiveCaps, transfer: u64, p: Partial) -> Result<(), String> {
        let open = self.peers.entry(peer).or_default();
        if open.contains_key(&transfer) {
            return Err(format!("latent transfer {transfer} is already open"));
        }
        if p.total > caps.total {
            return Err(format!("a latent transfer of {} bytes (at most {})", p.total, caps.total));
        }
        if open.len() >= caps.transfers {
            return Err(format!("{} latent transfers open from this peer (at most {})", open.len(), caps.transfers));
        }
        let held: usize = open.values().map(|p| p.total).sum();
        if held + p.total > caps.buffered {
            return Err(format!("latent transfers of {} bytes open from this peer (at most {})", held, caps.buffered));
        }
        open.insert(transfer, p);
        Ok(())
    }

    pub fn chunk(&mut self, peer: Peer, transfer: u64, offset: u32, bytes: &[u8]) -> Result<Option<Partial>, String> {
        let open = self.peers.get_mut(&peer).ok_or("no latent transfer open from this peer")?;
        let p = open.get_mut(&transfer).ok_or_else(|| format!("latent transfer {transfer} is not open"))?;
        let at = usize::try_from(offset).map_err(|_| "a chunk offset beyond this system's sizes")?;
        if at != p.bytes.len() || bytes.len() > MAX_CHUNK || bytes.is_empty() || at + bytes.len() > p.total {
            open.remove(&transfer);
            return Err(format!("latent transfer {transfer}: a chunk out of order or past its end (dropped)"));
        }
        p.bytes.extend_from_slice(bytes);
        if p.bytes.len() == p.total { Ok(open.remove(&transfer)) } else { Ok(None) }
    }

    pub fn cancel(&mut self, peer: Peer, transfer: u64) {
        if let Some(open) = self.peers.get_mut(&peer) {
            open.remove(&transfer);
        }
    }

    pub fn drop_peer(&mut self, peer: Peer) {
        self.peers.remove(&peer);
    }

    pub fn remove_package(&mut self, package: &str) {
        for open in self.peers.values_mut() {
            open.retain(|_, p| p.package != package);
        }
    }

    pub fn open(&self) -> usize {
        self.peers.values().map(BTreeMap::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfers_share_the_hosts_total_rate() {
        let mut out = LatentOut::default();
        let mut outbox = Outbox::new(1 << 16, 1 << 30);
        let to = Destination::Server;
        let a = out.start("p", to.clone(), "e", ElementId(1), vec![1; 1 << 20], 1_000_000);
        let b = out.start("p", to, "e", ElementId(1), vec![2; 1 << 20], 1_000_000);
        let total = 64 << 10;
        let mut sent: BTreeMap<u64, usize> = BTreeMap::new();
        let ms = Duration::from_millis;
        for step in 0..200 {
            out.step(if step == 0 { ms(1000) } else { ms(50) }, 16 << 10, Some(total), &mut outbox);
            for m in outbox.drain(usize::MAX).messages {
                if let Body::LatentChunk { transfer, bytes, .. } = m.body {
                    *sent.entry(transfer).or_default() += bytes.len();
                }
            }
        }
        let all: usize = sent.values().sum();
        let allowed = (total as f64 * 10.95) as usize;
        assert!(all <= allowed && all + 2 * (16 << 10) >= allowed, "{all} of {allowed}");
        let (sa, sb) = (sent[&a], sent[&b]);
        assert!(sa.abs_diff(sb) <= total as usize + (16 << 10), "shared: {sa} and {sb}");
        assert!(sa.min(sb) > all / 3, "neither starves: {sa} and {sb}");
        let mut out = LatentOut::default();
        out.start("p", Destination::Server, "e", ElementId(1), vec![1; 1 << 20], 200_000);
        out.step(ms(1000), 16 << 10, None, &mut outbox);
        let one: usize = outbox
            .drain(usize::MAX)
            .messages
            .iter()
            .map(|m| if let Body::LatentChunk { bytes, .. } = &m.body { bytes.len() } else { 0 })
            .sum();
        assert!(one <= 200_000 && one > 200_000 - (16 << 10), "{one}");
    }
}
