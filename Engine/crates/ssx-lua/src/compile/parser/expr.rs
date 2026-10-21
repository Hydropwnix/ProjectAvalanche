use super::*;

fn binop(t: &Tok) -> Option<(BinOp, u8, u8)> {
    Some(match t {
        Tok::Char(b'+') => (BinOp::Add, 6, 6),
        Tok::Char(b'-') => (BinOp::Sub, 6, 6),
        Tok::Char(b'*') => (BinOp::Mul, 7, 7),
        Tok::Char(b'/') => (BinOp::Div, 7, 7),
        Tok::Char(b'%') => (BinOp::Mod, 7, 7),
        Tok::Char(b'^') => (BinOp::Pow, 10, 9),
        Tok::Concat => (BinOp::Concat, 5, 4),
        Tok::Eq => (BinOp::Eq, 3, 3),
        Tok::Ne => (BinOp::Ne, 3, 3),
        Tok::Char(b'<') => (BinOp::Lt, 3, 3),
        Tok::Le => (BinOp::Le, 3, 3),
        Tok::Char(b'>') => (BinOp::Gt, 3, 3),
        Tok::Ge => (BinOp::Ge, 3, 3),
        Tok::And => (BinOp::And, 2, 2),
        Tok::Or => (BinOp::Or, 1, 1),
        _ => return None,
    })
}

const UNARY_PRIORITY: u8 = 8;

type SubExpr = (Expr, Option<(BinOp, u8, u8)>);

