use std::collections::VecDeque;

use super::elements::ClientId;
use crate::value::ElementId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Peer {
    Server,
    Client(ClientId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipients {
    All,
    Some(Vec<ClientId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    Server,
    Clients(Recipients),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Event { name: String, source: ElementId, args: Vec<u8> },
    LatentStart { transfer: u64, name: String, source: ElementId, total: u32 },
    LatentChunk { transfer: u64, offset: u32, bytes: Vec<u8> },
    LatentCancel { transfer: u64 },
    DataWrite { element: ElementId, key: String, value: Option<Vec<u8>> },
    Reset,
    Created { element: ElementId, kind: String, name: String, parent: ElementId },
    Destroyed { element: ElementId },
    Moved { element: ElementId, parent: ElementId },
    Renamed { element: ElementId, name: String },
    Data { element: ElementId, key: String, value: Option<Vec<u8>> },
    Hello { player: ElementId },
    Rider { element: ElementId, player: ElementId, number: u16 },
    Marker { element: ElementId, spec: MarkerSpec },
    Notice { title: String, text: String, seconds: f32 },
    Sound { id: String, position: Option<[f32; 3]> },
    Started,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerSpec {
    pub id: String,
    pub position: [f32; 3],
    pub heading: f32,
    pub size: f32,
    pub colour: u32,
    pub visible: bool,
    pub checkpoint: Option<u16>,
    pub instance: Option<ElementId>,
}

impl Body {
    pub fn size(&self) -> usize {
        16 + match self {
            Body::Event { name, args, .. } => name.len() + args.len(),
            Body::LatentStart { name, .. } => name.len(),
            Body::LatentChunk { bytes, .. } => bytes.len(),
            Body::LatentCancel { .. } | Body::Destroyed { .. } | Body::Moved { .. } | Body::Reset | Body::Started => 0,
            Body::DataWrite { key, value, .. } | Body::Data { key, value, .. } => {
                key.len() + value.as_ref().map_or(0, Vec::len)
            }
            Body::Created { kind, name, .. } => kind.len() + name.len(),
            Body::Renamed { name, .. } => name.len(),
            Body::Hello { .. } | Body::Rider { .. } => 0,
            Body::Marker { spec, .. } => spec.id.len() + 32,
            Body::Notice { title, text, .. } => title.len() + text.len(),
            Body::Sound { id, .. } => id.len() + 12,
        }
    }

    pub fn is_replication(&self) -> bool {
        matches!(
            self,
            Body::Reset
                | Body::Created { .. }
                | Body::Destroyed { .. }
                | Body::Moved { .. }
                | Body::Renamed { .. }
                | Body::Data { .. }
                | Body::Hello { .. }
                | Body::Rider { .. }
                | Body::Marker { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub package: Option<String>,
    pub to: Destination,
    pub body: Body,
}

#[derive(Debug, Default)]
pub struct Drained {
    pub messages: Vec<Message>,
    pub resync: bool,
}

pub struct Outbox {
    queue: VecDeque<Message>,
    bytes: usize,
    resync: bool,
    max_messages: usize,
    max_bytes: usize,
}

impl Outbox {
    pub fn new(max_messages: usize, max_bytes: usize) -> Self {
        Outbox { queue: VecDeque::new(), bytes: 0, resync: false, max_messages, max_bytes }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn fits(&self, size: usize) -> bool {
        self.queue.len() < self.max_messages && self.bytes + size <= self.max_bytes
    }

    pub fn push(&mut self, m: Message) -> Result<(), String> {
        let size = m.body.size();
        if !self.fits(size) {
            return Err(format!(
                "the outbox is full ({} messages, {} bytes; at most {} and {})",
                self.queue.len(),
                self.bytes,
                self.max_messages,
                self.max_bytes
            ));
        }
        self.bytes += size;
        self.queue.push_back(m);
        Ok(())
    }

    pub fn push_replication(&mut self, m: Message) {
        if self.resync {
            return;
        }
        if !self.fits(m.body.size()) {
            self.queue.retain(|q| !q.body.is_replication());
            self.bytes = self.queue.iter().map(|q| q.body.size()).sum();
            self.resync = true;
            return;
        }
        self.bytes += m.body.size();
        self.queue.push_back(m);
    }

    pub fn drain(&mut self, max: usize) -> Drained {
        let n = max.min(self.queue.len());
        let messages: Vec<Message> = self.queue.drain(..n).collect();
        self.bytes -= messages.iter().map(|m| m.body.size()).sum::<usize>();
        Drained { messages, resync: std::mem::take(&mut self.resync) }
    }

    pub fn purge_package(&mut self, package: &str) {
        self.queue.retain(|m| m.body.is_replication() || m.package.as_deref() != Some(package));
        self.bytes = self.queue.iter().map(|m| m.body.size()).sum();
    }

    pub fn count_package(&self, package: &str) -> usize {
        self.queue.iter().filter(|m| !m.body.is_replication() && m.package.as_deref() == Some(package)).count()
    }
}
