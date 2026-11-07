use std::collections::HashMap;

use toml::de::{DeTable, DeValue};

#[derive(Clone, Debug, Default)]
pub struct Spans {
    at: HashMap<String, (usize, usize)>,
}

struct Lines<'a> {
    text: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    fn new(text: &'a str) -> Self {
        let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1)).collect();
        Lines { text, starts }
    }

    fn at(&self, offset: usize) -> (usize, usize) {
        let offset = self.text.floor_char_boundary(offset.min(self.text.len()));
        let line = self.starts.partition_point(|&s| s <= offset).max(1);
        let start = self.starts[line - 1];
        (line, self.text[start..offset].chars().count() + 1)
    }
}

pub fn segment(key: &str) -> String {
    let bare = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare { key.to_string() } else { ssx_core::text::quoted(key).to_string() }
}

pub fn child(parent: &str, key: &str) -> String {
    if parent.is_empty() { segment(key) } else { format!("{parent}.{}", segment(key)) }
}

pub fn item(parent: &str, i: usize) -> String {
    format!("{parent}[{i}]")
}

impl Spans {
    pub fn of(text: &str) -> Spans {
        let mut s = Spans::default();
        let Ok(doc) = DeTable::parse(text) else { return s };
        let lines = Lines::new(text);
        s.table("", doc.get_ref(), &lines, 0);
        s
    }

    fn table(&mut self, path: &str, t: &DeTable<'_>, lines: &Lines<'_>, depth: usize) {
        if depth > crate::limits::MAX_TOML_DEPTH {
            return;
        }
        for (k, v) in t.iter() {
            let p = child(path, k.get_ref());
            self.at.insert(p.clone(), lines.at(k.span().start));
            self.value(&p, v.get_ref(), lines, depth + 1);
        }
    }

    fn value(&mut self, path: &str, v: &DeValue<'_>, lines: &Lines<'_>, depth: usize) {
        match v {
            DeValue::Table(t) => self.table(path, t, lines, depth),
            DeValue::Array(a) => {
                for (i, e) in a.iter().enumerate() {
                    let p = item(path, i);
                    self.at.insert(p.clone(), lines.at(e.span().start));
                    self.value(&p, e.get_ref(), lines, depth + 1);
                }
            }
            _ => {}
        }
    }

    pub fn find(&self, path: &str) -> (usize, usize) {
        let mut p = path;
        loop {
            if let Some(&at) = self.at.get(p) {
                return at;
            }
            match p.rfind(['.', '[']) {
                Some(i) => p = &p[..i],
                None => return (0, 0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_keys_and_items_have_their_lines() {
        let text = "a = 1\n[b]\nc = [1, { d = 2 }]\n\n[[e]]\nf = \"x\"\n[[e]]\nf = \"y\"\n\"odd key\" = 3\n";
        let s = Spans::of(text);
        assert_eq!(s.find("a"), (1, 1));
        assert_eq!(s.find("b.c"), (3, 1));
        assert_eq!(s.find("b.c[1].d"), (3, 11));
        assert_eq!(s.find("e[1].f"), (8, 1));
        assert_eq!(s.find("e[1].\"odd key\""), (9, 1));
        assert_eq!(s.find("b.c[1].d.missing"), (3, 11), "falls back to the nearest parent");
        assert_eq!(s.find("nothing"), (0, 0));
        assert_eq!(Spans::of("a = [").find("a"), (0, 0), "an unparsable text has no spans");
    }
}
