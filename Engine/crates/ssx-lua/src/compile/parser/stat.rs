use super::*;

fn block_follow(t: &Tok) -> bool {
    matches!(t, Tok::Else | Tok::Elseif | Tok::End | Tok::Until | Tok::Eof)
}

impl Parser<'_> {
    pub(super) fn block(&mut self) -> Result<Block, ParseError> {
        self.enter()?;
        let mut stats = Vec::new();
        while !block_follow(&self.tok.tok) {
            let last = matches!(self.tok.tok, Tok::Return | Tok::Break);
            stats.push(Stat::Line(self.tok_end));
            stats.push(self.statement()?);
            self.test_next(&Tok::Char(b';'))?;
            if last {
                break;
            }
        }
        self.leave();
        Ok(stats)
    }

    fn scoped_block(&mut self, is_loop: bool) -> Result<Block, ParseError> {
        self.open_block(is_loop);
        let b = self.block()?;
        self.close_block();
        Ok(b)
    }

    fn statement(&mut self) -> Result<Stat, ParseError> {
        let line = self.tok_end;
        match self.tok.tok {
            Tok::If => self.if_stat(line),
            Tok::While => {
                self.advance()?;
                let cond = self.expr()?;
                self.check_next(&Tok::Do)?;
                let body = self.scoped_block(true)?;
                self.check_match(&Tok::End, &Tok::While, line)?;
                Ok(Stat::While(cond, body))
            }
            Tok::Do => {
                self.advance()?;
                let b = self.scoped_block(false)?;
                self.check_match(&Tok::End, &Tok::Do, line)?;
                Ok(Stat::Do(b))
            }
            Tok::For => self.for_stat(line),
            Tok::Repeat => {
                self.advance()?;
                self.open_block(true);
                let body = self.block()?;
                self.check_match(&Tok::Until, &Tok::Repeat, line)?;
                let cond = self.expr()?;
                self.close_block();
                Ok(Stat::Repeat(body, cond))
            }
            Tok::Function => {
                self.advance()?;
                let n = self.name()?;
                let mut target = self.single_var(n)?;
                let mut is_method = false;
                let mut tline = self.tok_end;
                loop {
                    if self.tok.tok == Tok::Char(b'.') || self.tok.tok == Tok::Char(b':') {
                        is_method = self.tok.tok == Tok::Char(b':');
                        self.advance()?;
                        tline = self.tok_end;
                        let k = self.name()?;
                        target = Expr::Index(Box::new(target), Box::new(Expr::Str(k)), tline);
                        if is_method {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                let _ = tline;
                let f = self.body(is_method, line)?;
                Ok(Stat::Assign(vec![self.to_lvalue(target)?], vec![Expr::Func(Box::new(f))]))
            }
            Tok::Local => {
                self.advance()?;
                if self.test_next(&Tok::Function)? {
                    let n = self.name()?;
                    let id = self.new_local(n, 0)?;
                    self.activate(&[id]);
                    let f = self.body(false, line)?;
                    Ok(Stat::LocalFunction(id, Box::new(f)))
                } else {
                    let mut names = Vec::new();
                    loop {
                        let n = self.name()?;
                        names.push(self.new_local(n, names.len())?);
                        if !self.test_next(&Tok::Char(b','))? {
                            break;
                        }
                    }
                    let exprs = if self.test_next(&Tok::Char(b'='))? { self.exprlist()? } else { Vec::new() };
                    self.activate(&names);
                    Ok(Stat::Local(names, exprs))
                }
            }
            Tok::Return => {
                self.advance()?;
                let exprs = if block_follow(&self.tok.tok) || self.tok.tok == Tok::Char(b';') {
                    Vec::new()
                } else {
                    self.exprlist()?
                };
                Ok(Stat::Return(exprs, line))
            }
            Tok::Break => {
                self.advance()?;
                if !self.cur().blocks.iter().any(|b| b.is_loop) {
                    return Err(self.error("no loop to break"));
                }
                Ok(Stat::Break(line))
            }
            _ => self.expr_stat(),
        }
    }

    fn if_stat(&mut self, line: u32) -> Result<Stat, ParseError> {
        let mut conds = Vec::new();
        self.advance()?;
        let c = self.expr()?;
        self.check_next(&Tok::Then)?;
        conds.push((c, self.scoped_block(false)?));
        let mut els = None;
        loop {
            match self.tok.tok {
                Tok::Elseif => {
                    self.advance()?;
                    let c = self.expr()?;
                    self.check_next(&Tok::Then)?;
                    conds.push((c, self.scoped_block(false)?));
                }
                Tok::Else => {
                    self.advance()?;
                    els = Some(self.scoped_block(false)?);
                    self.check_match(&Tok::End, &Tok::If, line)?;
                    break;
                }
                _ => {
                    self.check_match(&Tok::End, &Tok::If, line)?;
                    break;
                }
            }
        }
        Ok(Stat::If(conds, els))
    }

    fn for_stat(&mut self, line: u32) -> Result<Stat, ParseError> {
        self.advance()?;
        self.open_block(true);
        let n1 = self.name()?;
        let base = self.cur().active.len() as u16;
        let st = match self.tok.tok {
            Tok::Char(b'=') => {
                self.advance()?;
                let start = self.expr()?;
                self.check_next(&Tok::Char(b','))?;
                let limit = self.expr()?;
                let step = if self.test_next(&Tok::Char(b','))? { Some(self.expr()?) } else { None };
                self.check_next(&Tok::Do)?;
                let hidden: Vec<LocalId> = ["(for index)", "(for limit)", "(for step)"]
                    .iter()
                    .enumerate()
                    .map(|(i, s)| self.new_local(Str::from(*s), i))
                    .collect::<Result<_, _>>()?;
                self.activate(&hidden);
                let var = self.new_local(n1, 0)?;
                self.open_block(false);
                self.activate(&[var]);
                let body = self.block()?;
                self.close_block();
                Stat::NumFor { base, var, start, limit, step, body, line }
            }
            Tok::Char(b',') | Tok::In => {
                let mut names = vec![n1];
                while self.test_next(&Tok::Char(b','))? {
                    names.push(self.name()?);
                }
                self.check_next(&Tok::In)?;
                let exprs = self.exprlist()?;
                self.check_next(&Tok::Do)?;
                let hidden: Vec<LocalId> = ["(for generator)", "(for state)", "(for control)"]
                    .iter()
                    .enumerate()
                    .map(|(i, s)| self.new_local(Str::from(*s), i))
                    .collect::<Result<_, _>>()?;
                self.activate(&hidden);
                let mut vars = Vec::new();
                for (i, n) in names.into_iter().enumerate() {
                    vars.push(self.new_local(n, i)?);
                }
                self.open_block(false);
                self.activate(&vars);
                let body = self.block()?;
                self.close_block();
                Stat::GenFor { base, vars, exprs, body, line }
            }
            _ => return Err(self.error("'=' or 'in' expected")),
        };
        self.check_match(&Tok::End, &Tok::For, line)?;
        self.close_block();
        Ok(st)
    }

    fn to_lvalue(&self, e: Expr) -> Result<LValue, ParseError> {
        Ok(match e {
            Expr::Local(id) => LValue::Local(id),
            Expr::Upval(i) => LValue::Upval(i),
            Expr::Global(n) => LValue::Global(n),
            Expr::Index(o, k, l) => LValue::Index(o, k, l),
            _ => return Err(self.error("syntax error")),
        })
    }

    fn expr_stat(&mut self) -> Result<Stat, ParseError> {
        let e = self.primary_expr()?;
        if matches!(e, Expr::Call(..) | Expr::Method(..)) {
            return Ok(Stat::Call(e));
        }
        let mut targets = vec![self.to_lvalue(e)?];
        while self.test_next(&Tok::Char(b','))? {
            let e = self.primary_expr()?;
            if matches!(e, Expr::Call(..) | Expr::Method(..)) {
                return Err(self.error("syntax error"));
            }
            targets.push(self.to_lvalue(e)?);
        }
        self.check_next(&Tok::Char(b'='))?;
        let exprs = self.exprlist()?;
        Ok(Stat::Assign(targets, exprs))
    }
}
