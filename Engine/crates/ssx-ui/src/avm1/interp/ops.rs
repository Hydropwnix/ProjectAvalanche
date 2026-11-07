use super::*;

impl Ctx<'_> {
    pub(super) fn step(&mut self, f: &mut Frame, act: &Action) -> Result<Option<Flow>, Fault> {
        match act.op {
            0x00 => return Ok(Some(Flow::End)),
            0x04 | 0x05 => self.frame_step(f, act.op == 0x04),
            0x06 | 0x07 => self.frame_play(f, act.op == 0x06),
            0x08 | 0x09 => {}
            0x0A | 0x47 => {
                let b = self.pop();
                let a = self.pop();
                let r = self.add2(&a, &b)?;
                self.push(r);
            }
            0x0B..=0x0D | 0x3F => {
                let b = self.pop();
                let a = self.pop();
                let (x, y) = (self.to_number(&a), self.to_number(&b));
                let r = match act.op {
                    0x0B => x - y,
                    0x0C => x * y,
                    0x0D => x / y,
                    _ => x % y,
                };
                self.push(Value::Number(r));
            }
            0x0E | 0x49 => {
                let b = self.pop();
                let a = self.pop();
                let r = if act.op == 0x0E {
                    let (x, y) = (self.to_number(&a), self.to_number(&b));
                    x == y
                } else {
                    self.equals2(&a, &b)?
                };
                self.push(Value::Bool(r));
            }
            0x0F | 0x48 | 0x67 => {
                let b = self.pop();
                let a = self.pop();
                let r = self.compare(&a, &b, act.op == 0x67)?;
                self.push(r);
            }
            0x10 | 0x11 => {
                let b = self.pop();
                let a = self.pop();
                let (x, y) = (self.to_bool(&a), self.to_bool(&b));
                self.push(Value::Bool(if act.op == 0x10 { x && y } else { x || y }));
            }
            0x12 => {
                let v = self.pop();
                let b = self.to_bool(&v);
                self.push(Value::Bool(!b));
            }
            0x13 | 0x29 | 0x68 => {
                let b = self.pop();
                let a = self.pop();
                let (x, y) = (self.to_string(&a)?, self.to_string(&b)?);
                self.push(Value::Bool(match act.op {
                    0x13 => x == y,
                    0x29 => x < y,
                    _ => x > y,
                }));
            }
            0x14 | 0x31 => {
                let v = self.pop();
                let s = self.to_string(&v)?;
                self.push(Value::Number(s.chars().count() as f64));
            }
            0x15 | 0x35 => {
                let n = self.pop();
                let i = self.pop();
                let s = self.pop();
                let s = self.to_string(&s)?;
                let (n, i) = (to_int32(self.to_number(&n)), to_int32(self.to_number(&i)));
                let r: String = s.chars().skip((i - 1).max(0) as usize).take(n.max(0) as usize).collect();
                self.push(r.into());
            }
            0x17 => {
                if self.vm.stack.pop().is_none() {
                    self.vm.diag.empty_pops += 1;
                }
            }
            0x18 => {
                let v = self.pop();
                let n = self.to_number(&v);
                self.push(Value::Number(f64::from(to_int32(n))));
            }
            0x1C => {
                let n = self.pop();
                let name = self.to_string(&n)?;
                let v = self.get_variable(f, &name)?;
                self.push(v);
            }
            0x1D => {
                let v = self.pop();
                let n = self.pop();
                let name = self.to_string(&n)?;
                self.set_variable(f, &name, v)?;
            }
            0x20 => {
                let t = self.pop();
                self.set_target_value(f, &t)?;
            }
            0x21 => {
                let b = self.pop();
                let a = self.pop();
                let (x, y) = (self.to_string(&a)?, self.to_string(&b)?);
                self.made_string(x.len() + y.len())?;
                self.push(format!("{x}{y}").into());
            }
            0x22 => {
                let i = self.pop();
                let t = self.pop();
                let v = self.get_property_action(f, &t, &i)?;
                self.push(v);
            }
            0x23 => {
                let v = self.pop();
                let i = self.pop();
                let t = self.pop();
                self.set_property_action(f, &t, &i, v)?;
            }
            0x24 => {
                let depth = self.pop();
                let name = self.pop();
                let src = self.pop();
                self.clone_sprite(f, &src, &name, &depth)?;
            }
            0x25 => {
                let t = self.pop();
                self.remove_sprite(f, &t)?;
            }
            0x26 => {
                let v = self.pop();
                let s = self.to_string(&v)?;
                self.vm.trace(&s);
            }
            0x27 => {
                let _t = self.pop();
                let _lock = self.pop();
                let c = self.pop();
                if self.to_bool(&c) {
                    self.pop_n(4);
                }
            }
            0x28 | 0x2D => {
                if act.op == 0x2D {
                    let n = self.pop_count();
                    self.pop_n(n);
                    self.push(Value::Undefined);
                }
            }
            _ => return self.step2(f, act),
        }
        Ok(None)
    }
}

