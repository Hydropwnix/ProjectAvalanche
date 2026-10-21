use std::rc::Rc;

use crate::ast::*;
use crate::bytecode::{Capture, Op, Proto, VarDesc};
use crate::value::{Str, Value};

const FIELDS_PER_FLUSH: u32 = 50;

pub fn fb_round(x: u32) -> u32 {
    let mut x = x;
    let mut e = 0;
    while x >= 16 {
        x = (x + 1) >> 1;
        e += 1;
    }
    let fb = if x < 8 { x } else { ((e + 1) << 3) | (x - 8) };
    let e = (fb >> 3) & 31;
    if e == 0 { fb } else { ((fb & 7) + 8) << (e - 1) }
}

struct Compiler<'a> {
    f: &'a FuncBody,
    p: Proto,
    line: u32,
    breaks: Vec<Vec<usize>>,
}

pub fn compile(f: &FuncBody, source: Rc<str>) -> Rc<Proto> {
    Rc::new(compile_fn(f, source, &[]))
}

fn compile_fn(f: &FuncBody, source: Rc<str>, parent: &[LocalDecl]) -> Proto {
    let mut c = Compiler { f, p: Proto::default(), line: f.line, breaks: Vec::new() };
    c.p.source = source;
    c.p.num_params = f.params.len() as u16;
    c.p.is_vararg = f.is_vararg;
    c.p.arg_slot = if f.needs_arg { f.arg_local.map(|a| f.locals[a as usize].slot) } else { None };
    c.p.num_slots = f.num_slots;
    c.p.line = f.line;
    c.p.last_line = f.last_line;
    for &id in f.params.iter().chain(f.arg_local.iter()) {
        let d = &f.locals[id as usize];
        if d.captured {
            c.p.captured_params.push(d.slot);
        }
    }
    for (name, src) in &f.upvals {
        c.p.upval_names.push(name.clone());
        c.p.captures.push(match src {
            UpvalSource::Local(id) => Capture::Slot(parent.get(*id as usize).map_or(0, |d| d.slot)),
            UpvalSource::Upval(i) => Capture::Upval(*i),
        });
    }
    c.block(&f.body);
    c.emit(Op::Return { n: 0, multi: false });
    c.p
}

