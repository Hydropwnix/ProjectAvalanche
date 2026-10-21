use std::cell::RefCell;
use std::rc::Rc;

use crate::bytecode::{Capture, Op, VarDesc};
use crate::heap::{Cell, Function};
use crate::meta::Arith;
use crate::value::{GcRef, Value};
use crate::vm::{Frame, Lua, LuaError, LuaResult, MAXCALLS, MAXCCALLS, Pending};

impl Lua {
    pub fn call(&mut self, f: Value, args: &[Value]) -> LuaResult<Vec<Value>> {
        if self.ccalls >= MAXCCALLS {
            return Err(self.rt_error("C stack overflow"));
        }
        let (nf, ns) = (self.th.frames.len(), self.th.stack.len());
        let fidx = ns;
        self.th.stack.push(f);
        self.th.stack.extend_from_slice(args);
        self.ccalls += 1;
        self.th.nny += 1;
        let r = match self.precall(fidx, args.len(), -1, None, true) {
            Ok(true) => self.run(),
            Ok(false) => Ok(()),
            Err(e) => Err(e),
        };
        self.ccalls -= 1;
        self.th.nny -= 1;
        match r {
            Ok(()) => Ok(self.th.stack.split_off(fidx)),
            Err(e) => {
                self.unwind(nf, ns);
                Err(e)
            }
        }
    }

    pub fn unwind(&mut self, nframes: usize, nstack: usize) {
        self.th.frames.truncate(nframes);
        self.th.stack.truncate(nstack);
        self.th.cells.truncate(nstack);
    }

    pub(crate) fn precall(
        &mut self,
        fidx: usize,
        nargs: usize,
        results: i32,
        name: Option<VarDesc>,
        boundary: bool,
    ) -> LuaResult<bool> {
        let mut nargs = nargs;
        let mut name = name;
        let fref = match &self.th.stack[fidx] {
            Value::Func(r) => *r,
            other => {
                let other = other.clone();
                let tm = self.metamethod(&other, "__call");
                let Value::Func(r) = tm else {
                    return Err(self.op_error("call", &other, name.as_ref()));
                };
                self.th.stack.insert(fidx, tm);
                nargs += 1;
                name = None;
                r
            }
        };
        let proto = match self.heap.func(fref) {
            Function::Lua(c) => c.proto.clone(),
            Function::Native(c) => {
                let f = c.f.clone();
                self.th.frames.push(Frame {
                    func: fref,
                    lua: false,
                    pc: 0,
                    base: fidx + 1,
                    varargs: Vec::new(),
                    ret: fidx,
                    results,
                    boundary,
                    tail: false,
                    name,
                });
                let args: Vec<Value> = self.th.stack[fidx + 1..fidx + 1 + nargs].to_vec();
                let r = f(self, &args);
                let vals = match r {
                    Ok(v) => v,
                    Err(LuaError::Yield(v)) => {
                        self.th.frames.pop();
                        self.th.pending = Some(Pending { func: fidx, results, tail: false });
                        return Err(LuaError::Yield(v));
                    }
                    Err(e) => return Err(e),
                };
                self.th.frames.pop();
                self.th.stack.truncate(fidx);
                self.th.cells.truncate(fidx);
                self.push_results(vals, results);
                return Ok(false);
            }
        };
        if self.th.frames.len() >= MAXCALLS {
            return Err(self.rt_error("stack overflow"));
        }
        let base = fidx + 1;
        let mut args: Vec<Value> = self.th.stack.drain(base..base + nargs).collect();
        self.th.stack.truncate(base);
        let np = proto.num_params as usize;
        let mut varargs = Vec::new();
        if proto.is_vararg && args.len() > np {
            varargs = args.split_off(np);
        }
        args.truncate(np);
        args.resize(np, Value::Nil);
        let size = base + proto.num_slots as usize;
        self.th.stack.extend(args);
        self.th.stack.resize(size, Value::Nil);
        if self.th.cells.len() > base {
            self.th.cells.truncate(base);
        }
        self.th.cells.resize(size, None);
        if let Some(slot) = proto.arg_slot {
            let t = self.new_table(varargs.len(), 1);
            for (i, v) in varargs.iter().enumerate() {
                let _ = self.heap.table_mut(t).set(Value::Num((i + 1) as f64), v.clone());
            }
            let _ = self.heap.table_mut(t).set(Value::str("n"), Value::Num(varargs.len() as f64));
            self.th.stack[base + slot as usize] = Value::Table(t);
        }
        for &s in &proto.captured_params {
            let i = base + s as usize;
            let v = std::mem::take(&mut self.th.stack[i]);
            self.th.cells[i] = Some(Rc::new(RefCell::new(v)));
        }
        self.th.frames.push(Frame {
            func: fref,
            lua: true,
            pc: 0,
            base,
            varargs,
            ret: fidx,
            results,
            boundary,
            tail: false,
            name,
        });
        Ok(true)
    }

