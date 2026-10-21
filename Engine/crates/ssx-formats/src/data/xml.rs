use crate::{Error, Result};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Element {
    pub name: String,
    pub attributes: Vec<(String, String)>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

pub fn parse(text: &str) -> Result<Element> {
    let mut p = Parser { s: text.as_bytes(), i: 0 };
    p.skip_misc()?;
    if p.s[p.i..].starts_with(b"<?xml") {
        let end = p.find(b"?>")?;
        p.i = end + 2;
    }
    p.skip_misc()?;
    let root = p.element()?;
    p.skip_misc()?;
    if p.i != p.s.len() {
        return Err(p.err("content after the root element"));
    }
    Ok(root)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> Error {
        Error::invalid(format!("xml: {what} at byte {}", self.i))
    }

    fn find(&self, pat: &[u8]) -> Result<usize> {
        self.s[self.i..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| self.i + p)
            .ok_or_else(|| self.err("unterminated construct"))
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn skip_misc(&mut self) -> Result<()> {
        loop {
            self.skip_ws();
            if self.s[self.i..].starts_with(b"<!--") {
                self.i = self.find(b"-->")? + 3;
            } else {
                return Ok(());
            }
        }
    }

    fn name(&mut self) -> Result<String> {
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            let ok = c.is_ascii_alphanumeric() || matches!(c, b'_' | b':' | b'-' | b'.');
            if !ok || (self.i == start && (c.is_ascii_digit() || c == b'-' || c == b'.')) {
                break;
            }
            self.i += 1;
        }
        if self.i == start {
            return Err(self.err("expected a name"));
        }
        Ok(String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        if self.s.get(self.i) != Some(&c) {
            return Err(self.err(&format!("expected '{}'", c as char)));
        }
        self.i += 1;
        Ok(())
    }

    fn element(&mut self) -> Result<Element> {
        self.expect(b'<')?;
        let name = self.name()?;
        let mut el = Element { name, ..Default::default() };
        loop {
            let had_ws = self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace());
            self.skip_ws();
            match self.s.get(self.i) {
                Some(b'/') => {
                    self.i += 1;
                    self.expect(b'>')?;
                    return Ok(el);
                }
                Some(b'>') => {
                    self.i += 1;
                    break;
                }
                Some(_) if had_ws => {
                    let key = self.name()?;
                    self.skip_ws();
                    self.expect(b'=')?;
                    self.skip_ws();
                    let value = self.quoted()?;
                    if el.attr(&key).is_some() {
                        return Err(self.err(&format!("duplicate attribute {key}")));
                    }
                    el.attributes.push((key, value));
                }
                _ => return Err(self.err("malformed start tag")),
            }
        }
        loop {
            self.skip_misc()?;
            if self.s[self.i..].starts_with(b"</") {
                self.i += 2;
                let end = self.name()?;
                if end != el.name {
                    return Err(self.err(&format!("end tag {end} closes {}", el.name)));
                }
                self.skip_ws();
                self.expect(b'>')?;
                return Ok(el);
            }
            match self.s.get(self.i) {
                Some(b'<') if !self.s[self.i..].starts_with(b"<!") && !self.s[self.i..].starts_with(b"<?") => {
                    let child = self.element()?;
                    el.children.push(child);
                }
                None => return Err(self.err(&format!("unclosed element {}", el.name))),
                _ => return Err(self.err("unsupported character data or markup")),
            }
        }
    }

    fn quoted(&mut self) -> Result<String> {
        let q = match self.s.get(self.i) {
            Some(&q @ (b'"' | b'\'')) => q,
            _ => return Err(self.err("expected a quoted attribute value")),
        };
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != q {
            if self.s[self.i] == b'<' {
                return Err(self.err("'<' in an attribute value"));
            }
            self.i += 1;
        }
        if self.i == self.s.len() {
            return Err(self.err("unterminated attribute value"));
        }
        let raw = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| self.err("attribute value is not UTF-8"))?;
        self.i += 1;
        unescape(raw).ok_or_else(|| self.err("bad entity reference"))
    }
}

fn unescape(raw: &str) -> Option<String> {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        let tail = &rest[p + 1..];
        let end = tail.find(';')?;
        let ent = &tail[..end];
        let ch = match ent {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ if ent.starts_with("#x") => char::from_u32(u32::from_str_radix(&ent[2..], 16).ok()?)?,
            _ if ent.starts_with('#') => char::from_u32(ent[1..].parse().ok()?)?,
            _ => return None,
        };
        out.push(ch);
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_elements_attributes_and_comments() {
        let doc = "<?xml version='1.0' encoding='UTF-8'?>\r\n<!-- c --><a x=\"1 &amp;&amp; 2\" y='&lt;&#65;'>\n<b/><!-- <c/> --><c z=\"\" ></c></a>\n";
        let a = parse(doc).unwrap();
        assert_eq!(a.name, "a");
        assert_eq!(a.attr("x"), Some("1 && 2"));
        assert_eq!(a.attr("y"), Some("<A"));
        assert_eq!(a.children.len(), 2);
        assert_eq!(a.children[1].attr("z"), Some(""));
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "<a><b></a>",
            "<a>text</a>",
            "<a x=1/>",
            "<a x=\"1\" x=\"2\"/>",
            "<a/><b/>",
            "<a",
            "<a x=\"&bogus;\"/>",
            "<ax=\"1\"/>",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
