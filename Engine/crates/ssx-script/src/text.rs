use std::fmt;

pub struct BoundedText {
    out: String,
    budget: usize,
    cut_bytes: u64,
    cut_values: u64,
    stopped: bool,
}

impl BoundedText {
    pub fn new(budget: usize) -> Self {
        BoundedText { out: String::new(), budget, cut_bytes: 0, cut_values: 0, stopped: false }
    }

    pub fn is_full(&self) -> bool {
        self.out.len() >= self.budget
    }

    fn room(&self) -> usize {
        self.budget.saturating_sub(self.out.len())
    }

    pub fn push_bytes(&mut self, bytes: &[u8]) {
        let prefix = &bytes[..bytes.len().min(self.room().saturating_add(3))];
        let mut used = 0usize;
        'chunks: for chunk in prefix.utf8_chunks() {
            let valid = chunk.valid();
            let room = self.room();
            if valid.len() > room {
                let end = valid.floor_char_boundary(room);
                self.out.push_str(&valid[..end]);
                used += end;
                break 'chunks;
            }
            self.out.push_str(valid);
            used += valid.len();
            if !chunk.invalid().is_empty() {
                if self.room() < char::REPLACEMENT_CHARACTER.len_utf8() {
                    break 'chunks;
                }
                self.out.push(char::REPLACEMENT_CHARACTER);
                used += chunk.invalid().len();
            }
        }
        self.cut_bytes = self.cut_bytes.saturating_add((bytes.len() - used) as u64);
    }

    pub fn push_str(&mut self, s: &str) {
        self.push_bytes(s.as_bytes());
    }

    pub fn cut(&mut self, n: usize) {
        self.cut_bytes = self.cut_bytes.saturating_add(n as u64);
    }

    pub fn skip_value(&mut self) {
        self.cut_values = self.cut_values.saturating_add(1);
    }

    pub fn finish(mut self) -> String {
        if self.stopped {
            self.out.push_str(&format!("… (at least {} bytes cut)", self.cut_bytes));
            return self.out;
        }
        match (self.cut_bytes, self.cut_values) {
            (0, 0) => {}
            (b, 0) => self.out.push_str(&format!("… ({b} bytes cut)")),
            (0, v) => self.out.push_str(&format!("… ({v} values not shown)")),
            (b, v) => self.out.push_str(&format!("… ({b} bytes cut, {v} values not shown)")),
        }
        self.out
    }
}

impl fmt::Write for BoundedText {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let before = self.cut_bytes;
        self.push_str(s);
        if self.cut_bytes > before {
            self.stopped = true;
            return Err(fmt::Error);
        }
        Ok(())
    }
}

pub fn display_bounded(value: &dyn fmt::Display, budget: usize) -> String {
    let mut t = BoundedText::new(budget);
    let _stopped = fmt::write(&mut t, format_args!("{value}"));
    t.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(budget: usize, parts: &[&[u8]]) -> String {
        let mut t = BoundedText::new(budget);
        for p in parts {
            t.push_bytes(p);
        }
        t.finish()
    }

    #[test]
    fn within_the_budget_nothing_is_cut() {
        assert_eq!(line(10, &[b"abc", b"def"]), "abcdef");
        assert_eq!(line(6, &[b"abc", b"def"]), "abcdef");
    }

    #[test]
    fn cut_at_the_budget_and_counted() {
        assert_eq!(line(4, &[b"abcdef"]), "abcd… (2 bytes cut)");
        assert_eq!(line(4, &[b"ab", b"cdef", b"gh"]), "abcd… (4 bytes cut)");
        assert_eq!(line(0, &[b"x"]), "… (1 bytes cut)");
    }

    #[test]
    fn characters_are_never_split() {
        assert_eq!(line(3, &["aéb".as_bytes()]), "aé… (1 bytes cut)");
        assert_eq!(line(2, &["aéb".as_bytes()]), "a… (3 bytes cut)");
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters_within_the_budget() {
        assert_eq!(line(10, &[b"a\xffb"]), "a\u{fffd}b");
        let s = line(3, &[b"\xff\xfe"]);
        assert_eq!(s, "\u{fffd}… (1 bytes cut)");
        assert!(line(2, &[b"\xff"]).starts_with("… (1 bytes cut)"), "no room for the replacement character");
        let raw: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
        let t = {
            let mut t = BoundedText::new(100);
            t.push_bytes(&raw);
            t
        };
        assert!(t.out.len() <= 100);
    }

    #[test]
    fn only_the_prefix_of_a_long_input_is_looked_at() {
        let big = vec![b'x'; 64 << 20];
        let start = std::time::Instant::now();
        let mut t = BoundedText::new(4096);
        for _ in 0..1000 {
            t.push_bytes(&big);
        }
        let s = t.finish();
        assert!(start.elapsed() < std::time::Duration::from_secs(2), "{:?}", start.elapsed());
        assert!(
            s.starts_with(&"x".repeat(4096)) && s.ends_with(&format!("({} bytes cut)", 1000u64 * (64 << 20) - 4096))
        );
    }

    #[test]
    fn display_stops_at_the_budget() {
        struct Endless;
        impl fmt::Display for Endless {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                loop {
                    f.write_str("0123456789")?;
                }
            }
        }
        let s = display_bounded(&Endless, 25);
        assert_eq!(s, "0123456789012345678901234… (at least 5 bytes cut)");
        assert_eq!(display_bounded(&"short", 25), "short");
        assert_eq!(display_bounded(&"x".repeat(25), 25), "x".repeat(25), "an exact fit is not cut");
    }
}
