pub const MAXCAPTURES: usize = 32;
const CAP_UNFINISHED: isize = -1;
const CAP_POSITION: isize = -2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Capture {
    Str(usize, usize),
    Pos(usize),
}

pub struct MatchState<'a> {
    pub src: &'a [u8],
    pub pat: &'a [u8],
    pub level: usize,
    cap: [(usize, isize); MAXCAPTURES],
}

pub type PResult<T> = Result<T, String>;

fn class_match(c: u8, cl: u8) -> bool {
    let res = match cl.to_ascii_lowercase() {
        b'a' => c.is_ascii_alphabetic(),
        b'c' => c.is_ascii_control(),
        b'd' => c.is_ascii_digit(),
        b'l' => c.is_ascii_lowercase(),
        b'p' => c.is_ascii_punctuation(),
        b's' => crate::lexer::is_space(c),
        b'u' => c.is_ascii_uppercase(),
        b'w' => c.is_ascii_alphanumeric(),
        b'x' => c.is_ascii_hexdigit(),
        b'z' => c == 0,
        _ => return cl == c,
    };
    if cl.is_ascii_uppercase() { !res } else { res }
}

impl<'a> MatchState<'a> {
    pub fn new(src: &'a [u8], pat: &'a [u8]) -> Self {
        MatchState { src, pat, level: 0, cap: [(0, 0); MAXCAPTURES] }
    }

    fn pat_at(&self, p: usize) -> u8 {
        self.pat.get(p).copied().unwrap_or(0)
    }

    fn class_end(&self, p: usize) -> PResult<usize> {
        let mut p = p;
        let c = self.pat_at(p);
        p += 1;
        if c == b'%' {
            if p >= self.pat.len() {
                return Err("malformed pattern (ends with '%')".into());
            }
            return Ok(p + 1);
        }
        if c == b'[' {
            if self.pat_at(p) == b'^' {
                p += 1;
            }
            loop {
                if p >= self.pat.len() {
                    return Err("malformed pattern (missing ']')".into());
                }
                let cc = self.pat[p];
                p += 1;
                if cc == b'%' && p < self.pat.len() {
                    p += 1;
                }
                if self.pat_at(p) == b']' {
                    break;
                }
            }
            return Ok(p + 1);
        }
        Ok(p)
    }

    fn bracket(&self, c: u8, p: usize, ec: usize) -> bool {
        let mut p = p + 1;
        let mut sig = true;
        if self.pat_at(p) == b'^' {
            sig = false;
            p += 1;
        }
        while p < ec {
            if self.pat[p] == b'%' {
                p += 1;
                if class_match(c, self.pat_at(p)) {
                    return sig;
                }
            } else if self.pat_at(p + 1) == b'-' && p + 2 < ec {
                if self.pat[p] <= c && c <= self.pat[p + 2] {
                    return sig;
                }
                p += 2;
            } else if self.pat[p] == c {
                return sig;
            }
            p += 1;
        }
        !sig
    }

    fn single(&self, s: usize, p: usize, ep: usize) -> bool {
        let Some(&c) = self.src.get(s) else { return false };
        match self.pat_at(p) {
            b'.' => true,
            b'%' => class_match(c, self.pat_at(p + 1)),
            b'[' => self.bracket(c, p, ep - 1),
            pc => pc == c,
        }
    }

    fn match_balance(&self, s: usize, p: usize) -> PResult<Option<usize>> {
        if p + 1 >= self.pat.len() {
            return Err("unbalanced pattern".into());
        }
        if self.src.get(s) != Some(&self.pat[p]) {
            return Ok(None);
        }
        let (b, e) = (self.pat[p], self.pat[p + 1]);
        let mut cont = 1;
        let mut s = s + 1;
        while s < self.src.len() {
            let c = self.src[s];
            if c == e {
                cont -= 1;
                if cont == 0 {
                    return Ok(Some(s + 1));
                }
            } else if c == b {
                cont += 1;
            }
            s += 1;
        }
        Ok(None)
    }

    fn max_expand(&mut self, s: usize, p: usize, ep: usize) -> PResult<Option<usize>> {
        let mut i = 0;
        while self.single(s + i, p, ep) {
            i += 1;
        }
        loop {
            if let Some(r) = self.do_match(s + i, ep + 1)? {
                return Ok(Some(r));
            }
            if i == 0 {
                return Ok(None);
            }
            i -= 1;
        }
    }

    fn min_expand(&mut self, s: usize, p: usize, ep: usize) -> PResult<Option<usize>> {
        let mut s = s;
        loop {
            if let Some(r) = self.do_match(s, ep + 1)? {
                return Ok(Some(r));
            }
            if self.single(s, p, ep) {
                s += 1;
            } else {
                return Ok(None);
            }
        }
    }

