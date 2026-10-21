use super::*;

impl Ctx<'_> {
    // TODO(guess): creation order (array elements first, then the chain); settle: ActionEnumerate2's hash-table order (0x82771090 case 0x55).
    pub fn enumerate(&mut self, obj: &Value) -> Vec<Str> {
        let Value::Object(id) = obj else { return Vec::new() };
        let id = self.resolve_handle(*id);
        let mut out: Vec<Str> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        if let Some(Kind::Array(a)) = self.vm.heap.get(id).map(|o| &o.kind) {
            for i in 0..a.len() {
                let k = Str::from(i.to_string());
                seen.insert(k.clone());
                out.push(k);
            }
        }
        if let Some(Kind::Display(d)) = self.vm.heap.get(id).map(|o| &o.kind) {
            let inst = d.id;
            for (_, c) in self.player.children(inst) {
                if let Some(n) = self.player.get(c).map(|i| i.name.clone()).filter(|n| !n.is_empty()) {
                    let k = Str::from(n.as_str());
                    if seen.insert(k.clone()) {
                        out.push(k);
                    }
                }
            }
        }
        let mut cur = Some(id);
        for _ in 0..MAX_CHAIN {
            let Some(c) = cur else { break };
            let Some(o) = self.vm.heap.get(c) else { break };
            for (k, p) in o.props.iter() {
                if seen.insert(k.clone()) && p.flags & flags::DONT_ENUM == 0 {
                    out.push(k.clone());
                }
            }
            cur = o.proto.as_object();
        }
        out
    }

    pub fn type_of(&mut self, v: &Value) -> &'static str {
        match v {
            Value::Undefined => "undefined",
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Object(o) => match self.vm.heap.get(*o).map(|o| &o.kind) {
                Some(Kind::Function(_)) => "function",
                Some(Kind::Display(d)) => {
                    let id = d.id;
                    match self.display_of(v).and_then(|i| self.player.get(i)) {
                        Some(i) if i.sprite().is_some() => "movieclip",
                        Some(_) => "object",
                        None => {
                            let _ = id;
                            "movieclip"
                        }
                    }
                }
                _ => "object",
            },
        }
    }

    pub fn instance_of(&mut self, obj: &Value, ctor: &Value) -> bool {
        let Value::Object(_) = obj else { return false };
        let Ok(proto) = self.get_member(ctor, "prototype") else { return false };
        let Some(proto) = proto.as_object() else { return false };
        let Some(ctor_id) = ctor.as_object() else { return false };
        let mut cur = self.proto_of(obj).as_object();
        for _ in 0..MAX_CHAIN {
            let Some(c) = cur else { break };
            if c == proto {
                return true;
            }
            let ifaces = self.vm.heap.get(c).map(|o| o.interfaces.clone()).unwrap_or_default();
            if self.implements(&ifaces, ctor_id, 0) {
                return true;
            }
            cur = self.vm.heap.get(c).and_then(|o| o.proto.as_object());
        }
        false
    }

    pub(super) fn implements(&mut self, ifaces: &[ObjId], ctor: ObjId, depth: usize) -> bool {
        if depth > 16 {
            return false;
        }
        for &i in ifaces {
            if i == ctor {
                return true;
            }
            let p = self.get_member(&Value::Object(i), "prototype").ok().and_then(|p| p.as_object());
            let sub = p.and_then(|p| self.vm.heap.get(p)).map(|o| o.interfaces.clone()).unwrap_or_default();
            if self.implements(&sub, ctor, depth + 1) {
                return true;
            }
        }
        false
    }

    pub fn add2(&mut self, a: &Value, b: &Value) -> Result<Value, Fault> {
        let pa = self.add_operand(a)?;
        let pb = self.add_operand(b)?;
        if matches!(pa, Value::String(_)) || matches!(pb, Value::String(_)) {
            let s = format!("{}{}", self.to_string(&pa)?, self.to_string(&pb)?);
            return Ok(s.into());
        }
        Ok(Value::Number(self.to_number(&pa) + self.to_number(&pb)))
    }

    pub(super) fn add_operand(&mut self, v: &Value) -> Result<Value, Fault> {
        match self.kind_of(v) {
            Some(Kind::Display(_)) => Ok(Value::String(self.to_string(v)?)),
            Some(Kind::Date(_)) => Ok(Value::String(self.to_string(v)?)),
            Some(_) => match self.to_primitive(v, false)? {
                Value::Object(_) => Ok(Value::String(self.to_string(v)?)),
                p => Ok(p),
            },
            None => Ok(v.clone()),
        }
    }
}

