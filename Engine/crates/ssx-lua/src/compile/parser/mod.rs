mod expr;
mod stat;

use crate::ast::*;
use crate::lexer::{LexError, Lexer, Tok, Token};
use crate::value::Str;

const MAXVARS: usize = 200;
const MAXUPVALUES: usize = 60;
const MAXCCALLS: u32 = 200;

#[derive(Clone, Debug)]
pub struct ParseError(pub String);

pub fn chunkid(source: &str) -> String {
    const IDSIZE: usize = 60;
    if let Some(rest) = source.strip_prefix('=') {
        return rest.chars().take(IDSIZE - 1).collect();
    }
    if let Some(rest) = source.strip_prefix('@') {
        let max = IDSIZE - 8;
        return if rest.len() > max { format!("...{}", &rest[rest.len() - max..]) } else { rest.to_string() };
    }
    let first = source.split(['\n', '\r']).next().unwrap_or("");
    let max = IDSIZE - 17;
    if first.len() < source.len() || first.len() > max {
        let n = first.len().min(max);
        format!("[string \"{}...\"]", &first[..n])
    } else {
        format!("[string \"{source}\"]")
    }
}

struct BlockInfo {
    nactive: usize,
    is_loop: bool,
}

struct FuncState {
    f: FuncBody,
    active: Vec<LocalId>,
    blocks: Vec<BlockInfo>,
}

impl FuncState {
    fn new(line: u32, vararg: bool) -> Self {
        FuncState {
            f: FuncBody {
                params: Vec::new(),
                is_vararg: vararg,
                arg_local: None,
                needs_arg: false,
                body: Vec::new(),
                locals: Vec::new(),
                upvals: Vec::new(),
                num_slots: 0,
                line,
                last_line: line,
            },
            active: Vec::new(),
            blocks: Vec::new(),
        }
    }
}

pub struct Parser<'a> {
    lex: Lexer<'a>,
    tok: Token,
    tok_end: u32,
    ahead: Option<(Token, u32)>,
    last_line: u32,
    fs: Vec<FuncState>,
    depth: u32,
    chunk: String,
}

pub fn parse(src: &[u8], chunkname: &str) -> Result<FuncBody, ParseError> {
    let mut p = Parser {
        lex: Lexer::new(src),
        tok: Token { tok: Tok::Eof, line: 1, raw: Vec::new() },
        tok_end: 1,
        ahead: None,
        last_line: 1,
        fs: vec![FuncState::new(0, true)],
        depth: 0,
        chunk: chunkid(chunkname),
    };
    p.advance()?;
    p.open_block(false);
    let body = p.block()?;
    if p.tok.tok != Tok::Eof {
        return Err(p.error_expected(&Tok::Eof));
    }
    p.close_block();
    let mut fs = p.fs.pop().ok_or_else(|| ParseError("parser state lost".into()))?;
    fs.f.body = body;
    Ok(fs.f)
}

