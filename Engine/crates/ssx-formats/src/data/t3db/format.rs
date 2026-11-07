#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Placeholder {
    Arg { at: usize, len: usize, number: u8, spec: Option<String>, conversion: char },
    Text { at: usize },
}

pub fn conversion(b: u8) -> Option<char> {
    match b {
        b'd' | b'D' => Some('d'),
        b'u' | b'U' => Some('u'),
        b'f' | b'F' => Some('f'),
        b's' | b'S' => Some('s'),
        b'c' | b'C' => Some('c'),
        _ => None,
    }
}

pub fn placeholders(text: &str) -> Vec<Placeholder> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for at in (0..b.len()).filter(|&i| b[i] == b'%') {
        let number = b.get(at + 1).map_or(0, |d| d.wrapping_sub(b'0'));
        let arg = (1..=9).contains(&number).then(|| match b.get(at + 2) {
            Some(b'[') => {
                let close = b[at + 3..].iter().position(|&c| c == b']').map(|k| at + 3 + k)?;
                let conversion = conversion(*b.get(close + 1)?)?;
                let spec = text.get(at + 3..close)?.to_string();
                let len = if spec.is_empty() { 3 } else { spec.len() + 5 };
                Some(Placeholder::Arg { at, len, number, spec: Some(spec), conversion })
            }
            Some(&c) => conversion(c).map(|conversion| Placeholder::Arg { at, len: 3, number, spec: None, conversion }),
            None => None,
        });
        out.push(arg.flatten().unwrap_or(Placeholder::Text { at }));
    }
    out
}