impl Ctx<'_> {
    pub(super) fn step2(&mut self, f: &mut Frame, act: &Action) -> Result<Option<Flow>, Fault> {
        match act.op {
            0x2A => {
                let v = self.pop();
                return Err(Fault::Throw(v));
            }
            0x2B => {
                let obj = self.pop();
                let ctor = self.pop();
                let r = if self.instance_of(&obj, &ctor) { obj } else { Value::Null };
                self.push(r);
            }
            0x2C => {
                let ctor = self.pop();
                let n = self.pop_count();
                let ifaces: Vec<ObjId> = self.pop_n(n).iter().filter_map(Value::as_object).collect();
                let proto = self.get_member(&ctor, "prototype")?;
                if let Some(o) = proto.as_object().and_then(|p| self.vm.heap.get_mut(p)) {
                    o.interfaces = ifaces;
                }
            }
            0x30 => {
                let m = self.pop();
                let m = to_int32(self.to_number(&m));
                let r = if m > 0 { self.vm.random() % m as u32 } else { 0 };
                self.push(Value::Number(f64::from(r)));
            }
            0x32 | 0x36 => {
                let v = self.pop();
                let s = self.to_string(&v)?;
                self.push(Value::Number(s.chars().next().map_or(0.0, |c| f64::from(u32::from(c)))));
            }
            0x33 | 0x37 => {
                let v = self.pop();
                let c = char::from_u32(to_int32(self.to_number(&v)) as u32).unwrap_or('\0');
                self.push(c.to_string().into());
            }
            0x34 => {
                let t = self.vm.timer_ms();
                self.push(Value::Number(t));
            }
            0x3A => {
                let n = self.pop();
                let o = self.pop();
                let name = self.to_string(&n)?;
                let r = self.delete_member(&o, &name);
                self.push(Value::Bool(r));
            }
            0x3B => {
                let n = self.pop();
                let name = self.to_string(&n)?;
                let r = self.delete_variable(f, &name);
                self.push(Value::Bool(r));
            }
            0x3C => {
                let v = self.pop();
                let n = self.pop();
                let name = self.to_string(&n)?;
                self.define_local(f, &name, v);
            }
            0x41 => {
                let n = self.pop();
                let name = self.to_string(&n)?;
                let local = f.scope.obj;
                if self.vm.heap.get(local).is_some_and(|o| !o.props.contains(&name)) {
                    self.define_local(f, &name, Value::Undefined);
                }
            }
            0x3D => {
                let n = self.pop();
                let name = self.to_string(&n)?;
                let count = self.pop_count();
                let args = self.pop_n(count);
                let (func, this) = self.get_variable_this(f, &name)?;
                let r = self.call_checked(&func, this, &args, Value::Undefined, &name)?;
                self.push(r);
            }
            0x3E => {
                let v = self.pop();
                return Ok(Some(Flow::Return(v)));
            }
            0x40 => {
                let n = self.pop();
                let name = self.to_string(&n)?;
                let count = self.pop_count();
                let args = self.pop_n(count);
                let ctor = self.get_variable(f, &name)?;
                let r = self.construct_checked(&ctor, &args, &name)?;
                self.push(r);
            }
            0x42 => {
                let n = self.pop_count();
                let items = self.pop_n(n);
                let arr = self.new_array(items);
                self.push(arr.into());
            }
            0x43 => {
                let n = self.pop_count();
                let obj = self.new_object();
                for _ in 0..n {
                    let v = self.pop();
                    let k = self.pop();
                    let k = self.to_string(&k)?;
                    self.set_member(&Value::Object(obj), &k, v)?;
                }
                self.push(obj.into());
            }
            0x44 => {
                let v = self.pop();
                let t = self.type_of(&v);
                self.push(t.into());
            }
            0x45 => {
                let v = self.pop();
                let r = match self.display_of(&v) {
                    Some(id) => self.player.path(id).into(),
                    None => Value::Undefined,
                };
                self.push(r);
            }
            0x46 | 0x55 => {
                let v = self.pop();
                let obj = if act.op == 0x46 {
                    let name = self.to_string(&v)?;
                    self.get_variable(f, &name)?
                } else {
                    v
                };
                self.push(Value::Undefined);
                for k in self.enumerate(&obj) {
                    self.push(Value::String(k));
                }
            }
            0x4A => {
                let v = self.pop();
                let n = self.to_number(&v);
                self.push(Value::Number(n));
            }
            0x4B => {
                let v = self.pop();
                let s = self.to_string(&v)?;
                self.push(Value::String(s));
            }
            0x4C => {
                let v = self.vm.stack.last().cloned().unwrap_or_default();
                self.push(v);
            }
            0x4D => {
                let b = self.pop();
                let a = self.pop();
                self.push(b);
                self.push(a);
            }
            0x4E => {
                let n = self.pop();
                let o = self.pop();
                let name = self.to_string(&n)?;
                let v = self.get_member(&o, &name)?;
                self.push(v);
            }
            0x4F => {
                let v = self.pop();
                let n = self.pop();
                let o = self.pop();
                let name = self.to_string(&n)?;
                self.set_member(&o, &name, v)?;
            }
            0x50 | 0x51 => {
                let v = self.pop();
                let n = self.to_number(&v);
                self.push(Value::Number(if act.op == 0x50 { n + 1.0 } else { n - 1.0 }));
            }
            _ => return self.step3(f, act),
        }
        Ok(None)
    }
}

