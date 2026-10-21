use std::sync::Arc;

use ssx_formats::gfx::avm1::{Action, ActionData, PushValue};

use super::object::{AvmFunction, Func, FuncCode, Kind, ObjId, Object, Scope, ScopeKind, flags};
use super::value::{Str, Value, to_int32};
use super::{Ctx, Fault};
use crate::library::MovieId;

mod calls;
mod ops;

pub const MAX_CALL_DEPTH: u32 = 255;

pub struct Frame {
    pub code: Arc<[Action]>,
    pub pc: usize,
    pub pool: Arc<Vec<Str>>,
    pub scope: Arc<Scope>,
    pub regs: Vec<Value>,
    pub this: Value,
    pub target: Option<ObjId>,
    pub version: u8,
    pub movie: MovieId,
    pub callee: Value,
    pub home: Value,
    pub withs: Vec<(usize, Arc<Scope>)>,
}

pub enum Flow {
    Exit(usize),
    Return(Value),
    End,
}

fn block_len(code: &[Action]) -> u32 {
    code.last().map_or(0, |a| a.offset + 1)
}

pub fn index_of(code: &[Action], offset: u32) -> Option<usize> {
    if offset == block_len(code) {
        return Some(code.len());
    }
    code.binary_search_by_key(&offset, |a| a.offset).ok()
}

fn offset_at(code: &[Action], i: usize) -> u32 {
    code.get(i).map_or_else(|| block_len(code), |a| a.offset)
}

impl Ctx<'_> {
    pub fn push(&mut self, v: Value) {
        self.vm.stack.push(v);
    }

    pub fn pop(&mut self) -> Value {
        match self.vm.stack.pop() {
            Some(v) => v,
            None => {
                let what = format!(
                    "{} at {:#x} in {}",
                    ssx_formats::gfx::avm1::op_name(self.vm.cur_op.0).unwrap_or("?"),
                    self.vm.cur_op.1,
                    self.lib.movie(self.vm.cur_movie).path
                );
                self.vm.diag.note("stack underflow", &what);
                Value::Undefined
            }
        }
    }

    fn pop_n(&mut self, n: usize) -> Vec<Value> {
        (0..n).map(|_| self.pop()).collect()
    }

    fn pop_count(&mut self) -> usize {
        let v = self.pop();
        let n = self.to_number(&v);
        if n.is_nan() || n < 0.0 { 0 } else { (n as usize).min(self.vm.stack.len()) }
    }

    pub fn run(&mut self, f: &mut Frame, start: usize, end: usize) -> Result<Flow, Fault> {
        f.pc = start;
        loop {
            while let Some(&(e, _)) = f.withs.last() {
                if f.pc < e {
                    break;
                }
                if let Some((_, s)) = f.withs.pop() {
                    f.scope = s;
                }
            }
            if f.pc < start || f.pc >= end {
                return Ok(Flow::Exit(f.pc));
            }
            self.vm.budget = self.vm.budget.saturating_sub(1);
            if self.vm.budget == 0 {
                return Err(Fault::Abort("script time-out (action budget spent)"));
            }
            self.vm.diag.actions += 1;
            let code = f.code.clone();
            let a = &code[f.pc];
            self.vm.cur_op = (a.op, a.offset);
            self.vm.diag.count_op(a.op);
            f.pc += 1;
            if let Some(flow) = self.step(f, a)? {
                return Ok(flow);
            }
        }
    }

    fn branch(&mut self, f: &mut Frame, delta: i16) -> Result<(), Fault> {
        let next = i64::from(offset_at(&f.code, f.pc));
        let t = next + i64::from(delta);
        if t < 0 {
            return Err(Fault::Abort("branch before the start of the block"));
        }
        if t >= i64::from(block_len(&f.code)) {
            f.pc = f.code.len();
            return Ok(());
        }
        match index_of(&f.code, t as u32) {
            Some(i) => {
                f.pc = i;
                Ok(())
            }
            None => Err(Fault::Abort("branch off an action boundary")),
        }
    }

    fn constant(&mut self, f: &Frame, i: u16) -> Value {
        match f.pool.get(usize::from(i)) {
            Some(s) => Value::String(s.clone()),
            None => Value::Undefined,
        }
    }

    fn register(&mut self, f: &Frame, r: u8) -> Value {
        match f.regs.get(usize::from(r)) {
            Some(v) => v.clone(),
            None => Value::Undefined,
        }
    }
}
