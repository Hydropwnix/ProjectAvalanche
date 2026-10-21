use std::collections::HashMap;

use super::fnv1;

const MIN_LEN: usize = 2;
const SEPARATORS: &[u8] = b".\\/ ,;:='\"()[]{}<>";

#[derive(Clone, Debug, Default)]
pub struct NameDict {
    names: HashMap<u32, String>,
}

impl NameDict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn insert(&mut self, name: &str) -> u32 {
        let h = fnv1(name.as_bytes());
        self.names.entry(h).or_insert_with(|| name.to_owned());
        h
    }

    pub fn get(&self, hash: u32) -> Option<&str> {
        self.names.get(&hash).map(String::as_str)
    }

    pub fn show(&self, hash: u32) -> String {
        self.get(hash).map_or_else(|| format!("{hash:08x}"), str::to_owned)
    }

    pub fn add_strings_from(&mut self, data: &[u8]) {
        for run in data.split(|b| !(0x20..0x7f).contains(b)).filter(|r| r.len() >= MIN_LEN) {
            self.add_candidate(run);
            for token in run.split(|b| SEPARATORS.contains(b)).filter(|t| t.len() >= MIN_LEN && t.len() < run.len()) {
                self.add_candidate(token);
            }
            if let Some(i) = run.windows(13).position(|w| w == b"ClassRefSpec_") {
                self.add_candidate(&run[i + 13..]);
            }
        }
    }

    fn add_candidate(&mut self, bytes: &[u8]) {
        if bytes.len() >= MIN_LEN {
            self.insert(std::str::from_utf8(bytes).expect("printable ASCII"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_names_and_tokens() {
        let mut d = NameDict::new();
        d.add_strings_from(b"\x00\x01terrain_rock_alps_mat.Material\x00Attrib::Gen::ClassRefSpec_ge_texture\x00x\x00");
        assert_eq!(d.get(0xBB14_C3B4), Some("terrain_rock_alps_mat"));
        assert_eq!(d.get(0x6D04_D9A2), Some("Material"));
        assert_eq!(d.get(fnv1(b"ge_texture")), Some("ge_texture"));
        assert_eq!(d.get(fnv1(b"x")), None);
        assert_eq!(d.show(0x1234_5678), "12345678");
    }
}