impl Parser<'_> {
    pub(super) fn exprlist(&mut self) -> Result<Vec<Expr>, ParseError> {
        let mut v = vec![self.expr()?];
        while self.test_next(&Tok::Char(b','))? {
            v.push(self.expr()?);
        }
        Ok(v)
    }

    pub fn expr(&mut self) -> Result<Expr, ParseError> {
        self.subexpr(0).map(|(e, _)| e)
    }

    fn subexpr(&mut self, limit: u8) -> Result<SubExpr, ParseError> {
        self.enter()?;
        let line = self.tok_end;
        let un = match self.tok.tok {
            Tok::Not => Some(UnOp::Not),
            Tok::Char(b'-') => Some(UnOp::Neg),
            Tok::Char(b'#') => Some(UnOp::Len),
            _ => None,
        };
        let mut e = if let Some(op) = un {
            self.advance()?;
            let (x, _) = self.subexpr(UNARY_PRIORITY)?;
            fold_unary(op, x, line)
        } else {
            self.simple_expr()?
        };
        let mut op = binop(&self.tok.tok);
        while let Some((o, left, right)) = op {
            if left <= limit {
                break;
            }
            let line = self.tok_end;
            self.advance()?;
            let (rhs, next) = self.subexpr(right)?;
            e = Expr::Bin(o, Box::new(e), Box::new(rhs), line);
            op = next;
        }
        self.leave();
        Ok((e, op))
    }

    fn simple_expr(&mut self) -> Result<Expr, ParseError> {
        let e = match &self.tok.tok {
            Tok::Num(n) => Expr::Num(*n),
            Tok::Str(s) => Expr::Str(Str::new(s)),
            Tok::Nil => Expr::Nil,
            Tok::True => Expr::True,
            Tok::False => Expr::False,
            Tok::Dots => {
                let fs = self.cur();
                if !fs.f.is_vararg {
                    return Err(self.error("cannot use '...' outside a vararg function"));
                }
                fs.f.needs_arg = false;
                Expr::Vararg
            }
            Tok::Char(b'{') => return self.constructor(),
            Tok::Function => {
                let line = self.tok_end;
                self.advance()?;
                return Ok(Expr::Func(Box::new(self.body(false, line)?)));
            }
            _ => return self.primary_expr(),
        };
        self.advance()?;
        Ok(e)
    }

    fn prefix_expr(&mut self) -> Result<Expr, ParseError> {
        match self.tok.tok {
            Tok::Name(_) => {
                let n = self.name()?;
                self.single_var(n)
            }
            Tok::Char(b'(') => {
                let line = self.tok_end;
                self.advance()?;
                let e = self.expr()?;
                self.check_match(&Tok::Char(b')'), &Tok::Char(b'('), line)?;
                Ok(Expr::Paren(Box::new(e)))
            }
            _ => Err(self.error("unexpected symbol")),
        }
    }

    pub(super) fn primary_expr(&mut self) -> Result<Expr, ParseError> {
        let mut e = self.prefix_expr()?;
        loop {
            let line = self.tok_end;
            match self.tok.tok {
                Tok::Char(b'.') => {
                    self.advance()?;
                    let k = self.name()?;
                    e = Expr::Index(Box::new(e), Box::new(Expr::Str(k)), line);
                }
                Tok::Char(b'[') => {
                    self.advance()?;
                    let k = self.expr()?;
                    self.check_next(&Tok::Char(b']'))?;
                    e = Expr::Index(Box::new(e), Box::new(k), line);
                }
                Tok::Char(b':') => {
                    self.advance()?;
                    let m = self.name()?;
                    let args = self.func_args()?;
                    e = Expr::Method(Box::new(e), m, args, line);
                }
                Tok::Char(b'(') | Tok::Str(_) | Tok::Char(b'{') => {
                    let args = self.func_args()?;
                    e = Expr::Call(Box::new(e), args, line);
                }
                _ => return Ok(e),
            }
        }
    }

    fn func_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        let line = self.tok_end;
        match &self.tok.tok {
            Tok::Str(s) => {
                let e = Expr::Str(Str::new(s));
                self.advance()?;
                Ok(vec![e])
            }
            Tok::Char(b'{') => Ok(vec![self.constructor()?]),
            Tok::Char(b'(') => {
                if line != self.last_line {
                    return Err(self.error("ambiguous syntax (function call x new statement)"));
                }
                self.advance()?;
                let args = if self.tok.tok == Tok::Char(b')') { Vec::new() } else { self.exprlist()? };
                self.check_match(&Tok::Char(b')'), &Tok::Char(b'('), line)?;
                Ok(args)
            }
            _ => Err(self.error("function arguments expected")),
        }
    }

    fn constructor(&mut self) -> Result<Expr, ParseError> {
        let line = self.tok_end;
        self.check_next(&Tok::Char(b'{'))?;
        let mut items = Vec::new();
        loop {
            if self.tok.tok == Tok::Char(b'}') {
                break;
            }
            match self.tok.tok {
                Tok::Name(_) => {
                    if self.peek()? == &Tok::Char(b'=') {
                        let n = self.name()?;
                        self.advance()?;
                        items.push(TableItem::Named(n, self.expr()?));
                    } else {
                        items.push(TableItem::Pos(self.expr()?));
                    }
                }
                Tok::Char(b'[') => {
                    self.advance()?;
                    let k = self.expr()?;
                    self.check_next(&Tok::Char(b']'))?;
                    self.check_next(&Tok::Char(b'='))?;
                    items.push(TableItem::Keyed(k, self.expr()?));
                }
                _ => items.push(TableItem::Pos(self.expr()?)),
            }
            if !(self.test_next(&Tok::Char(b','))? || self.test_next(&Tok::Char(b';'))?) {
                break;
            }
        }
        self.check_match(&Tok::Char(b'}'), &Tok::Char(b'{'), line)?;
        Ok(Expr::Table(items, line))
    }

    pub(super) fn body(&mut self, is_method: bool, line: u32) -> Result<FuncBody, ParseError> {
        self.fs.push(FuncState::new(line, false));
        self.open_block(false);
        let mut params = Vec::new();
        if is_method {
            params.push(self.new_local(Str::from("self"), 0)?);
            self.activate(&params.clone());
        }
        self.check_next(&Tok::Char(b'('))?;
        let mut names = Vec::new();
        if self.tok.tok != Tok::Char(b')') {
            loop {
                match self.tok.tok {
                    Tok::Name(_) => {
                        let n = self.name()?;
                        names.push(self.new_local(n, names.len())?);
                    }
                    Tok::Dots => {
                        self.advance()?;
                        let a = self.new_local(Str::from("arg"), names.len())?;
                        names.push(a);
                        let fs = self.cur();
                        fs.f.is_vararg = true;
                        fs.f.needs_arg = true;
                        fs.f.arg_local = Some(a);
                        break;
                    }
                    _ => return Err(self.error("<name> or '...' expected")),
                }
                if !self.test_next(&Tok::Char(b','))? {
                    break;
                }
            }
        }
        self.activate(&names);
        let fs = self.cur();
        let arg = fs.f.arg_local;
        params.extend(names.into_iter().filter(|n| Some(*n) != arg));
        self.check_next(&Tok::Char(b')'))?;
        let body = self.block()?;
        let last = self.tok_end;
        self.check_match(&Tok::End, &Tok::Function, line)?;
        self.close_block();
        let mut fs = self.fs.pop().ok_or_else(|| ParseError("parser state lost".into()))?;
        fs.f.params = params;
        fs.f.body = body;
        fs.f.last_line = last;
        Ok(fs.f)
    }
}

fn fold_unary(op: UnOp, e: Expr, line: u32) -> Expr {
    match (op, &e) {
        (UnOp::Neg, Expr::Num(n)) => Expr::Num(-n),
        _ => Expr::Un(op, Box::new(e), line),
    }
}
