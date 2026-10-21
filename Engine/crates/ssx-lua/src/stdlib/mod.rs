pub mod base;
pub mod coroutine;
pub mod debug;
pub mod math;
pub mod os;
pub mod package;
pub mod pattern;
pub mod string;
pub mod tablib;

use crate::number::to_int;
use crate::value::{GcRef, Str, Value};
use crate::vm::{Lua, LuaResult};

pub fn arg(args: &[Value], i: usize) -> Option<&Value> {
    args.get(i - 1)
}

fn tname(v: Option<&Value>) -> &'static str {
    v.map_or("no value", Value::type_name)
}

impl Lua {
    pub fn tag_error(&self, i: usize, expected: &str, got: Option<&Value>) -> crate::LuaError {
        self.arg_error(i, &format!("{expected} expected, got {}", tname(got)))
    }

    pub fn check_any<'a>(&self, args: &'a [Value], i: usize) -> LuaResult<&'a Value> {
        arg(args, i).ok_or_else(|| self.arg_error(i, "value expected"))
    }

    pub fn check_num(&self, args: &[Value], i: usize) -> LuaResult<f64> {
        let v = arg(args, i);
        v.and_then(Value::to_num).ok_or_else(|| self.tag_error(i, "number", v))
    }

    pub fn check_int(&self, args: &[Value], i: usize) -> LuaResult<i32> {
        self.check_num(args, i).map(to_int)
    }

    pub fn opt_num(&self, args: &[Value], i: usize, def: f64) -> LuaResult<f64> {
        match arg(args, i) {
            None | Some(Value::Nil) => Ok(def),
            _ => self.check_num(args, i),
        }
    }

    pub fn opt_int(&self, args: &[Value], i: usize, def: i32) -> LuaResult<i32> {
        match arg(args, i) {
            None | Some(Value::Nil) => Ok(def),
            _ => self.check_int(args, i),
        }
    }

    pub fn check_str(&self, args: &[Value], i: usize) -> LuaResult<Str> {
        let v = arg(args, i);
        v.and_then(Value::to_str_coerce).ok_or_else(|| self.tag_error(i, "string", v))
    }

    pub fn opt_str(&self, args: &[Value], i: usize, def: &str) -> LuaResult<Str> {
        match arg(args, i) {
            None | Some(Value::Nil) => Ok(Str::from(def)),
            _ => self.check_str(args, i),
        }
    }

    pub fn check_table(&self, args: &[Value], i: usize) -> LuaResult<GcRef> {
        match arg(args, i) {
            Some(Value::Table(t)) => Ok(*t),
            v => Err(self.tag_error(i, "table", v)),
        }
    }

    pub fn check_func(&self, args: &[Value], i: usize) -> LuaResult<Value> {
        match arg(args, i) {
            Some(v @ Value::Func(_)) => Ok(v.clone()),
            v => Err(self.tag_error(i, "function", v)),
        }
    }

    pub fn lib_fn(&mut self, t: GcRef, name: &str, f: impl Fn(&mut Lua, &[Value]) -> LuaResult<Vec<Value>> + 'static) {
        let v = self.create_function(name, f);
        let _ = self.heap.table_mut(t).set(Value::str(name), v);
    }

    pub fn lib_table(&mut self, name: &str) -> GcRef {
        let loaded = self.loaded_table();
        if let Some(l) = loaded
            && let Value::Table(t) = self.heap.table(l).get_str(name)
        {
            return t;
        }
        let t = match self.get_global(name) {
            Value::Table(t) => t,
            _ => {
                let t = self.new_table(0, 0);
                self.set_global(name, Value::Table(t));
                t
            }
        };
        if let Some(l) = loaded {
            let _ = self.heap.table_mut(l).set(Value::str(name), Value::Table(t));
        }
        t
    }

    pub fn loaded_table(&mut self) -> Option<GcRef> {
        let r = self.registry;
        match self.heap.table(r).get_str("_LOADED") {
            Value::Table(t) => Some(t),
            _ => {
                let t = self.new_table(0, 0);
                let _ = self.heap.table_mut(r).set(Value::str("_LOADED"), Value::Table(t));
                Some(t)
            }
        }
    }
}

pub fn one(v: impl Into<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![v.into()])
}

pub fn none() -> LuaResult<Vec<Value>> {
    Ok(Vec::new())
}