    fn start_capture(&mut self, s: usize, p: usize, what: isize) -> PResult<Option<usize>> {
        if self.level >= MAXCAPTURES {
            return Err("too many captures".into());
        }
        self.cap[self.level] = (s, what);
        self.level += 1;
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.level -= 1;
        }
        Ok(r)
    }

    fn end_capture(&mut self, s: usize, p: usize) -> PResult<Option<usize>> {
        let l = (0..self.level).rev().find(|&i| self.cap[i].1 == CAP_UNFINISHED);
        let Some(l) = l else { return Err("invalid pattern capture".into()) };
        self.cap[l].1 = (s - self.cap[l].0) as isize;
        let r = self.do_match(s, p)?;
        if r.is_none() {
            self.cap[l].1 = CAP_UNFINISHED;
        }
        Ok(r)
    }

    fn match_capture(&self, s: usize, l: u8) -> PResult<Option<usize>> {
        let l = l.wrapping_sub(b'1') as usize;
        if l >= self.level || self.cap[l].1 == CAP_UNFINISHED {
            return Err("invalid capture index".into());
        }
        if self.cap[l].1 == CAP_POSITION {
            return Ok(None);
        }
        let (init, len) = (self.cap[l].0, self.cap[l].1 as usize);
        if self.src.len() - s >= len && self.src[init..init + len] == self.src[s..s + len] {
            Ok(Some(s + len))
        } else {
            Ok(None)
        }
    }

    pub fn do_match(&mut self, s: usize, p: usize) -> PResult<Option<usize>> {
        let mut s = s;
        let mut p = p;
        loop {
            if p >= self.pat.len() {
                return Ok(Some(s));
            }
            match self.pat[p] {
                b'(' => {
                    return if self.pat_at(p + 1) == b')' {
                        self.start_capture(s, p + 2, CAP_POSITION)
                    } else {
                        self.start_capture(s, p + 1, CAP_UNFINISHED)
                    };
                }
                b')' => return self.end_capture(s, p + 1),
                b'%' if self.pat_at(p + 1) == b'b' => match self.match_balance(s, p + 2)? {
                    Some(ns) => {
                        s = ns;
                        p += 4;
                        continue;
                    }
                    None => return Ok(None),
                },
                b'%' if self.pat_at(p + 1) == b'f' => {
                    p += 2;
                    if self.pat_at(p) != b'[' {
                        return Err("missing '[' after '%f' in pattern".into());
                    }
                    let ep = self.class_end(p)?;
                    let prev = if s == 0 { 0 } else { self.src[s - 1] };
                    let cur = self.src.get(s).copied().unwrap_or(0);
                    if !self.bracket(prev, p, ep - 1) && self.bracket(cur, p, ep - 1) {
                        p = ep;
                        continue;
                    }
                    return Ok(None);
                }
                b'%' if self.pat_at(p + 1).is_ascii_digit() => match self.match_capture(s, self.pat_at(p + 1))? {
                    Some(ns) => {
                        s = ns;
                        p += 2;
                        continue;
                    }
                    None => return Ok(None),
                },
                b'$' if p + 1 == self.pat.len() => {
                    return Ok(if s == self.src.len() { Some(s) } else { None });
                }
                _ => {}
            }
            let ep = self.class_end(p)?;
            let m = s < self.src.len() && self.single(s, p, ep);
            match self.pat_at(ep) {
                b'?' => {
                    if m && let Some(r) = self.do_match(s + 1, ep + 1)? {
                        return Ok(Some(r));
                    }
                    p = ep + 1;
                }
                b'*' => return self.max_expand(s, p, ep),
                b'+' => return if m { self.max_expand(s + 1, p, ep) } else { Ok(None) },
                b'-' => return self.min_expand(s, p, ep),
                _ => {
                    if !m {
                        return Ok(None);
                    }
                    s += 1;
                    p = ep;
                }
            }
        }
    }

    pub fn capture(&self, i: usize, s: usize, e: usize) -> PResult<Capture> {
        if i >= self.level {
            if i == 0 {
                return Ok(Capture::Str(s, e));
            }
            return Err("invalid capture index".into());
        }
        let (init, l) = self.cap[i];
        if l == CAP_UNFINISHED {
            return Err("unfinished capture".into());
        }
        if l == CAP_POSITION {
            return Ok(Capture::Pos(init + 1));
        }
        Ok(Capture::Str(init, init + l as usize))
    }

    pub fn captures(&self, s: usize, e: usize, whole: bool) -> PResult<Vec<Capture>> {
        let n = if self.level == 0 && whole { 1 } else { self.level };
        (0..n).map(|i| self.capture(i, s, e)).collect()
    }
}
