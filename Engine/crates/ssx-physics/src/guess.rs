use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub struct Site {
    name: &'static str,
    hits: AtomicU64,
    listed: AtomicBool,
}

static SITES: Mutex<Vec<&'static Site>> = Mutex::new(Vec::new());

impl Site {
    pub const fn new(name: &'static str) -> Self {
        Self { name, hits: AtomicU64::new(0), listed: AtomicBool::new(false) }
    }

    pub fn hit(&'static self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
        if !self.listed.load(Ordering::Relaxed) && !self.listed.swap(true, Ordering::AcqRel) {
            SITES.lock().unwrap_or_else(|e| e.into_inner()).push(self);
        }
    }
}

pub fn hits() -> Vec<(&'static str, u64)> {
    let mut v: Vec<(&'static str, u64)> = SITES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|s| (s.name, s.hits.load(Ordering::Relaxed)))
        .collect();
    v.sort_unstable();
    v
}

#[macro_export]
macro_rules! guess_hit {
    () => {{
        static SITE: $crate::guess::Site = $crate::guess::Site::new(concat!(file!(), ":", line!()));
        SITE.hit();
    }};
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_site_counts_its_runs() {
        for _ in 0..3 {
            crate::guess_hit!();
        }
        let hits = super::hits();
        let mine: Vec<_> = hits.iter().filter(|(n, _)| n.contains("guess.rs:")).collect();
        assert_eq!(mine.len(), 1, "{hits:?}");
        assert_eq!(mine[0].1, 3);
    }
}
