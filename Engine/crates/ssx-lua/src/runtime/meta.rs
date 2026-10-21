use crate::bytecode::VarDesc;
use crate::number;
use crate::value::{Str, Value};
use crate::vm::{Lua, LuaError, LuaResult};

const MAXTAGLOOP: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Unm,
}

impl Arith {
    fn event(self) -> &'static str {
        match self {
            Arith::Add => "__add",
            Arith::Sub => "__sub",
            Arith::Mul => "__mul",
            Arith::Div => "__div",
            Arith::Mod => "__mod",
            Arith::Pow => "__pow",
            Arith::Unm => "__unm",
        }
    }

    pub fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            Arith::Add => a + b,
            Arith::Sub => a - b,
            Arith::Mul => a * b,
            Arith::Div => a / b,
            Arith::Mod => a - (a / b).floor() * b,
            Arith::Pow => a.powf(b),
            Arith::Unm => -a,
        }
    }
}

impl Lua {
    pub fn op_error(&self, op: &str, v: &Value, desc: Option<&VarDesc>) -> LuaError {
        match desc {
            Some(d) => self.rt_error(&format!(
                "attempt to {op} {} '{}' (a {} value)",
                d.kind(),
                d.name().to_str_lossy(),
                v.type_name()
            )),
            None => self.rt_error(&format!("attempt to {op} a {} value", v.type_name())),
        }
    }

    pub fn index(&mut self, t: &Value, k: &Value, desc: Option<&VarDesc>) -> LuaResult<Value> {
        let mut t = t.clone();
        for _ in 0..MAXTAGLOOP {
            let tm = if let Value::Table(h) = t {
                let v = self.heap.table(h).get(k);
                if !v.is_nil() {
                    return Ok(v);
                }
                match self.heap.table(h).meta {
                    None => return Ok(Value::Nil),
                    Some(m) => {
                        let tm = self.heap.table(m).get_str("__index");
                        if tm.is_nil() {
                            return Ok(Value::Nil);
                        }
                        tm
                    }
                }
            } else {
                let tm = self.metamethod(&t, "__index");
                if tm.is_nil() {
                    return Err(self.op_error("index", &t, desc));
                }
                tm
            };
            if let Value::Func(_) = tm {
                return Ok(self.call(tm, &[t, k.clone()])?.into_iter().next().unwrap_or_default());
            }
            t = tm;
        }
        Err(self.rt_error("loop in gettable"))
    }

    pub fn set_index(&mut self, t: &Value, k: Value, v: Value, desc: Option<&VarDesc>) -> LuaResult<()> {
        let mut t = t.clone();
        for _ in 0..MAXTAGLOOP {
            let tm = if let Value::Table(h) = t {
                let old = self.heap.table(h).get(&k);
                let tm = match self.heap.table(h).meta {
                    Some(m) if old.is_nil() => self.heap.table(m).get_str("__newindex"),
                    _ => Value::Nil,
                };
                if tm.is_nil() {
                    return self.raw_set(h, k, v);
                }
                self.raw_set(h, k.clone(), Value::Nil)?;
                tm
            } else {
                let tm = self.metamethod(&t, "__newindex");
                if tm.is_nil() {
                    return Err(self.op_error("index", &t, desc));
                }
                tm
            };
            if let Value::Func(_) = tm {
                self.call(tm, &[t, k, v])?;
                return Ok(());
            }
            t = tm;
        }
        Err(self.rt_error("loop in settable"))
    }

    fn bin_tm(&mut self, a: &Value, b: &Value, event: &str) -> LuaResult<Option<Value>> {
        let mut tm = self.metamethod(a, event);
        if tm.is_nil() {
            tm = self.metamethod(b, event);
        }
        if tm.is_nil() {
            return Ok(None);
        }
        Ok(Some(self.call(tm, &[a.clone(), b.clone()])?.into_iter().next().unwrap_or_default()))
    }

    pub fn arith(
        &mut self,
        op: Arith,
        a: &Value,
        b: &Value,
        da: Option<&VarDesc>,
        db: Option<&VarDesc>,
    ) -> LuaResult<Value> {
        if let (Some(x), Some(y)) = (a.to_num(), b.to_num()) {
            return Ok(Value::Num(op.apply(x, y)));
        }
        if let Some(r) = self.bin_tm(a, b, op.event())? {
            return Ok(r);
        }
        if a.to_num().is_none() {
            Err(self.op_error("perform arithmetic on", a, da))
        } else {
            Err(self.op_error("perform arithmetic on", b, db))
        }
    }

    pub fn equals(&mut self, a: &Value, b: &Value) -> LuaResult<bool> {
        let (ma, mb) = match (a, b) {
            (Value::Table(x), Value::Table(y)) if x != y => (self.heap.table(*x).meta, self.heap.table(*y).meta),
            (Value::UserData(x), Value::UserData(y)) if x != y => {
                (self.heap.userdata(*x).meta, self.heap.userdata(*y).meta)
            }
            _ => return Ok(a.raw_eq(b)),
        };
        let Some(m1) = ma else { return Ok(false) };
        let tm1 = self.heap.table(m1).get_str("__eq");
        if tm1.is_nil() {
            return Ok(false);
        }
        if Some(m1) != mb {
            let Some(m2) = mb else { return Ok(false) };
            let tm2 = self.heap.table(m2).get_str("__eq");
            if !tm1.raw_eq(&tm2) {
                return Ok(false);
            }
        }
        Ok(self.call(tm1, &[a.clone(), b.clone()])?.first().is_some_and(Value::truthy))
    }