    pub(crate) fn push_results(&mut self, mut vals: Vec<Value>, results: i32) {
        if results < 0 {
            self.th.multret = vals.len();
        } else {
            vals.resize(results as usize, Value::Nil);
        }
        self.th.stack.extend(vals);
    }

    fn poscall(&mut self, n: usize) -> bool {
        let top = self.th.stack.len();
        let vals: Vec<Value> = self.th.stack.drain(top - n..).collect();
        let Some(fr) = self.th.frames.pop() else { return true };
        self.th.stack.truncate(fr.ret);
        self.th.cells.truncate(fr.ret);
        self.push_results(vals, fr.results);
        fr.boundary
    }

    fn capture(&mut self, caps: &[Capture], base: usize, own: GcRef) -> Vec<Cell> {
        caps.iter()
            .map(|c| match c {
                Capture::Slot(s) => {
                    let i = base + *s as usize;
                    if let Some(Some(cell)) = self.th.cells.get(i) {
                        return cell.clone();
                    }
                    let v = self.th.stack.get(i).cloned().unwrap_or_default();
                    let cell = Rc::new(RefCell::new(v));
                    if i < self.th.cells.len() {
                        self.th.cells[i] = Some(cell.clone());
                    }
                    cell
                }
                Capture::Upval(u) => match self.heap.func(own) {
                    Function::Lua(cl) => cl.upvals.get(*u as usize).cloned().unwrap_or_default(),
                    Function::Native(_) => Rc::default(),
                },
            })
            .collect()
    }
}

fn as_num_for(v: &Value) -> Option<f64> {
    v.to_num()
}

impl Lua {
    fn konst(&self, fref: GcRef, k: u32) -> Value {
        match self.heap.func(fref) {
            Function::Lua(c) => c.proto.consts.get(k as usize).cloned().unwrap_or_default(),
            Function::Native(_) => Value::Nil,
        }
    }

    fn env_of(&self, fref: GcRef) -> GcRef {
        match self.heap.func(fref) {
            Function::Lua(c) => c.env,
            Function::Native(c) => c.env,
        }
    }

    fn pop(&mut self) -> Value {
        self.th.stack.pop().unwrap_or_default()
    }

    fn top(&self) -> Value {
        self.th.stack.last().cloned().unwrap_or_default()
    }

