use crate::number::str2d;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Eof,
    Name(Vec<u8>),
    Str(Vec<u8>),
    Num(f64),
    And,
    Break,
    Do,
    Else,
    Elseif,
    End,
    False,
    For,
    Function,
    If,
    In,
    Local,
    Nil,
    Not,
    Or,
    Repeat,
    Return,
    Then,
    True,
    Until,
    While,
    Concat,
    Dots,
    Eq,
    Ge,
    Le,
    Ne,
    Char(u8),
}

const RESERVED: [(&str, Tok); 21] = [
    ("and", Tok::And),
    ("break", Tok::Break),
    ("do", Tok::Do),
    ("else", Tok::Else),
    ("elseif", Tok::Elseif),
    ("end", Tok::End),
    ("false", Tok::False),
    ("for", Tok::For),
    ("function", Tok::Function),
    ("if", Tok::If),
    ("in", Tok::In),
    ("local", Tok::Local),
    ("nil", Tok::Nil),
    ("not", Tok::Not),
    ("or", Tok::Or),
    ("repeat", Tok::Repeat),
    ("return", Tok::Return),
    ("then", Tok::Then),
    ("true", Tok::True),
    ("until", Tok::Until),
    ("while", Tok::While),
];

impl Tok {
    pub fn describe(&self) -> String {
        match self {
            Tok::Eof => "<eof>".into(),
            Tok::Name(_) => "<name>".into(),
            Tok::Str(_) => "<string>".into(),
            Tok::Num(_) => "<number>".into(),
            Tok::Concat => "..".into(),
            Tok::Dots => "...".into(),
            Tok::Eq => "==".into(),
            Tok::Ge => ">=".into(),
            Tok::Le => "<=".into(),
            Tok::Ne => "~=".into(),
            Tok::Char(c) if c.is_ascii_control() => format!("char({c})"),
            Tok::Char(c) => (*c as char).to_string(),
            t => RESERVED.iter().find(|(_, r)| r == t).map(|(s, _)| s.to_string()).unwrap_or_default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub raw: Vec<u8>,
}

impl Token {
    pub fn near(&self) -> String {
        match self.tok {
            Tok::Name(_) | Tok::Str(_) | Tok::Num(_) => String::from_utf8_lossy(&self.raw).into_owned(),
            _ => self.tok.describe(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LexError {
    pub msg: String,
    pub line: u32,
    pub near: Option<String>,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    pub line: u32,
    buf: Vec<u8>,
}

pub fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

fn is_newline(c: Option<u8>) -> bool {
    matches!(c, Some(b'\n') | Some(b'\r'))
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        Lexer { src, pos: 0, line: 1, buf: Vec::new() }
    }

    fn cur(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn next(&mut self) {
        self.pos += 1;
    }

    fn save_next(&mut self) {
        if let Some(c) = self.cur() {
            self.buf.push(c);
        }
        self.next();
    }

    fn err(&self, msg: &str, near: Option<String>) -> LexError {
        LexError { msg: msg.into(), line: self.line, near }
    }

    fn near_buf(&self) -> Option<String> {
        Some(String::from_utf8_lossy(&self.buf).into_owned())
    }

    fn inc_line(&mut self) -> Result<(), LexError> {
        let old = self.cur();
        self.next();
        if is_newline(self.cur()) && self.cur() != old {
            self.next();
        }
        self.line += 1;
        if self.line >= i32::MAX as u32 - 2 {
            return Err(self.err("chunk has too many lines", None));
        }
        Ok(())
    }

    fn skip_sep(&mut self) -> i32 {
        let s = self.cur();
        self.save_next();
        let mut count = 0;
        while self.cur() == Some(b'=') {
            self.save_next();
            count += 1;
        }
        if self.cur() == s { count } else { -count - 1 }
    }

    fn read_long_string(&mut self, sep: i32, keep: bool) -> Result<Vec<u8>, LexError> {
        self.save_next();
        if is_newline(self.cur()) {
            self.inc_line()?;
        }
        loop {
            match self.cur() {
                None => {
                    let msg = if keep { "unfinished long string" } else { "unfinished long comment" };
                    return Err(self.err(msg, Some("<eof>".into())));
                }
                Some(b'[') => {
                    if self.skip_sep() == sep {
                        self.save_next();
                        if sep == 0 {
                            return Err(self.err("nesting of [[...]] is deprecated", Some("[".into())));
                        }
                    }
                }
                Some(b']') => {
                    if self.skip_sep() == sep {
                        self.save_next();
                        break;
                    }
                }
                Some(b'\n') | Some(b'\r') => {
                    self.buf.push(b'\n');
                    self.inc_line()?;
                    if !keep {
                        self.buf.clear();
                    }
                }
                Some(_) => {
                    if keep {
                        self.save_next();
                    } else {
                        self.next();
                    }
                }
            }
        }
        let n = self.buf.len();
        let s = (sep + 2) as usize;
        Ok(if keep && n >= 2 * s { self.buf[s..n - s].to_vec() } else { Vec::new() })
    }
}

impl Lexer<'_> {
    fn read_numeral(&mut self) -> Result<Token, LexError> {
        let line = self.line;
        loop {
            self.save_next();
            match self.cur() {
                Some(c) if c.is_ascii_digit() || c == b'.' => {}
                _ => break,
            }
        }
        if matches!(self.cur(), Some(b'e') | Some(b'E')) {
            self.save_next();
            if matches!(self.cur(), Some(b'+') | Some(b'-')) {
                self.save_next();
            }
        }
        while matches!(self.cur(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
            self.save_next();
        }
        match str2d(&self.buf) {
            Some(n) => Ok(Token { tok: Tok::Num(n), line, raw: self.buf.clone() }),
            None => Err(self.err("malformed number", self.near_buf())),
        }
    }

    fn read_string(&mut self, del: u8) -> Result<Token, LexError> {
        let line = self.line;
        self.save_next();
        let mut val = Vec::new();
        while self.cur() != Some(del) {
            match self.cur() {
                None => return Err(self.err("unfinished string", Some("<eof>".into()))),
                Some(b'\n') | Some(b'\r') => return Err(self.err("unfinished string", self.near_buf())),
                Some(b'\\') => {
                    self.next();
                    let c = match self.cur() {
                        Some(b'a') => 7,
                        Some(b'b') => 8,
                        Some(b'f') => 12,
                        Some(b'n') => b'\n',
                        Some(b'r') => b'\r',
                        Some(b't') => b'\t',
                        Some(b'v') => 11,
                        Some(b'\n') | Some(b'\r') => {
                            self.buf.push(b'\n');
                            val.push(b'\n');
                            self.inc_line()?;
                            continue;
                        }
                        None => continue,
                        Some(d) if d.is_ascii_digit() => {
                            let mut v: u32 = 0;
                            let mut i = 0;
                            while i < 3 && matches!(self.cur(), Some(d) if d.is_ascii_digit()) {
                                v = 10 * v + u32::from(self.cur().unwrap_or(b'0') - b'0');
                                self.next();
                                i += 1;
                            }
                            if v > 255 {
                                return Err(self.err("escape sequence too large", self.near_buf()));
                            }
                            self.buf.push(v as u8);
                            val.push(v as u8);
                            continue;
                        }
                        Some(other) => other,
                    };
                    self.buf.push(c);
                    val.push(c);
                    self.next();
                }
                Some(c) => {
                    val.push(c);
                    self.save_next();
                }
            }
        }
        self.save_next();
        Ok(Token { tok: Tok::Str(val), line, raw: self.buf.clone() })
    }

    pub fn token(&mut self) -> Result<Token, LexError> {
        self.buf.clear();
        loop {
            let line = self.line;
            let t = |tok| Ok(Token { tok, line, raw: Vec::new() });
            let Some(c) = self.cur() else { return t(Tok::Eof) };
            match c {
                b'\n' | b'\r' => self.inc_line()?,
                b'-' => {
                    self.next();
                    if self.cur() != Some(b'-') {
                        return t(Tok::Char(b'-'));
                    }
                    self.next();
                    if self.cur() == Some(b'[') {
                        let sep = self.skip_sep();
                        self.buf.clear();
                        if sep >= 0 {
                            self.read_long_string(sep, false)?;
                            self.buf.clear();
                            continue;
                        }
                    }
                    while !is_newline(self.cur()) && self.cur().is_some() {
                        self.next();
                    }
                }
                b'[' => {
                    let sep = self.skip_sep();
                    if sep >= 0 {
                        let s = self.read_long_string(sep, true)?;
                        return Ok(Token { tok: Tok::Str(s), line, raw: self.buf.clone() });
                    } else if sep == -1 {
                        return t(Tok::Char(b'['));
                    }
                    return Err(self.err("invalid long string delimiter", self.near_buf()));
                }
                b'=' | b'<' | b'>' | b'~' => {
                    self.next();
                    if self.cur() != Some(b'=') {
                        return t(Tok::Char(c));
                    }
                    self.next();
                    return t(match c {
                        b'=' => Tok::Eq,
                        b'<' => Tok::Le,
                        b'>' => Tok::Ge,
                        _ => Tok::Ne,
                    });
                }
                b'"' | b'\'' => return self.read_string(c),
                b'.' => {
                    self.save_next();
                    if self.cur() == Some(b'.') {
                        self.next();
                        if self.cur() == Some(b'.') {
                            self.next();
                            return t(Tok::Dots);
                        }
                        return t(Tok::Concat);
                    }
                    if !matches!(self.cur(), Some(d) if d.is_ascii_digit()) {
                        return t(Tok::Char(b'.'));
                    }
                    self.pos -= 1;
                    self.buf.clear();
                    return self.read_numeral();
                }
                c if is_space(c) => self.next(),
                c if c.is_ascii_digit() => return self.read_numeral(),
                c if c.is_ascii_alphabetic() || c == b'_' => {
                    while matches!(self.cur(), Some(c) if c.is_ascii_alphanumeric() || c == b'_') {
                        self.save_next();
                    }
                    let name = self.buf.clone();
                    for (s, r) in RESERVED.iter() {
                        if s.as_bytes() == name.as_slice() {
                            return Ok(Token { tok: r.clone(), line, raw: name });
                        }
                    }
                    return Ok(Token { tok: Tok::Name(name.clone()), line, raw: name });
                }
                c => {
                    self.next();
                    return t(Tok::Char(c));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        let mut l = Lexer::new(s.as_bytes());
        let mut out = Vec::new();
        loop {
            let t = l.token().expect("lex");
            if t.tok == Tok::Eof {
                return out;
            }
            out.push(t.tok);
        }
    }

    #[test]
    fn numbers_strings_comments() {
        assert_eq!(toks("0x1F 3.5e2 .5"), vec![Tok::Num(31.0), Tok::Num(350.0), Tok::Num(0.5)]);
        assert_eq!(toks("'a\\65\\n' [==[x]]y]==]"), vec![Tok::Str(b"aA\n".to_vec()), Tok::Str(b"x]]y".to_vec())]);
        assert_eq!(toks("a -- c\n--[[ long\n ]] b"), vec![Tok::Name(b"a".to_vec()), Tok::Name(b"b".to_vec())]);
        assert_eq!(toks("-- [[StateSet]] x"), vec![]);
        assert_eq!(toks("a.b..c...~="), {
            let n = |s: &str| Tok::Name(s.as_bytes().to_vec());
            vec![n("a"), Tok::Char(b'.'), n("b"), Tok::Concat, n("c"), Tok::Dots, Tok::Ne]
        });
    }

    #[test]
    fn errors() {
        let e = Lexer::new(b"[[a [[b]] ]]").token().unwrap_err();
        assert_eq!(e.msg, "nesting of [[...]] is deprecated");
        assert_eq!(Lexer::new(b"3..2").token().unwrap_err().msg, "malformed number");
        assert_eq!(Lexer::new(b"'abc\n'").token().unwrap_err().msg, "unfinished string");
    }
}