    fn order_error(&self, a: &Value, b: &Value) -> LuaError {
        let (t1, t2) = (a.type_name(), b.type_name());
        if t1.as_bytes()[2] == t2.as_bytes()[2] {
            self.rt_error(&format!("attempt to compare two {t1} values"))
        } else {
            self.rt_error(&format!("attempt to compare {t1} with {t2}"))
        }
    }

    fn order_tm(&mut self, a: &Value, b: &Value, event: &str) -> LuaResult<Option<bool>> {
        let tm1 = self.metamethod(a, event);
        if tm1.is_nil() {
            return Ok(None);
        }
        let tm2 = self.metamethod(b, event);
        if !tm1.raw_eq(&tm2) {
            return Ok(None);
        }
        Ok(Some(self.call(tm1, &[a.clone(), b.clone()])?.first().is_some_and(Value::truthy)))
    }

    pub fn less_than(&mut self, a: &Value, b: &Value) -> LuaResult<bool> {
        match (a, b) {
            (Value::Num(x), Value::Num(y)) => Ok(x < y),
            // TODO(guess): `l_strcmp` uses `strcoll`; the game is assumed to stay in the "C" locale (byte order); settle: look for `setlocale` calls in the exe.
            (Value::Str(x), Value::Str(y)) => Ok(x.bytes() < y.bytes()),
            _ if std::mem::discriminant(a) != std::mem::discriminant(b) => Err(self.order_error(a, b)),
            _ => match self.order_tm(a, b, "__lt")? {
                Some(r) => Ok(r),
                None => Err(self.order_error(a, b)),
            },
        }
    }

    pub fn less_equal(&mut self, a: &Value, b: &Value) -> LuaResult<bool> {
        match (a, b) {
            (Value::Num(x), Value::Num(y)) => Ok(x <= y),
            (Value::Str(x), Value::Str(y)) => Ok(x.bytes() <= y.bytes()),
            _ if std::mem::discriminant(a) != std::mem::discriminant(b) => Err(self.order_error(a, b)),
            _ => {
                if let Some(r) = self.order_tm(a, b, "__le")? {
                    return Ok(r);
                }
                match self.order_tm(b, a, "__lt")? {
                    Some(r) => Ok(!r),
                    None => Err(self.order_error(a, b)),
                }
            }
        }
    }

    pub fn length(&mut self, v: &Value, desc: Option<&VarDesc>) -> LuaResult<Value> {
        match v {
            Value::Table(t) => Ok(Value::Num(self.heap.table(*t).length() as f64)),
            Value::Str(s) => Ok(Value::Num(s.bytes().len() as f64)),
            _ => match self.bin_tm(v, &Value::Nil, "__len")? {
                Some(r) => Ok(r),
                None => Err(self.op_error("get length of", v, desc)),
            },
        }
    }

    pub fn concat(&mut self, mut vals: Vec<Value>, descs: &[Option<VarDesc>]) -> LuaResult<Value> {
        let is_sn = |v: &Value| matches!(v, Value::Str(_) | Value::Num(_));
        let root = self.th.stack.len();
        while vals.len() > 1 {
            let top = vals.len();
            let (a, b) = (vals[top - 2].clone(), vals[top - 1].clone());
            if !is_sn(&a) || !is_sn(&b) {
                match self.bin_tm(&a, &b, "__concat")? {
                    Some(r) => {
                        self.th.stack.push(r.clone());
                        vals.truncate(top - 2);
                        vals.push(r);
                        continue;
                    }
                    None => {
                        let (bad, i) = if is_sn(&a) { (&b, top - 1) } else { (&a, top - 2) };
                        return Err(self.op_error("concatenate", bad, descs.get(i).and_then(Option::as_ref)));
                    }
                }
            }
            let mut n = 1;
            while n < top && is_sn(&vals[top - n - 1]) {
                n += 1;
            }
            let mut buf = Vec::new();
            for v in &vals[top - n..] {
                if let Some(s) = v.to_str_coerce() {
                    buf.extend_from_slice(s.bytes());
                }
            }
            vals.truncate(top - n);
            vals.push(Value::Str(Str::new(&buf)));
        }
        self.th.stack.truncate(root);
        Ok(vals.pop().unwrap_or_default())
    }

    pub fn tostring(&mut self, v: &Value) -> LuaResult<Str> {
        let tm = self.metamethod(v, "__tostring");
        if !tm.is_nil() {
            let r = self.call(tm, std::slice::from_ref(v))?.into_iter().next().unwrap_or_default();
            return match r {
                Value::Str(s) => Ok(s),
                Value::Num(n) => Ok(Str::from(number::tostring(n).as_str())),
                _ => Err(self.rt_error("'__tostring' must return a string")),
            };
        }
        Ok(Str::from(self.default_tostring(v).as_str()))
    }

    // TODO(guess): `%p` is assumed to print eight upper-case hex digits (Microsoft runtime); the value is our object index, not an address, so it never matches the game's text; settle: not reproducible (an address).
    pub fn default_tostring(&self, v: &Value) -> String {
        match v {
            Value::Nil => "nil".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => number::tostring(*n),
            Value::Str(s) => s.to_str_lossy(),
            Value::LightUserData(p) => format!("userdata: {:08X}", *p as u32),
            other => {
                let r = other.gc().map_or(0, |g| 0x4000_0000 | (g.0 << 5));
                format!("{}: {r:08X}", other.type_name())
            }
        }
    }
}
