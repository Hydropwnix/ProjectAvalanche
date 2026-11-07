use std::cell::Cell;

use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_file_bytes: usize,
    pub max_movie_bytes: usize,
    pub max_inflate_padding: usize,
    pub max_tags: u64,
    pub max_shape_records: u64,
    pub max_records_per_shape: u64,
    pub max_actions: u64,
    pub max_frames: u64,
    pub max_text_glyphs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: 8 << 20,
            max_movie_bytes: 8 << 20,
            max_inflate_padding: 64 << 10,
            max_tags: 100_000,
            max_shape_records: 1_600_000,
            max_records_per_shape: 16_384,
            max_actions: 2_000_000,
            max_frames: 65_536,
            max_text_glyphs: 1_000_000,
        }
    }
}

pub struct Budget {
    limits: Limits,
    tags: Cell<u64>,
    shape_records: Cell<u64>,
    actions: Cell<u64>,
    frames: Cell<u64>,
    text_glyphs: Cell<u64>,
}

impl Budget {
    pub fn new(limits: Limits) -> Self {
        Budget {
            limits,
            tags: Cell::new(0),
            shape_records: Cell::new(0),
            actions: Cell::new(0),
            frames: Cell::new(0),
            text_glyphs: Cell::new(0),
        }
    }

    pub fn unlimited() -> Self {
        Budget::new(Limits {
            max_file_bytes: usize::MAX,
            max_movie_bytes: usize::MAX,
            max_inflate_padding: usize::MAX,
            max_tags: u64::MAX,
            max_shape_records: u64::MAX,
            max_records_per_shape: u64::MAX,
            max_actions: u64::MAX,
            max_frames: u64::MAX,
            max_text_glyphs: u64::MAX,
        })
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn take(counter: &Cell<u64>, n: u64, cap: u64, what: &str) -> Result<()> {
        let total = counter.get().saturating_add(n);
        if total > cap {
            return Err(Error::invalid(format!("more than {cap} {what} (the movie cap)")));
        }
        counter.set(total);
        Ok(())
    }

    pub fn tag(&self) -> Result<()> {
        Self::take(&self.tags, 1, self.limits.max_tags, "tags")
    }

    pub fn shape_records(&self, n: u64) -> Result<()> {
        Self::take(&self.shape_records, n, self.limits.max_shape_records, "shape, morph and glyph records")
    }

    pub fn shape_record(&self, held: &mut u64) -> Result<()> {
        *held += 1;
        if *held > self.limits.max_records_per_shape {
            let cap = self.limits.max_records_per_shape;
            return Err(Error::invalid(format!("a shape of more than {cap} records (the shape cap)")));
        }
        self.shape_records(1)
    }

    pub fn actions(&self, n: u64) -> Result<()> {
        Self::take(&self.actions, n, self.limits.max_actions, "action records and their values")
    }

    pub fn timeline(&self, frame_count: u16) -> Result<()> {
        Self::take(&self.frames, u64::from(frame_count.max(1)), self.limits.max_frames, "timeline frames")
    }

    pub fn text_glyphs(&self, n: u64) -> Result<()> {
        Self::take(&self.text_glyphs, n, self.limits.max_text_glyphs, "static text glyphs")
    }

    pub fn totals(&self) -> [u64; 5] {
        [self.tags.get(), self.shape_records.get(), self.actions.get(), self.frames.get(), self.text_glyphs.get()]
    }
}