impl Compiler<'_> {
    fn emit(&mut self, op: Op) -> usize {
        self.p.code.push(op);
        self.p.lines.push(self.line);
        self.p.code.len() - 1
    }

    fn pc(&self) -> usize {
        self.p.code.len()
    }

    fn patch(&mut self, at: usize, target: usize) {
        let off = target as i32 - at as i32 - 1;
        self.p.code[at] = match self.p.code[at] {
            Op::Jump(_) => Op::Jump(off),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(off),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(off),
            Op::AndJump(_) => Op::AndJump(off),
            Op::OrJump(_) => Op::OrJump(off),
            Op::ForPrep { base, .. } => Op::ForPrep { base, jump: off },
            Op::ForLoop { base, .. } => Op::ForLoop { base, jump: off },
            Op::TForLoop { base, .. } => Op::TForLoop { base, jump: off },
            op => op,
        };
    }

    fn hint(&mut self, pc: usize, a: Option<VarDesc>, b: Option<VarDesc>) {
        if a.is_some() || b.is_some() {
            self.p.hints.push((pc as u32, a, b));
        }
    }

    fn konst(&mut self, v: Value) -> u32 {
        let found = self.p.consts.iter().position(|k| match (k, &v) {
            (Value::Num(a), Value::Num(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            _ => false,
        });
        if let Some(i) = found {
            return i as u32;
        }
        self.p.consts.push(v);
        (self.p.consts.len() - 1) as u32
    }

    fn kstr(&mut self, s: &Str) -> u32 {
        self.konst(Value::Str(s.clone()))
    }

    fn local(&self, id: LocalId) -> &LocalDecl {
        &self.f.locals[id as usize]
    }

    fn block(&mut self, b: &Block) {
        for s in b {
            self.stat(s);
        }
    }

    fn declare(&mut self, id: LocalId) {
        let d = self.local(id);
        let op = if d.captured { Op::NewCell(d.slot) } else { Op::SetLocal(d.slot) };
        self.emit(op);
    }

    fn explist_n(&mut self, exprs: &[Expr], n: usize) {
        let len = exprs.len();
        for (i, e) in exprs.iter().enumerate() {
            if i + 1 == len && e.is_multi() {
                let want = n as i32 - (len as i32 - 1);
                self.expr_results(e, want.max(0));
                if want < 0 {
                    let extra = (len - 1 - n) as u32;
                    if extra > 0 {
                        self.emit(Op::Pop(extra));
                    }
                }
                return;
            }
            self.expr(e);
        }
        if len > n {
            self.emit(Op::Pop((len - n) as u32));
        }
        for _ in len..n {
            self.emit(Op::Nil);
        }
    }

    fn explist_all(&mut self, exprs: &[Expr]) -> (u32, bool) {
        let len = exprs.len();
        for (i, e) in exprs.iter().enumerate() {
            if i + 1 == len && e.is_multi() {
                self.expr_results(e, -1);
                return ((len - 1) as u32, true);
            }
            self.expr(e);
        }
        (len as u32, false)
    }
}

impl Compiler<'_> {
    fn stat(&mut self, s: &Stat) {
        match s {
            Stat::Call(e) => {
                self.expr_results(e, 0);
            }
            Stat::Local(names, exprs) => {
                self.explist_n(exprs, names.len());
                for &id in names.iter().rev() {
                    self.declare(id);
                }
            }
            Stat::LocalFunction(id, body) => {
                let d = self.local(*id).clone();
                if d.captured {
                    self.emit(Op::Nil);
                    self.emit(Op::NewCell(d.slot));
                    self.closure(body);
                    self.emit(Op::SetCell(d.slot));
                } else {
                    self.closure(body);
                    self.emit(Op::SetLocal(d.slot));
                }
            }
            Stat::Assign(targets, exprs) => self.assign(targets, exprs),
            Stat::Do(b) => self.block(b),
            Stat::While(cond, body) => {
                let start = self.pc();
                self.expr(cond);
                let exit = self.emit(Op::JumpIfFalse(0));
                self.breaks.push(Vec::new());
                self.block(body);
                let back = self.emit(Op::Jump(0));
                self.patch(back, start);
                let end = self.pc();
                self.patch(exit, end);
                self.close_loop(end);
            }
            Stat::Repeat(body, cond) => {
                let start = self.pc();
                self.breaks.push(Vec::new());
                self.block(body);
                self.expr(cond);
                let j = self.emit(Op::JumpIfFalse(0));
                self.patch(j, start);
                let end = self.pc();
                self.close_loop(end);
            }
            Stat::If(conds, els) => {
                let mut ends = Vec::new();
                for (i, (c, b)) in conds.iter().enumerate() {
                    self.expr(c);
                    let next = self.emit(Op::JumpIfFalse(0));
                    self.block(b);
                    if i + 1 < conds.len() || els.is_some() {
                        ends.push(self.emit(Op::Jump(0)));
                    }
                    let here = self.pc();
                    self.patch(next, here);
                }
                if let Some(b) = els {
                    self.block(b);
                }
                let end = self.pc();
                for j in ends {
                    self.patch(j, end);
                }
            }
            Stat::NumFor { base, var, start, limit, step, body, line } => {
                self.line = *line;
                self.expr(start);
                self.expr(limit);
                match step {
                    Some(s) => {
                        self.expr(s);
                    }
                    None => {
                        let k = self.konst(Value::Num(1.0));
                        self.emit(Op::Const(k));
                    }
                }
                for i in (0..3).rev() {
                    self.emit(Op::SetLocal(base + i));
                }
                let prep = self.emit(Op::ForPrep { base: *base, jump: 0 });
                let body_start = self.pc();
                self.emit(Op::GetLocal(*base));
                self.declare(*var);
                self.breaks.push(Vec::new());
                self.block(body);
                let lp = self.pc();
                self.patch(prep, lp);
                self.line = *line;
                let l = self.emit(Op::ForLoop { base: *base, jump: 0 });
                self.patch(l, body_start);
                let end = self.pc();
                self.close_loop(end);
            }
            Stat::GenFor { base, vars, exprs, body, line } => {
                self.line = *line;
                self.explist_n(exprs, 3);
                for i in (0..3).rev() {
                    self.emit(Op::SetLocal(base + i));
                }
                let j = self.emit(Op::Jump(0));
                let body_start = self.pc();
                for &v in vars {
                    let d = self.local(v).clone();
                    if d.captured {
                        self.emit(Op::GetLocal(d.slot));
                        self.emit(Op::NewCell(d.slot));
                    }
                }
                self.breaks.push(Vec::new());
                self.block(body);
                let call = self.pc();
                self.patch(j, call);
                self.line = *line;
                self.emit(Op::TForCall { base: *base, nvars: vars.len() as u16 });
                let l = self.emit(Op::TForLoop { base: *base, jump: 0 });
                self.patch(l, body_start);
                let end = self.pc();
                self.close_loop(end);
            }
            Stat::Return(exprs, line) => {
                self.line = *line;
                if exprs.len() == 1
                    && let e @ (Expr::Call(..) | Expr::Method(..)) = &exprs[0]
                {
                    self.call(e, 0, true);
                    return;
                }
                let (n, multi) = self.explist_all(exprs);
                self.emit(Op::Return { n, multi });
            }
            Stat::Line(line) => self.line = *line,
            Stat::Break(line) => {
                self.line = *line;
                let j = self.emit(Op::Jump(0));
                if let Some(b) = self.breaks.last_mut() {
                    b.push(j);
                }
            }
        }
    }

    fn close_loop(&mut self, end: usize) {
        if let Some(bs) = self.breaks.pop() {
            for j in bs {
                self.patch(j, end);
            }
        }
    }

    fn assign(&mut self, targets: &[LValue], exprs: &[Expr]) {
        if targets.len() == 1 && exprs.len() == 1 {
            match &targets[0] {
                LValue::Index(o, k, line) => {
                    let od = self.expr(o);
                    if let Expr::Str(s) = &**k {
                        self.expr(&exprs[0]);
                        self.line = *line;
                        let ks = self.kstr(s);
                        let pc = self.emit(Op::SetField(ks));
                        self.hint(pc, od, None);
                    } else {
                        self.expr(k);
                        self.expr(&exprs[0]);
                        self.line = *line;
                        let pc = self.emit(Op::SetIndex);
                        self.hint(pc, od, None);
                    }
                }
                t => {
                    self.expr(&exprs[0]);
                    self.store(t);
                }
            }
            return;
        }
        let mut parts = Vec::new();
        for t in targets {
            if let LValue::Index(o, k, _) = t {
                let od = self.expr(o);
                self.expr(k);
                parts.push((2u16, od));
            } else {
                parts.push((0, None));
            }
        }
        self.explist_n(exprs, targets.len());
        let n = targets.len();
        for i in (0..n).rev() {
            match &targets[i] {
                LValue::Index(_, _, line) => {
                    let above: u16 = parts[i + 1..].iter().map(|p| p.0).sum::<u16>() + i as u16;
                    self.line = *line;
                    let pc = self.emit(Op::SetIndexDeep(above));
                    let od = parts[i].1.clone();
                    self.hint(pc, od, None);
                }
                t => self.store(t),
            }
        }
        let total: u32 = parts.iter().map(|p| u32::from(p.0)).sum();
        if total > 0 {
            self.emit(Op::Pop(total));
        }
    }

    fn store(&mut self, t: &LValue) {
        match t {
            LValue::Local(id) => {
                let d = self.local(*id);
                let op = if d.captured { Op::SetCell(d.slot) } else { Op::SetLocal(d.slot) };
                self.emit(op);
            }
            LValue::Upval(i) => {
                self.emit(Op::SetUpval(*i));
            }
            LValue::Global(n) => {
                let k = self.kstr(n);
                self.emit(Op::SetGlobal(k));
            }
            LValue::Index(..) => {}
        }
    }

    fn closure(&mut self, body: &FuncBody) {
        let child = compile_fn(body, self.p.source.clone(), &self.f.locals);
        self.p.protos.push(Rc::new(child));
        let i = (self.p.protos.len() - 1) as u32;
        self.emit(Op::Closure(i));
    }
}

