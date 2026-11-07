use super::LayoutError;
use super::json::{Key, Node, Pos, Value};

#[derive(Clone, Copy)]
pub(super) struct Cx<'f> {
    pub file: &'f str,
}

impl Cx<'_> {
    pub fn err(&self, pos: Pos, message: impl Into<String>) -> LayoutError {
        LayoutError { file: self.file.to_string(), pos, message: message.into() }
    }

    pub fn obj<'n>(&self, node: &'n Node, what: &str) -> Result<Obj<'n>, LayoutError> {
        match &node.value {
            Value::Object(m) => {
                Ok(Obj { pos: node.pos, members: m, used: vec![false; m.len()], what: what.to_string() })
            }
            _ => Err(self.err(node.pos, format!("{what}: expected an object, found {}", node.kind()))),
        }
    }

    pub fn array<'n>(&self, node: &'n Node, what: &str, max: usize) -> Result<&'n [Node], LayoutError> {
        match &node.value {
            Value::Array(a) if a.len() <= max => Ok(a),
            Value::Array(a) => Err(self.err(node.pos, format!("{what}: {} elements, at most {max}", a.len()))),
            _ => Err(self.err(node.pos, format!("{what}: expected an array, found {}", node.kind()))),
        }
    }

    pub fn str<'n>(&self, node: &'n Node, what: &str, max_chars: usize) -> Result<&'n str, LayoutError> {
        match &node.value {
            Value::String(s) if s.chars().count() <= max_chars => Ok(s),
            Value::String(_) => Err(self.err(node.pos, format!("{what}: longer than {max_chars} characters"))),
            _ => Err(self.err(node.pos, format!("{what}: expected a string, found {}", node.kind()))),
        }
    }

    pub fn bool(&self, node: &Node, what: &str) -> Result<bool, LayoutError> {
        match node.value {
            Value::Bool(b) => Ok(b),
            _ => Err(self.err(node.pos, format!("{what}: expected true or false, found {}", node.kind()))),
        }
    }

    pub fn num(&self, node: &Node, what: &str, min: f64, max: f64) -> Result<f64, LayoutError> {
        match node.value {
            Value::Number(v) if (min..=max).contains(&v) => Ok(v),
            Value::Number(v) => Err(self.err(node.pos, format!("{what}: {v} is outside {min} … {max}"))),
            _ => Err(self.err(node.pos, format!("{what}: expected a number, found {}", node.kind()))),
        }
    }

    pub fn int(&self, node: &Node, what: &str, min: i64, max: i64) -> Result<i64, LayoutError> {
        let v = self.num(node, what, min as f64, max as f64)?;
        if v.fract() != 0.0 {
            return Err(self.err(node.pos, format!("{what}: {v} is not a whole number")));
        }
        Ok(v as i64)
    }

    pub fn name<'n>(&self, node: &'n Node, what: &str, prefix: &str, max: usize) -> Result<&'n str, LayoutError> {
        let s = self.str(node, what, max)?;
        if !is_name(s) || !s.starts_with(prefix) || s.len() == prefix.len() && !prefix.is_empty() {
            let need = if prefix.is_empty() { String::new() } else { format!(" starting with `{prefix}`") };
            return Err(self.err(node.pos, format!("{what}: {s:?} is not a name (letters, digits, `_`{need})")));
        }
        Ok(s)
    }

    pub fn choice<'c>(&self, node: &Node, what: &str, choices: &[&'c str]) -> Result<&'c str, LayoutError> {
        let s = self.str(node, what, 64)?;
        choices
            .iter()
            .find(|c| **c == s)
            .copied()
            .ok_or_else(|| self.err(node.pos, format!("{what}: {s:?} is not one of {}", choices.join(", "))))
    }

    pub fn color(&self, node: &Node, what: &str) -> Result<[u8; 4], LayoutError> {
        let s = self.str(node, what, 9)?;
        let hex =
            s.strip_prefix('#').filter(|h| (h.len() == 6 || h.len() == 8) && h.bytes().all(|b| b.is_ascii_hexdigit()));
        let Some(hex) = hex else {
            return Err(self.err(node.pos, format!("{what}: {s:?} is not #rrggbb or #rrggbbaa")));
        };
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
        Ok([byte(0), byte(2), byte(4), if hex.len() == 8 { byte(6) } else { 0xFF }])
    }
}

pub fn is_name(s: &str) -> bool {
    let mut b = s.bytes();
    b.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_') && b.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

pub(super) struct Obj<'n> {
    pub pos: Pos,
    members: &'n [(Key, Node)],
    used: Vec<bool>,
    what: String,
}

impl<'n> Obj<'n> {
    pub fn opt(&mut self, key: &str) -> Option<&'n Node> {
        let i = self.members.iter().position(|(k, _)| k.name == key)?;
        self.used[i] = true;
        Some(&self.members[i].1)
    }

    pub fn req(&mut self, cx: &Cx, key: &str) -> Result<&'n Node, LayoutError> {
        let what = self.what.clone();
        let pos = self.pos;
        self.opt(key).ok_or_else(|| cx.err(pos, format!("{what}: `{key}` is missing")))
    }

    pub fn finish(self, cx: &Cx) -> Result<(), LayoutError> {
        match self.members.iter().zip(&self.used).find(|(_, u)| !**u) {
            Some(((k, _), _)) => Err(cx.err(k.pos, format!("{}: unknown member `{}`", self.what, k.name))),
            None => Ok(()),
        }
    }
}