impl Ctx<'_> {
    pub(super) fn step3(&mut self, f: &mut Frame, act: &Action) -> Result<Option<Flow>, Fault> {
        match act.op {
            0x52 | 0x53 => {
                let n = self.pop();
                let obj = self.pop();
                let count = self.pop_count();
                let args = self.pop_n(count);
                let name = match &n {
                    Value::Undefined => None,
                    v => Some(self.to_string(v)?).filter(|s| !s.is_empty()),
                };
                let r = if act.op == 0x52 {
                    self.call_method(f, &obj, name.as_deref(), &args)?
                } else {
                    let ctor = match &name {
                        Some(n) => self.get_member(&obj, n)?,
                        None => obj.clone(),
                    };
                    self.construct_checked(&ctor, &args, name.as_deref().unwrap_or("<function>"))?
                };
                self.push(r);
            }
            0x54 => {
                let ctor = self.pop();
                let obj = self.pop();
                let r = self.instance_of(&obj, &ctor);
                self.push(Value::Bool(r));
            }
            0x60..=0x65 => {
                let b = self.pop();
                let a = self.pop();
                let (x, y) = (to_int32(self.to_number(&a)), to_int32(self.to_number(&b)));
                let r = match act.op {
                    0x60 => f64::from(x & y),
                    0x61 => f64::from(x | y),
                    0x62 => f64::from(x ^ y),
                    0x63 => f64::from(x.wrapping_shl(y as u32 & 31)),
                    0x64 => f64::from(x >> (y as u32 & 31)),
                    _ => f64::from((x as u32) >> (y as u32 & 31)),
                };
                self.push(Value::Number(r));
            }
            0x66 => {
                let b = self.pop();
                let a = self.pop();
                let r = self.strict_equals(&a, &b);
                self.push(Value::Bool(r));
            }
            0x69 => {
                let sup = self.pop();
                let sub = self.pop();
                self.extends(&sub, &sup)?;
            }
            _ => return self.step_data(f, act),
        }
        Ok(None)
    }

    pub(super) fn call_method(
        &mut self,
        f: &mut Frame,
        obj: &Value,
        name: Option<&str>,
        args: &[Value],
    ) -> Result<Value, Fault> {
        let sup = obj.as_object().and_then(|o| match &self.vm.heap.get(o)?.kind {
            Kind::Super { this, base, ctor } => Some((this.clone(), base.clone(), ctor.clone())),
            _ => None,
        });
        match (name, sup) {
            (None, Some((this, _, ctor))) => {
                let home = self.get_member(&ctor, "prototype")?;
                self.call_checked(&ctor, this, args, home, "super")
            }
            (None, None) => {
                let this = f.this.clone();
                self.call_checked(obj, this, args, Value::Undefined, "<function>")
            }
            (Some(n), Some((this, base, _))) => {
                let (func, home) = self.lookup_from(&base, &this, n)?;
                self.call_checked(&func, this, args, home, n)
            }
            (Some(n), None) => {
                let (func, home) = self.get_method(obj, n)?;
                if func.is_undefined() {
                    self.note_missing(obj, n);
                    return Ok(Value::Undefined);
                }
                self.call_checked(&func, obj.clone(), args, home, n)
            }
        }
    }

    pub(super) fn extends(&mut self, sub: &Value, sup: &Value) -> Result<(), Fault> {
        let sup_proto = self.get_member(sup, "prototype")?;
        if sub.as_object().is_none() || sup_proto.as_object().is_none() {
            return Ok(());
        }
        let proto = self.vm.heap.alloc(Object::new(sup_proto, Kind::Plain));
        self.define(proto, "__constructor__", sup.clone(), flags::DONT_ENUM | flags::DONT_DELETE);
        self.set_member(sub, "prototype", proto.into())
    }
}