impl Compiler<'_> {
    fn expr(&mut self, e: &Expr) -> Option<VarDesc> {
        match e {
            Expr::Nil => {
                self.emit(Op::Nil);
                None
            }
            Expr::True => {
                self.emit(Op::Bool(true));
                None
            }
            Expr::False => {
                self.emit(Op::Bool(false));
                None
            }
            Expr::Num(n) => {
                let k = self.konst(Value::Num(*n));
                self.emit(Op::Const(k));
                None
            }
            Expr::Str(s) => {
                let k = self.kstr(s);
                self.emit(Op::Const(k));
                None
            }
            Expr::Vararg => {
                self.emit(Op::Vararg(1));
                None
            }
            Expr::Func(body) => {
                self.closure(body);
                None
            }
            Expr::Local(id) => {
                let d = self.local(*id).clone();
                self.emit(if d.captured { Op::GetCell(d.slot) } else { Op::GetLocal(d.slot) });
                Some(VarDesc::Local(d.name))
            }
            Expr::Upval(i) => {
                self.emit(Op::GetUpval(*i));
                self.f.upvals.get(*i as usize).map(|(n, _)| VarDesc::Upvalue(n.clone()))
            }
            Expr::Global(n) => {
                let k = self.kstr(n);
                self.emit(Op::GetGlobal(k));
                Some(VarDesc::Global(n.clone()))
            }
            Expr::Index(o, k, line) => {
                let od = self.expr(o);
                if let Expr::Str(s) = &**k {
                    self.line = *line;
                    let ks = self.kstr(s);
                    let pc = self.emit(Op::GetField(ks));
                    self.hint(pc, od, None);
                    Some(VarDesc::Field(s.clone()))
                } else {
                    self.expr(k);
                    self.line = *line;
                    let pc = self.emit(Op::GetIndex);
                    self.hint(pc, od, None);
                    None
                }
            }
            Expr::Call(..) | Expr::Method(..) => {
                self.call(e, 1, false);
                None
            }
            Expr::Paren(inner) => self.expr(inner),
            Expr::Bin(op, a, b, line) => {
                match op {
                    BinOp::And | BinOp::Or => {
                        self.expr(a);
                        let j = self.emit(if *op == BinOp::And { Op::AndJump(0) } else { Op::OrJump(0) });
                        self.expr(b);
                        let end = self.pc();
                        self.patch(j, end);
                        return None;
                    }
                    BinOp::Concat => {
                        let mut ops = vec![&**a];
                        let mut rest = &**b;
                        while let Expr::Bin(BinOp::Concat, x, y, _) = rest {
                            ops.push(x);
                            rest = y;
                        }
                        ops.push(rest);
                        let descs: Vec<Option<VarDesc>> = ops.iter().map(|x| self.expr(x)).collect();
                        self.line = *line;
                        let pc = self.emit(Op::Concat(ops.len() as u32));
                        if descs.iter().any(Option::is_some) {
                            self.p.concat_hints.push((pc as u32, descs));
                        }
                        return None;
                    }
                    _ => {}
                }
                let da = self.expr(a);
                let db = self.expr(b);
                self.line = *line;
                let pc = self.emit(match op {
                    BinOp::Add => Op::Add,
                    BinOp::Sub => Op::Sub,
                    BinOp::Mul => Op::Mul,
                    BinOp::Div => Op::Div,
                    BinOp::Mod => Op::Mod,
                    BinOp::Pow => Op::Pow,
                    BinOp::Eq => Op::Eq,
                    BinOp::Ne => Op::Ne,
                    BinOp::Lt => Op::Lt,
                    BinOp::Le => Op::Le,
                    BinOp::Gt => Op::Gt,
                    _ => Op::Ge,
                });
                self.hint(pc, da, db);
                None
            }
            Expr::Un(op, a, line) => {
                let d = self.expr(a);
                self.line = *line;
                let pc = self.emit(match op {
                    UnOp::Neg => Op::Unm,
                    UnOp::Not => Op::Not,
                    UnOp::Len => Op::Len,
                });
                self.hint(pc, d, None);
                None
            }
            Expr::Table(items, line) => {
                self.table(items, *line);
                None
            }
        }
    }

    fn expr_results(&mut self, e: &Expr, results: i32) {
        match e {
            Expr::Call(..) | Expr::Method(..) => self.call(e, results, false),
            Expr::Vararg => {
                self.emit(Op::Vararg(results));
            }
            _ => {
                self.expr(e);
                if results == 0 {
                    self.emit(Op::Pop(1));
                }
            }
        }
    }

    fn call(&mut self, e: &Expr, results: i32, tail: bool) {
        let (fdesc, nself, args, line) = match e {
            Expr::Call(f, args, line) => (self.expr(f), 0, args, *line),
            Expr::Method(o, m, args, line) => {
                let od = self.expr(o);
                self.line = *line;
                let k = self.kstr(m);
                let pc = self.emit(Op::SelfOp(k));
                self.hint(pc, od, None);
                (Some(VarDesc::Method(m.clone())), 1, args, *line)
            }
            _ => return,
        };
        let (n, multi) = self.explist_all(args);
        self.line = line;
        let args = n + nself;
        let pc =
            if tail { self.emit(Op::TailCall { args, multi }) } else { self.emit(Op::Call { args, multi, results }) };
        self.hint(pc, fdesc, None);
        if tail {
            self.emit(Op::Return { n: 0, multi: true });
        }
    }

    fn table(&mut self, items: &[TableItem], line: u32) {
        let mut na = 0u32;
        let mut nh = 0u32;
        for (i, it) in items.iter().enumerate() {
            match it {
                TableItem::Pos(e) => {
                    if !(i + 1 == items.len() && e.is_multi()) {
                        na += 1;
                    }
                }
                _ => nh += 1,
            }
        }
        self.line = line;
        self.emit(Op::NewTable(fb_round(na), fb_round(nh)));
        let mut pending = 0u32;
        let mut stored = 0u32;
        for (i, it) in items.iter().enumerate() {
            match it {
                TableItem::Pos(e) => {
                    if i + 1 == items.len() && e.is_multi() {
                        self.expr_results(e, -1);
                        self.emit(Op::SetList { n: pending, multi: true, first: stored + 1 });
                        pending = 0;
                        continue;
                    }
                    self.expr(e);
                    pending += 1;
                    if pending == FIELDS_PER_FLUSH {
                        self.emit(Op::SetList { n: pending, multi: false, first: stored + 1 });
                        stored += pending;
                        pending = 0;
                    }
                }
                TableItem::Named(k, v) => {
                    self.emit_table_dup(pending);
                    let ks = self.kstr(k);
                    self.expr(v);
                    self.emit(Op::SetField(ks));
                }
                TableItem::Keyed(k, v) => {
                    self.emit_table_dup(pending);
                    self.expr(k);
                    self.expr(v);
                    self.emit(Op::SetIndex);
                }
            }
        }
        if pending > 0 {
            self.emit(Op::SetList { n: pending, multi: false, first: stored + 1 });
        }
    }

    fn emit_table_dup(&mut self, pending: u32) {
        self.emit(Op::Dup(pending));
    }
}
