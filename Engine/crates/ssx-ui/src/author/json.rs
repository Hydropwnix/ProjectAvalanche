use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Node>),
    Object(Vec<(Key, Node)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub name: String,
    pub pos: Pos,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub pos: Pos,
    pub value: Value,
}

impl Node {
    pub fn kind(&self) -> &'static str {
        match self.value {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Number(_) => "a number",
            Value::String(_) => "a string",
            Value::Array(_) => "an array",
            Value::Object(_) => "an object",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_depth: usize,
    pub max_values: usize,
    pub max_string: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub pos: Pos,
    pub message: String,
}

pub fn parse(text: &str, limits: &Limits) -> Result<Node, SyntaxError> {
    if text.len() > limits.max_bytes {
        return Err(SyntaxError {
            pos: Pos { line: 1, col: 1 },
            message: format!("{} bytes, more than the limit of {}", text.len(), limits.max_bytes),
        });
    }
    let mut p = Parser { chars: text.chars().collect(), at: 0, line: 1, col: 1, values: 0, limits };
    p.skip_ws();
    let node = p.value(0)?;
    p.skip_ws();
    if p.at < p.chars.len() {
        return Err(p.err("text after the document's value"));
    }
    Ok(node)
}

struct Parser<'l> {
    chars: Vec<char>,
    at: usize,
    line: u32,
    col: u32,
    values: usize,
    limits: &'l Limits,
}

impl Parser<'_> {
    fn pos(&self) -> Pos {
        Pos { line: self.line, col: self.col }
    }

    fn err(&self, message: impl Into<String>) -> SyntaxError {
        SyntaxError { pos: self.pos(), message: message.into() }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        if c == '\n' {
            self.line = self.line.saturating_add(1);
            self.col = 1;
        } else {
            self.col = self.col.saturating_add(1);
        }
        Some(c)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.bump();
        }
    }

    fn expect(&mut self, c: char) -> Result<(), SyntaxError> {
        match self.peek() {
            Some(x) if x == c => {
                self.bump();
                Ok(())
            }
            Some(x) => Err(self.err(format!("expected {c:?}, found {x:?}"))),
            None => Err(self.err(format!("expected {c:?}, found the end"))),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Node, SyntaxError> {
        if depth >= self.limits.max_depth {
            return Err(self.err(format!("nested deeper than {} levels", self.limits.max_depth)));
        }
        self.values += 1;
        if self.values > self.limits.max_values {
            return Err(self.err(format!("more than {} values", self.limits.max_values)));
        }
        let pos = self.pos();
        let value = match self.peek() {
            Some('{') => self.object(depth)?,
            Some('[') => self.array(depth)?,
            Some('"') => Value::String(self.string()?),
            Some('t') => self.word("true", Value::Bool(true))?,
            Some('f') => self.word("false", Value::Bool(false))?,
            Some('n') => self.word("null", Value::Null)?,
            Some(c) if c == '-' || c.is_ascii_digit() => Value::Number(self.number()?),
            Some(c) => return Err(self.err(format!("unexpected {c:?}"))),
            None => return Err(self.err("unexpected end")),
        };
        Ok(Node { pos, value })
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, SyntaxError> {
        for c in w.chars() {
            if self.peek() != Some(c) {
                return Err(self.err(format!("expected `{w}`")));
            }
            self.bump();
        }
        Ok(v)
    }

    fn object(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        self.expect('{')?;
        let mut members: Vec<(Key, Node)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.bump();
            return Ok(Value::Object(members));
        }
        loop {
            self.skip_ws();
            let pos = self.pos();
            if self.peek() != Some('"') {
                return Err(self.err("expected a member name in quotes"));
            }
            let name = self.string()?;
            if members.iter().any(|(k, _)| k.name == name) {
                return Err(SyntaxError { pos, message: format!("member {name:?} given twice") });
            }
            self.skip_ws();
            self.expect(':')?;
            self.skip_ws();
            let v = self.value(depth + 1)?;
            members.push((Key { name, pos }, v));
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some('}') => return Ok(Value::Object(members)),
                _ => return Err(self.err("expected `,` or `}` after a member")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, SyntaxError> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.bump();
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.bump() {
                Some(',') => continue,
                Some(']') => return Ok(Value::Array(items)),
                _ => return Err(self.err("expected `,` or `]` after an element")),
            }
        }
    }

    fn string(&mut self) -> Result<String, SyntaxError> {
        self.expect('"')?;
        let mut s = String::new();
        let mut n = 0usize;
        loop {
            let c = self.bump().ok_or_else(|| self.err("unterminated string"))?;
            n += 1;
            if n > self.limits.max_string {
                return Err(self.err(format!("a string longer than {} characters", self.limits.max_string)));
            }
            match c {
                '"' => return Ok(s),
                '\\' => s.push(self.escape()?),
                c if (c as u32) < 0x20 => return Err(self.err("a control character in a string")),
                c => s.push(c),
            }
        }
    }

    fn escape(&mut self) -> Result<char, SyntaxError> {
        let c = self.bump().ok_or_else(|| self.err("unterminated escape"))?;
        Ok(match c {
            '"' => '"',
            '\\' => '\\',
            '/' => '/',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'u' => {
                let hi = self.hex4()?;
                if (0xD800..0xDC00).contains(&hi) {
                    if self.bump() != Some('\\') || self.bump() != Some('u') {
                        return Err(self.err("a high surrogate without its low surrogate"));
                    }
                    let lo = self.hex4()?;
                    if !(0xDC00..0xE000).contains(&lo) {
                        return Err(self.err("a high surrogate without its low surrogate"));
                    }
                    let code = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                    char::from_u32(code).ok_or_else(|| self.err("an invalid code point"))?
                } else {
                    char::from_u32(hi).ok_or_else(|| self.err("a lone low surrogate"))?
                }
            }
            c => return Err(self.err(format!("unknown escape \\{c}"))),
        })
    }

    fn hex4(&mut self) -> Result<u32, SyntaxError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let c = self.bump().ok_or_else(|| self.err("unterminated \\u escape"))?;
            v = v * 16 + c.to_digit(16).ok_or_else(|| self.err(format!("{c:?} in a \\u escape")))?;
        }
        Ok(v)
    }

    fn number(&mut self) -> Result<f64, SyntaxError> {
        let start = self.at;
        if self.peek() == Some('-') {
            self.bump();
        }
        match self.peek() {
            Some('0') => {
                self.bump();
            }
            Some(c) if c.is_ascii_digit() => {
                while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                }
            }
            _ => return Err(self.err("a number without digits")),
        }
        if self.peek() == Some('.') {
            self.bump();
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.err("no digits after the decimal point"));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        if matches!(self.peek(), Some('e' | 'E')) {
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            if !self.peek().is_some_and(|c| c.is_ascii_digit()) {
                return Err(self.err("no digits in the exponent"));
            }
            while self.peek().is_some_and(|c| c.is_ascii_digit()) {
                self.bump();
            }
        }
        let text: String = self.chars[start..self.at].iter().collect();
        if text.len() > 64 {
            return Err(self.err("a number literal longer than 64 characters"));
        }
        let v: f64 = text.parse().map_err(|_| self.err(format!("number {text}")))?;
        if !v.is_finite() {
            return Err(self.err(format!("number {text} is out of range")));
        }
        Ok(v)
    }
}