impl Parser<'_> {
    fn lex_err(&self, e: LexError) -> ParseError {
        match e.near {
            Some(n) => ParseError(format!("{}:{}: {} near '{}'", self.chunk, e.line, e.msg, n)),
            None => ParseError(format!("{}:{}: {}", self.chunk, e.line, e.msg)),
        }
    }

    fn error(&self, msg: &str) -> ParseError {
        ParseError(format!("{}:{}: {} near '{}'", self.chunk, self.tok_end, msg, self.tok.near()))
    }

    fn error_expected(&self, t: &Tok) -> ParseError {
        self.error(&format!("'{}' expected", t.describe()))
    }

    fn advance(&mut self) -> Result<(), ParseError> {
        self.last_line = self.tok_end;
        if let Some((t, end)) = self.ahead.take() {
            self.tok = t;
            self.tok_end = end;
            return Ok(());
        }
        self.tok = self.lex.token().map_err(|e| self.lex_err(e))?;
        self.tok_end = self.lex.line;
        Ok(())
    }

    fn peek(&mut self) -> Result<&Tok, ParseError> {
        if self.ahead.is_none() {
            let t = self.lex.token().map_err(|e| self.lex_err(e))?;
            self.ahead = Some((t, self.lex.line));
        }
        Ok(self.ahead.as_ref().map(|(t, _)| &t.tok).unwrap_or(&Tok::Eof))
    }

    fn check(&self, t: &Tok) -> Result<(), ParseError> {
        if &self.tok.tok == t { Ok(()) } else { Err(self.error_expected(t)) }
    }

    fn check_next(&mut self, t: &Tok) -> Result<(), ParseError> {
        self.check(t)?;
        self.advance()
    }

    fn test_next(&mut self, t: &Tok) -> Result<bool, ParseError> {
        if &self.tok.tok == t {
            self.advance()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn check_match(&mut self, what: &Tok, who: &Tok, line: u32) -> Result<(), ParseError> {
        if &self.tok.tok == what {
            return self.advance();
        }
        if line == self.tok_end {
            Err(self.error_expected(what))
        } else {
            Err(self.error(&format!("'{}' expected (to close '{}' at line {})", what.describe(), who.describe(), line)))
        }
    }

    fn name(&mut self) -> Result<Str, ParseError> {
        if let Tok::Name(n) = &self.tok.tok {
            let s = Str::new(n);
            self.advance()?;
            Ok(s)
        } else {
            Err(self.error_expected(&Tok::Name(Vec::new())))
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAXCCALLS {
            return Err(ParseError(format!("{}:{}: chunk has too many syntax levels", self.chunk, self.tok_end)));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn cur(&mut self) -> &mut FuncState {
        let n = self.fs.len() - 1;
        &mut self.fs[n]
    }

    fn limit_error(&self, limit: usize, what: &str) -> ParseError {
        let line = self.fs.last().map_or(0, |f| f.f.line);
        let msg = if line == 0 {
            format!("main function has more than {limit} {what}")
        } else {
            format!("function at line {line} has more than {limit} {what}")
        };
        ParseError(format!("{}:{}: {}", self.chunk, self.tok_end, msg))
    }

    fn new_local(&mut self, name: Str, offset: usize) -> Result<LocalId, ParseError> {
        let fs = self.fs.last().ok_or_else(|| ParseError("no function".into()))?;
        let slot = fs.active.len() + offset;
        if slot + 1 > MAXVARS {
            return Err(self.limit_error(MAXVARS, "local variables"));
        }
        let fs = self.cur();
        fs.f.locals.push(LocalDecl { name, slot: slot as u16, captured: false });
        let id = (fs.f.locals.len() - 1) as LocalId;
        fs.f.num_slots = fs.f.num_slots.max(slot as u16 + 1);
        Ok(id)
    }

    fn activate(&mut self, ids: &[LocalId]) {
        self.cur().active.extend_from_slice(ids);
    }

    fn open_block(&mut self, is_loop: bool) {
        let n = self.cur().active.len();
        self.cur().blocks.push(BlockInfo { nactive: n, is_loop });
    }

    fn close_block(&mut self) {
        if let Some(b) = self.cur().blocks.pop() {
            self.cur().active.truncate(b.nactive);
        }
    }

    fn resolve(&mut self, level: usize, name: &Str, base: bool) -> Result<Option<Expr>, ParseError> {
        let fs = &self.fs[level];
        if let Some(&id) = fs.active.iter().rev().find(|&&id| fs.f.locals[id as usize].name == *name) {
            if !base {
                self.fs[level].f.locals[id as usize].captured = true;
            }
            return Ok(Some(Expr::Local(id)));
        }
        if level == 0 {
            return Ok(None);
        }
        let src = match self.resolve(level - 1, name, false)? {
            None => return Ok(None),
            Some(Expr::Local(id)) => UpvalSource::Local(id),
            Some(Expr::Upval(i)) => UpvalSource::Upval(i),
            Some(_) => return Ok(None),
        };
        let ups = &self.fs[level].f.upvals;
        let same = |a: &UpvalSource, b: &UpvalSource| match (a, b) {
            (UpvalSource::Local(x), UpvalSource::Local(y)) => x == y,
            (UpvalSource::Upval(x), UpvalSource::Upval(y)) => x == y,
            _ => false,
        };
        if let Some(i) = ups.iter().position(|(_, s)| same(s, &src)) {
            return Ok(Some(Expr::Upval(i as u16)));
        }
        if ups.len() + 1 > MAXUPVALUES {
            return Err(self.limit_error(MAXUPVALUES, "upvalues"));
        }
        self.fs[level].f.upvals.push((name.clone(), src));
        Ok(Some(Expr::Upval((self.fs[level].f.upvals.len() - 1) as u16)))
    }

    fn single_var(&mut self, name: Str) -> Result<Expr, ParseError> {
        let level = self.fs.len() - 1;
        Ok(self.resolve(level, &name, true)?.unwrap_or(Expr::Global(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(src: &str) -> String {
        parse(src.as_bytes(), "t").map(|_| ()).unwrap_err().0
    }

    #[test]
    fn messages_match_lua_5_1() {
        let cases = [
            ("x = = 1", "1: unexpected symbol near '='"),
            ("f(\n", "2: unexpected symbol near '<eof>'"),
            ("if x then", "1: 'end' expected near '<eof>'"),
            ("if x then\n\n", "3: 'end' expected (to close 'if' at line 1) near '<eof>'"),
            ("x", "1: '=' expected near '<eof>'"),
            ("break", "1: no loop to break near '<eof>'"),
            ("return 1 2", "1: '<eof>' expected near '2'"),
            ("function f() return ... end", "1: cannot use '...' outside a vararg function near '...'"),
            ("f\n(1)", "2: ambiguous syntax (function call x new statement) near '('"),
            ("local t = {1,2", "1: '}' expected near '<eof>'"),
            ("x = 'abc", "1: unfinished string near '<eof>'"),
            ("x = [[a [[b]] ]]", "1: nesting of [[...]] is deprecated near '['"),
            ("a.b:c = 1", "1: function arguments expected near '='"),
            ("local 1", "1: '<name>' expected near '1'"),
            ("for i=1 do end", "1: ',' expected near 'do'"),
            ("for a b in x do end", "1: '=' or 'in' expected near 'b'"),
            ("x = 3..4", "1: malformed number near '3..4'"),
            ("x = '\\300'", "1: escape sequence too large near '''"),
            ("(f)", "1: syntax error near '<eof>'"),
            ("a, f() = 1", "1: syntax error near '='"),
        ];
        for (src, msg) in cases {
            assert_eq!(err(src), format!("[string \"t\"]:{msg}"), "{src:?}");
        }
    }

    #[test]
    fn scoping() {
        let f = parse(b"local a = 1 local function g() return a end local b, c = a, g()", "t").expect("parse");
        assert_eq!(f.locals.len(), 4);
        assert!(f.locals[0].captured);
        assert!(!f.locals[2].captured);
        assert_eq!(f.num_slots, 4);
    }
}