impl Ctx<'_> {
    pub(super) fn is_display(&self, v: &Value) -> bool {
        matches!(self.kind_of(v), Some(Kind::Display(_)))
    }

    pub fn equals2(&mut self, a: &Value, b: &Value) -> Result<bool, Fault> {
        let (na, nb) = (a.is_nullish(), b.is_nullish());
        if na || nb {
            return Ok(na && nb);
        }
        match a {
            Value::Bool(_) => {
                let n = Value::Number(self.to_number(a));
                self.equals2(&n, b)
            }
            Value::Number(x) => {
                if matches!(b, Value::Object(_)) && !self.is_display(b) {
                    return match self.to_primitive(b, false)? {
                        Value::Object(_) => Ok(false),
                        p => self.equals2(a, &p),
                    };
                }
                let y = self.to_number(b);
                Ok(if x.is_nan() || y.is_nan() { x.is_nan() && y.is_nan() } else { *x == y })
            }
            Value::String(s) => match b {
                Value::Number(_) => {
                    let n = Value::Number(self.to_number(a));
                    self.equals2(&n, b)
                }
                Value::Bool(_) => {
                    let n = Value::Number(self.to_number(b));
                    self.equals2(a, &n)
                }
                Value::Object(_) if !self.is_display(b) => match self.to_primitive(b, false)? {
                    Value::Object(_) => Ok(false),
                    p => self.equals2(a, &p),
                },
                _ => {
                    let t = self.to_string(b)?;
                    Ok(*s == t)
                }
            },
            Value::Object(x) => {
                if self.is_display(a) {
                    return Ok(match b {
                        Value::Object(_) if self.is_display(b) => {
                            let (p, q) = (self.display_of(a), self.display_of(b));
                            p.is_some() && p == q || *x == b.as_object().unwrap_or(ObjId(u32::MAX))
                        }
                        _ => false,
                    });
                }
                match b {
                    Value::Object(y) => Ok(x == y),
                    _ => match self.to_primitive(a, false)? {
                        Value::Object(_) => Ok(false),
                        p => self.equals2(&p, b),
                    },
                }
            }
            Value::Undefined | Value::Null => Ok(false),
        }
    }

    pub fn strict_equals(&mut self, a: &Value, b: &Value) -> bool {
        if self.is_display(a) && self.is_display(b) {
            let (p, q) = (self.display_of(a), self.display_of(b));
            return p.is_some() && p == q || a.same(b);
        }
        a.same(b)
    }

    pub fn compare(&mut self, a: &Value, b: &Value, greater: bool) -> Result<Value, Fault> {
        let pa = self.to_primitive(a, false)?;
        let pb = self.to_primitive(b, false)?;
        if let (Value::String(x), Value::String(y)) = (&pa, &pb) {
            return Ok(Value::Bool(if greater { x > y } else { x < y }));
        }
        if self.version() > 6 && (pa.is_undefined() || pb.is_undefined()) {
            return Ok(Value::Undefined);
        }
        let (x, y) = (self.to_number(&pa), self.to_number(&pb));
        let (l, r) = if greater { (y, x) } else { (x, y) };
        if l.is_nan() || r.is_nan() {
            return Ok(Value::Undefined);
        }
        Ok(Value::Bool(l < r))
    }

    pub fn call_checked(
        &mut self,
        func: &Value,
        this: Value,
        args: &[Value],
        home: Value,
        name: &str,
    ) -> Result<Value, Fault> {
        if self.is_function(func) {
            self.vm.diag.calls += 1;
            self.vm.call_names.push(name.into());
            let r = self.call(func, this, args, home);
            self.vm.call_names.pop();
            return r;
        }
        match func {
            Value::Undefined => self.vm.diag.note("undefined function", name),
            _ => self.vm.diag.note("not a function", name),
        }
        Ok(Value::Undefined)
    }

    pub fn construct(&mut self, ctor: &Value, args: &[Value]) -> Result<Value, Fault> {
        let proto = self.get_member(ctor, "prototype")?;
        let obj = self.vm.heap.alloc(Object::new(proto.clone(), Kind::Plain));
        self.define(obj, "__constructor__", ctor.clone(), flags::DONT_ENUM);
        self.vm.next_construct = true;
        let r = self.call(ctor, Value::Object(obj), args, proto);
        self.vm.next_construct = false;
        r?;
        Ok(Value::Object(obj))
    }

    pub fn construct_checked(&mut self, ctor: &Value, args: &[Value], name: &str) -> Result<Value, Fault> {
        if self.is_function(ctor) {
            self.vm.diag.calls += 1;
            return self.construct(ctor, args);
        }
        match ctor {
            Value::Undefined => self.vm.diag.note("undefined class", name),
            _ => self.vm.diag.note("not a constructor", name),
        }
        Ok(Value::Undefined)
    }

    pub fn note_missing(&mut self, obj: &Value, name: &str) {
        match obj {
            Value::Undefined | Value::Null => {
                let mut d = name.to_string();
                if self.vm.debug_stacks {
                    let n = self.vm.call_names.len();
                    d += &format!(" ← {}", self.vm.call_names[n.saturating_sub(6)..].join(" ← "));
                }
                self.vm.diag.note("method of undefined", &d)
            }
            _ => {
                let (class, _) = self.class_name(obj);
                let kind = if crate::avm1::builtins::API_METHODS.contains(&name) {
                    "missing built-in method"
                } else {
                    "method not on the object"
                };
                let mut detail = format!("{class}.{name}");
                if self.vm.debug_stacks {
                    if let Some(id) = self.display_of(obj) {
                        let ch = self.player.get(id).and_then(|i| i.character);
                        detail += &format!(" on {} {ch:?}", self.player.path(id));
                    }
                    let n = self.vm.call_names.len();
                    detail += &format!(" ← {}", self.vm.call_names[n.saturating_sub(6)..].join(" ← "));
                }
                self.vm.diag.note(kind, &detail);
            }
        }
    }

    pub fn class_name(&self, v: &Value) -> (&'static str, bool) {
        match v {
            Value::Bool(_) => ("Boolean", true),
            Value::Number(_) => ("Number", true),
            Value::String(_) => ("String", true),
            Value::Object(o) => {
                if let Some(Kind::Function(_)) = self.vm.heap.get(*o).map(|o| &o.kind) {
                    return ("Function", true);
                }
                let mut cur = self.vm.heap.get(*o).and_then(|x| x.proto.as_object());
                let mut first = true;
                for _ in 0..MAX_CHAIN {
                    let Some(c) = cur else { break };
                    if let Some(n) = self.vm.class_names.get(&c) {
                        return (n, first || *n != "Object");
                    }
                    first = false;
                    cur = self.vm.heap.get(c).and_then(|x| x.proto.as_object());
                }
                ("Object", false)
            }
            _ => ("undefined", true),
        }
    }
}
