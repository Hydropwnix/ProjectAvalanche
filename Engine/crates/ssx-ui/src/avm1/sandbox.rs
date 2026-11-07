use std::collections::HashMap;

use crate::library::MovieId;

pub mod caps {
    pub const WORK_PER_FRAME: u64 = 1_000_000;
    pub const STACK_VALUES: usize = 1 << 16;
    pub const CALLS_PER_FRAME: u32 = 1024;
    pub const STRING_BYTES: usize = 1 << 20;
    pub const ARRAY_LEN: usize = 1 << 20;
    pub const PROPS: usize = 1 << 16;
    pub const OBJECTS: usize = 1 << 21;
    pub const HEAP_BYTES: u64 = 256 << 20;
    pub const TRACE_LINES: usize = 16_384;
    pub const TRACE_BYTES: usize = 1024;
    pub const DIAGNOSTICS: usize = 16_384;
    pub const DETAIL_BYTES: usize = 256;
    pub const CALLS_KEPT: usize = 4096;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Charge {
    pub work: u64,
    pub calls: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub movie: MovieId,
    pub why: String,
}

#[derive(Default)]
pub struct Sandbox {
    pub charged: Option<MovieId>,
    pub frame: HashMap<MovieId, Charge>,
    pub stopped: HashMap<MovieId, String>,
    pub new_stops: Vec<Stop>,
    pub heap_bytes: u64,
    pub peaks: Peaks,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Peaks {
    pub frame_actions: u64,
    pub block_actions: u64,
    pub call_depth: u32,
    pub stack: usize,
    pub objects: usize,
    pub heap_bytes: u64,
    pub string_bytes: usize,
    pub array_len: usize,
    pub props: usize,
    pub natives_per_frame: u32,
    pub loads_queued: usize,
}

impl Sandbox {
    pub fn new_frame(&mut self) {
        self.frame.clear();
    }

    pub fn stop(&mut self, movie: MovieId, why: String) {
        if self.stopped.contains_key(&movie) {
            return;
        }
        self.stopped.insert(movie, why.clone());
        self.new_stops.push(Stop { movie, why });
    }

    pub fn is_stopped(&self, movie: MovieId) -> bool {
        self.stopped.contains_key(&movie)
    }

    pub fn work(&mut self, units: u64) -> Option<(MovieId, String)> {
        let m = self.charged?;
        let c = self.frame.entry(m).or_default();
        c.work = c.work.saturating_add(units);
        (c.work > caps::WORK_PER_FRAME)
            .then(|| (m, format!("more than {} actions and work units in one frame", caps::WORK_PER_FRAME)))
    }

    pub fn native_call(&mut self) -> Option<(MovieId, String)> {
        let m = self.charged?;
        let c = self.frame.entry(m).or_default();
        c.calls = c.calls.saturating_add(1);
        (c.calls > caps::CALLS_PER_FRAME)
            .then(|| (m, format!("more than {} calls into the game in one frame", caps::CALLS_PER_FRAME)))
    }

    pub fn grow(&mut self, bytes: u64) {
        self.heap_bytes = self.heap_bytes.saturating_add(bytes);
        self.peaks.heap_bytes = self.peaks.heap_bytes.max(self.heap_bytes);
    }

    pub fn heap_over(&self, made: usize, live: usize) -> Option<(MovieId, String)> {
        let m = self.charged?;
        if live > caps::OBJECTS {
            return Some((m, format!("more than {} objects", caps::OBJECTS)));
        }
        let bytes = self.heap_bytes.saturating_add(made as u64 * OBJECT_BYTES);
        (bytes > caps::HEAP_BYTES).then(|| {
            (
                m,
                format!(
                    "the scripts' data passed {} MiB (objects, properties, arrays, strings)",
                    caps::HEAP_BYTES >> 20
                ),
            )
        })
    }
}

pub const OBJECT_BYTES: u64 = 160;
pub const PROPERTY_BYTES: u64 = 64;
pub const ELEMENT_BYTES: u64 = 24;

impl super::Ctx<'_> {
    pub fn made_string(&mut self, bytes: usize) -> Result<(), super::Fault> {
        if bytes > caps::STRING_BYTES {
            return Err(super::Fault::Abort("a string of more than 1 MiB"));
        }
        let peaks = &mut self.vm.sandbox.peaks;
        peaks.string_bytes = peaks.string_bytes.max(bytes);
        self.vm.sandbox.grow(bytes as u64);
        self.charge(bytes as u64 / 64)
    }

    pub fn grew_array(&mut self, len: usize, added: usize) -> Result<(), super::Fault> {
        if len > caps::ARRAY_LEN {
            return Err(super::Fault::Abort("an array of more than 1 048 576 elements"));
        }
        let peaks = &mut self.vm.sandbox.peaks;
        peaks.array_len = peaks.array_len.max(len);
        self.vm.sandbox.grow(added as u64 * ELEMENT_BYTES);
        self.charge(added as u64 / 4)
    }

    pub fn walked(&mut self, n: usize) -> Result<(), super::Fault> {
        self.charge(n as u64)
    }
}

#[cfg(test)]
mod tests;