    pub(crate) fn run(&mut self) -> LuaResult<()> {
        'frame: loop {
            let fi = self.th.frames.len() - 1;
            let (fref, base) = (self.th.frames[fi].func, self.th.frames[fi].base);
            let proto = match self.heap.func(fref) {
                Function::Lua(c) => c.proto.clone(),
                Function::Native(_) => return Err(self.rt_error("internal: native frame in the interpreter")),
            };
            let mut pc = self.th.frames[fi].pc;
            loop {
                let Some(&op) = proto.code.get(pc) else {
                    return Err(self.rt_error("internal: pc out of range"));
                };
                pc += 1;
                self.th.frames[fi].pc = pc;
                if self.budget > 0 {
                    self.executed += 1;
                    if self.executed > self.budget {
                        return Err(self.rt_error("instruction budget exhausted"));
                    }
                }
                let h = |i: usize| proto.hint(i);
                match op {
                    Op::Nil => self.th.stack.push(Value::Nil),
                    Op::Bool(b) => self.th.stack.push(Value::Bool(b)),
                    Op::Const(k) => self.th.stack.push(proto.consts[k as usize].clone()),
                    Op::Pop(n) => {
                        let l = self.th.stack.len() - n as usize;
                        self.th.stack.truncate(l);
                    }
                    Op::Dup(n) => {
                        let v = self.th.stack[self.th.stack.len() - 1 - n as usize].clone();
                        self.th.stack.push(v);
                    }
                    Op::GetLocal(s) => {
                        let i = base + s as usize;
                        let v = match &self.th.cells[i] {
                            Some(c) => c.borrow().clone(),
                            None => self.th.stack[i].clone(),
                        };
                        self.th.stack.push(v);
                    }
                    Op::SetLocal(s) => {
                        let v = self.pop();
                        let i = base + s as usize;
                        self.th.cells[i] = None;
                        self.th.stack[i] = v;
                    }
                    Op::NewCell(s) => {
                        let v = self.pop();
                        self.th.cells[base + s as usize] = Some(Rc::new(RefCell::new(v)));
                    }
                    Op::GetCell(s) => {
                        let v = match &self.th.cells[base + s as usize] {
                            Some(c) => c.borrow().clone(),
                            None => self.th.stack[base + s as usize].clone(),
                        };
                        self.th.stack.push(v);
                    }
                    Op::SetCell(s) => {
                        let v = self.pop();
                        match &self.th.cells[base + s as usize] {
                            Some(c) => *c.borrow_mut() = v,
                            None => self.th.stack[base + s as usize] = v,
                        }
                    }
                    Op::GetUpval(u) => {
                        let v = match self.heap.func(fref) {
                            Function::Lua(c) => c.upvals[u as usize].borrow().clone(),
                            Function::Native(_) => Value::Nil,
                        };
                        self.th.stack.push(v);
                    }
                    Op::SetUpval(u) => {
                        let v = self.pop();
                        if let Function::Lua(c) = self.heap.func(fref) {
                            *c.upvals[u as usize].borrow_mut() = v;
                        }
                    }
                    Op::GetGlobal(k) => {
                        let key = self.konst(fref, k);
                        let env = self.env_of(fref);
                        let v = self.index(&Value::Table(env), &key, None)?;
                        self.th.stack.push(v);
                    }
                    Op::SetGlobal(k) => {
                        let key = self.konst(fref, k);
                        let env = self.env_of(fref);
                        let v = self.pop();
                        self.set_index(&Value::Table(env), key, v, None)?;
                    }
                    Op::GetIndex => {
                        let k = self.pop();
                        let o = self.pop();
                        let v = self.index(&o, &k, h(pc - 1).0)?;
                        self.th.stack.push(v);
                    }
                    Op::GetField(k) => {
                        let key = proto.consts[k as usize].clone();
                        let o = self.pop();
                        let v = self.index(&o, &key, h(pc - 1).0)?;
                        self.th.stack.push(v);
                    }
                    Op::SetIndex => {
                        let v = self.pop();
                        let k = self.pop();
                        let o = self.pop();
                        self.set_index(&o, k, v, h(pc - 1).0)?;
                    }
                    Op::SetField(k) => {
                        let key = proto.consts[k as usize].clone();
                        let v = self.pop();
                        let o = self.pop();
                        self.set_index(&o, key, v, h(pc - 1).0)?;
                    }
                    Op::SetIndexDeep(d) => {
                        let v = self.pop();
                        let l = self.th.stack.len();
                        let o = self.th.stack[l - d as usize - 2].clone();
                        let k = self.th.stack[l - d as usize - 1].clone();
                        self.set_index(&o, k, v, h(pc - 1).0)?;
                    }
                    Op::NewTable(a, hsz) => {
                        self.maybe_collect();
                        let t = self.new_table(a as usize, hsz as usize);
                        self.th.stack.push(Value::Table(t));
                    }
                    Op::SetList { n, multi, first } => {
                        let count = n as usize + if multi { self.th.multret } else { 0 };
                        let l = self.th.stack.len();
                        let vals: Vec<Value> = self.th.stack.drain(l - count..).collect();
                        if let Value::Table(t) = self.top() {
                            let last = first as usize - 1 + count;
                            if last > self.heap.table(t).array_len() {
                                self.heap.table_mut(t).resize_array(last);
                            }
                            for (i, v) in vals.into_iter().enumerate() {
                                let _ = self.heap.table_mut(t).set(Value::Num((first as usize + i) as f64), v);
                            }
                        }
                    }
                    Op::SelfOp(k) => {
                        let key = proto.consts[k as usize].clone();
                        let o = self.pop();
                        let m = self.index(&o, &key, h(pc - 1).0)?;
                        self.th.stack.push(m);
                        self.th.stack.push(o);
                    }
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Pow => {
                        let b = self.pop();
                        let a = self.pop();
                        let r = match (&a, &b) {
                            (Value::Num(x), Value::Num(y)) => Value::Num(arith_of(op).apply(*x, *y)),
                            _ => {
                                let (da, db) = h(pc - 1);
                                self.arith(arith_of(op), &a, &b, da, db)?
                            }
                        };
                        self.th.stack.push(r);
                    }
                    Op::Unm => {
                        let a = self.pop();
                        let r = match a {
                            Value::Num(x) => Value::Num(-x),
                            _ => {
                                let d = h(pc - 1).0;
                                self.arith(Arith::Unm, &a, &a, d, d)?
                            }
                        };
                        self.th.stack.push(r);
                    }
                    Op::Not => {
                        let a = self.pop();
                        self.th.stack.push(Value::Bool(!a.truthy()));
                    }
                    Op::Len => {
                        let a = self.pop();
                        let r = self.length(&a, h(pc - 1).0)?;
                        self.th.stack.push(r);
                    }
                    Op::Concat(n) => {
                        self.maybe_collect();
                        let l = self.th.stack.len();
                        let vals: Vec<Value> = self.th.stack[l - n as usize..].to_vec();
                        let descs = proto
                            .concat_hints
                            .iter()
                            .find(|c| c.0 as usize == pc - 1)
                            .map(|c| c.1.clone())
                            .unwrap_or_default();
                        let r = self.concat(vals, &descs)?;
                        self.th.stack.truncate(l - n as usize);
                        self.th.stack.push(r);
                    }
                    Op::Eq | Op::Ne => {
                        let b = self.pop();
                        let a = self.pop();
                        let e = self.equals(&a, &b)?;
                        self.th.stack.push(Value::Bool(e == (op == Op::Eq)));
                    }
                    Op::Lt | Op::Gt => {
                        let b = self.pop();
                        let a = self.pop();
                        let r = if op == Op::Lt { self.less_than(&a, &b)? } else { self.less_than(&b, &a)? };
                        self.th.stack.push(Value::Bool(r));
                    }
                    Op::Le | Op::Ge => {
                        let b = self.pop();
                        let a = self.pop();
                        let r = if op == Op::Le { self.less_equal(&a, &b)? } else { self.less_equal(&b, &a)? };
                        self.th.stack.push(Value::Bool(r));
                    }
                    Op::Jump(o) => pc = (pc as i64 + o as i64) as usize,
                    Op::JumpIfFalse(o) => {
                        if !self.pop().truthy() {
                            pc = (pc as i64 + o as i64) as usize;
                        }
                    }
                    Op::JumpIfTrue(o) => {
                        if self.pop().truthy() {
                            pc = (pc as i64 + o as i64) as usize;
                        }
                    }
                    Op::AndJump(o) => {
                        if !self.top().truthy() {
                            pc = (pc as i64 + o as i64) as usize;
                        } else {
                            self.pop();
                        }
                    }
                    Op::OrJump(o) => {
                        if self.top().truthy() {
                            pc = (pc as i64 + o as i64) as usize;
                        } else {
                            self.pop();
                        }
                    }
                    Op::Call { args, multi, results } => {
                        let nargs = args as usize + if multi { self.th.multret } else { 0 };
                        let fidx = self.th.stack.len() - nargs - 1;
                        let name = h(pc - 1).0.cloned();
                        if self.precall(fidx, nargs, results, name, false)? {
                            continue 'frame;
                        }
                    }
                    Op::TailCall { args, multi } => {
                        let nargs = args as usize + if multi { self.th.multret } else { 0 };
                        let fidx = self.th.stack.len() - nargs - 1;
                        let name = h(pc - 1).0.cloned();
                        let is_lua = matches!(&self.th.stack[fidx], Value::Func(r) if matches!(self.heap.func(*r), Function::Lua(_)));
                        if is_lua {
                            let Some(cur) = self.th.frames.pop() else { return Ok(()) };
                            let moved: Vec<Value> = self.th.stack.drain(fidx..).collect();
                            self.th.stack.truncate(cur.ret);
                            self.th.cells.truncate(cur.ret);
                            self.th.stack.extend(moved);
                            if self.precall(cur.ret, nargs, cur.results, name, cur.boundary)? {
                                if let Some(f) = self.th.frames.last_mut() {
                                    f.tail = true;
                                }
                                continue 'frame;
                            }
                            return Ok(());
                        }
                        if self.precall(fidx, nargs, -1, name, false)? {
                            continue 'frame;
                        }
                    }
                    Op::Return { n, multi } => {
                        let count = n as usize + if multi { self.th.multret } else { 0 };
                        if self.poscall(count) {
                            return Ok(());
                        }
                        continue 'frame;
                    }
                    Op::Vararg(n) => {
                        let va = self.th.frames[fi].varargs.clone();
                        if n < 0 {
                            self.th.multret = va.len();
                            self.th.stack.extend(va);
                        } else {
                            for i in 0..n as usize {
                                self.th.stack.push(va.get(i).cloned().unwrap_or_default());
                            }
                        }
                    }
                    Op::Closure(i) => {
                        self.maybe_collect();
                        let p = proto.protos[i as usize].clone();
                        let ups = self.capture(&p.captures, base, fref);
                        let env = self.env_of(fref);
                        let c = self.new_closure(p, ups, env);
                        self.th.stack.push(c);
                    }
                    Op::ForPrep { base: b, jump } => {
                        let b = base + b as usize;
                        let msgs = [
                            "'for' initial value must be a number",
                            "'for' limit must be a number",
                            "'for' step must be a number",
                        ];
                        for (i, msg) in msgs.iter().enumerate() {
                            match as_num_for(&self.th.stack[b + i]) {
                                Some(x) => self.th.stack[b + i] = Value::Num(x),
                                None => return Err(self.rt_error(msg)),
                            }
                        }
                        let (Value::Num(init), Value::Num(step)) = (&self.th.stack[b], &self.th.stack[b + 2]) else {
                            return Err(self.rt_error("internal: for"));
                        };
                        self.th.stack[b] = Value::Num(init - step);
                        pc = (pc as i64 + jump as i64) as usize;
                    }
                    Op::ForLoop { base: b, jump } => {
                        let b = base + b as usize;
                        let (Value::Num(idx), Value::Num(limit), Value::Num(step)) =
                            (&self.th.stack[b], &self.th.stack[b + 1], &self.th.stack[b + 2])
                        else {
                            return Err(self.rt_error("internal: for"));
                        };
                        let idx = idx + step;
                        let go = if 0.0 < *step { idx <= *limit } else { *limit <= idx };
                        if go {
                            self.th.stack[b] = Value::Num(idx);
                            pc = (pc as i64 + jump as i64) as usize;
                        }
                    }
                    Op::TForCall { base: b, nvars } => {
                        let b = base + b as usize;
                        let f = self.th.stack[b].clone();
                        let args = [self.th.stack[b + 1].clone(), self.th.stack[b + 2].clone()];
                        let mut r = self.call(f, &args)?;
                        r.resize(nvars as usize, Value::Nil);
                        for (i, v) in r.into_iter().enumerate() {
                            self.th.cells[b + 3 + i] = None;
                            self.th.stack[b + 3 + i] = v;
                        }
                    }
                    Op::TForLoop { base: b, jump } => {
                        let b = base + b as usize;
                        let v = self.th.stack[b + 3].clone();
                        if !v.is_nil() {
                            self.th.stack[b + 2] = v;
                            pc = (pc as i64 + jump as i64) as usize;
                        }
                    }
                }
            }
        }
    }
}

fn arith_of(op: Op) -> Arith {
    match op {
        Op::Add => Arith::Add,
        Op::Sub => Arith::Sub,
        Op::Mul => Arith::Mul,
        Op::Div => Arith::Div,
        Op::Mod => Arith::Mod,
        _ => Arith::Pow,
    }
}
