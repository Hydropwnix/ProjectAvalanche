use super::*;

impl Ctx<'_> {
    pub(super) fn index_after(&self, f: &Frame, size: u16) -> Result<usize, Fault> {
        let off = offset_at(&f.code, f.pc) + u32::from(size);
        index_of(&f.code, off).ok_or(Fault::Abort("block end off an action boundary"))
    }

    pub(super) fn step_data(&mut self, f: &mut Frame, act: &Action) -> Result<Option<Flow>, Fault> {
        match &act.data {
            ActionData::GotoFrame(n) => {
                self.frame_goto(f, Value::Number(f64::from(*n) + 1.0), None)?;
            }
            ActionData::GotoLabel(l) => {
                let t = f.target.map(Value::Object).unwrap_or_else(|| f.this.clone());
                if let Some(id) = self.display_of(&t) {
                    self.goto_value_with(id, &Value::str(l), None, false)?;
                }
            }
            ActionData::GotoFrame2 { flags, scene_bias } => {
                let v = self.pop();
                let v = match (&v, scene_bias) {
                    (Value::String(_), _) | (_, None) => v,
                    (_, Some(b)) => Value::Number(self.to_number(&v) + f64::from(*b)),
                };
                self.frame_goto(f, v, Some(flags & 1 != 0))?;
            }
            ActionData::StoreRegister(r) => {
                let v = self.vm.stack.last().cloned().unwrap_or_default();
                if let Some(slot) = f.regs.get_mut(usize::from(*r)) {
                    *slot = v;
                }
            }
            ActionData::ConstantPool(p) => {
                f.pool = Arc::new(p.iter().map(|s| Str::from(s.as_str())).collect());
            }
            ActionData::WaitForFrame { .. } => {}
            ActionData::WaitForFrame2 { .. } => drop(self.pop()),
            ActionData::SetTarget(t) => {
                let v = Value::str(t);
                self.set_target_value(f, &v)?;
            }
            ActionData::DefineFunction { name, params, code_size } => {
                let params = params.iter().map(|p| (0u8, Str::from(p.as_str()))).collect();
                self.define_function(f, name, None, params, *code_size)?;
            }
            ActionData::DefineFunction2 { name, register_count, flags, params, code_size } => {
                let params = params.iter().map(|(r, p)| (*r, Str::from(p.as_str()))).collect();
                self.define_function(f, name, Some((*register_count, *flags)), params, *code_size)?;
            }
            ActionData::Try { flags, try_size, catch_size, finally_size, catch_name, catch_register } => {
                return self.run_try(f, *flags, [*try_size, *catch_size, *finally_size], catch_name, *catch_register);
            }
            ActionData::With { size } => {
                let v = self.pop();
                let end = self.index_after(f, *size)?;
                if let Some(o) = v.as_object() {
                    let s = Arc::new(Scope { obj: o, kind: ScopeKind::With, parent: Some(f.scope.clone()) });
                    f.withs.push((end, std::mem::replace(&mut f.scope, s)));
                } else {
                    f.pc = end;
                }
            }
            ActionData::Push(values) => {
                for v in values {
                    let x = match v {
                        PushValue::String(s) => Value::str(s),
                        PushValue::Float(x) => Value::Number(f64::from(*x)),
                        PushValue::Null => Value::Null,
                        PushValue::Undefined => Value::Undefined,
                        PushValue::Register(r) => self.register(f, *r),
                        PushValue::Bool(b) => Value::Bool(*b),
                        PushValue::Double(d) => Value::Number(*d),
                        // TODO(guess): Int pushes are plain numbers (Scaleform keeps type 4, 0x82774A38, and `Equals2` compares it via ToInt32, 0x8277DDA8); settle: an integer-vs-string comparison on the disc.
                        PushValue::Int(i) => Value::Number(f64::from(*i)),
                        PushValue::Constant(c) => self.constant(f, *c),
                    };
                    self.push(x);
                }
            }
            ActionData::Jump(d) => self.branch(f, *d)?,
            ActionData::If(d) => {
                let v = self.pop();
                if self.to_bool(&v) {
                    self.branch(f, *d)?;
                }
            }
            ActionData::GetUrl { url, target } => {
                let what = format!("{url} → {target}");
                self.vm.diag.note("GetURL", &what);
            }
            ActionData::GetUrl2(_) => {
                let t = self.pop();
                let u = self.pop();
                let what = format!("{} → {}", self.to_string(&u)?, self.to_string(&t)?);
                self.vm.diag.note("GetURL2", &what);
            }
            ActionData::None if act.op == 0x9E => {
                let v = self.pop();
                let what = self.to_string(&v)?;
                self.vm.diag.note("Call (frame actions) not supported", &what);
            }
            _ => {
                let what = format!("{:#04x}", act.op);
                self.vm.diag.note("unknown action", &what);
            }
        }
        Ok(None)
    }

    pub(super) fn define_function(
        &mut self,
        f: &mut Frame,
        name: &str,
        v2: Option<(u8, u16)>,
        params: Vec<(u8, Str)>,
        size: u16,
    ) -> Result<(), Fault> {
        let start = f.pc;
        let end = self.index_after(f, size)?;
        let af = AvmFunction {
            name: Str::from(name),
            code: f.code.clone(),
            start,
            end,
            v2,
            params,
            pool: f.pool.clone(),
            version: f.version,
            movie: f.movie,
        };
        let func = Func { code: FuncCode::Avm(Arc::new(af)), scope: Some(f.scope.clone()), base: f.target };
        let fo = self.new_function_object(func);
        f.pc = end;
        if name.is_empty() {
            self.push(fo.into());
        } else {
            self.define_local(f, name, fo.into());
        }
        Ok(())
    }
}

impl Ctx<'_> {
    pub(super) fn run_try(
        &mut self,
        f: &mut Frame,
        flags: u8,
        sizes: [u16; 3],
        catch_name: &Option<String>,
        catch_register: Option<u8>,
    ) -> Result<Option<Flow>, Fault> {
        let try_start = f.pc;
        let try_end = self.index_after(f, sizes[0])?;
        f.pc = try_end;
        let catch_end = self.index_after(f, sizes[1])?;
        f.pc = catch_end;
        let finally_end = self.index_after(f, sizes[2])?;
        let (has_catch, has_finally) = (flags & 1 != 0, flags & 2 != 0);
        let height = self.vm.stack.len();
        let mut out = self.run(f, try_start, try_end);
        if has_catch && let Err(Fault::Throw(v)) = &out {
            let v = v.clone();
            self.vm.stack.truncate(height);
            match (catch_register, catch_name) {
                (Some(r), _) => {
                    if let Some(slot) = f.regs.get_mut(usize::from(r)) {
                        *slot = v;
                    }
                }
                (None, Some(n)) => self.define_local(f, n, v),
                (None, None) => {}
            }
            out = self.run(f, try_end, catch_end);
        }
        let out = match out {
            Ok(Flow::Exit(pc)) if pc == try_end || pc == catch_end => Ok(Flow::Exit(finally_end)),
            o => o,
        };
        if has_finally {
            match self.run(f, catch_end, finally_end)? {
                Flow::Exit(pc) if pc == finally_end => {}
                Flow::Exit(pc) => {
                    f.pc = pc;
                    return Ok(None);
                }
                other => return Ok(Some(other)),
            }
        }
        match out? {
            Flow::Exit(pc) => {
                f.pc = pc;
                Ok(None)
            }
            other => Ok(Some(other)),
        }
    }

    pub fn call(&mut self, func: &Value, this: Value, args: &[Value], home: Value) -> Result<Value, Fault> {
        let Some(fid) = func.as_object() else { return Ok(Value::Undefined) };
        let Some(fd) = self.vm.heap.get(fid).and_then(|o| o.func()).cloned() else { return Ok(Value::Undefined) };
        if self.vm.depth >= MAX_CALL_DEPTH {
            self.vm.diag.note("abort", "256 levels of recursion");
            if let Some(m) = self.vm.sandbox.charged {
                self.vm.sandbox.stop(m, "256 levels of recursion".to_string());
                return Err(Fault::Stopped(m));
            }
            return Ok(Value::Undefined);
        }
        let peaks = &mut self.vm.sandbox.peaks;
        peaks.call_depth = peaks.call_depth.max(self.vm.depth + 1);
        let constructing = std::mem::replace(&mut self.vm.next_construct, false);
        match fd.code.clone() {
            FuncCode::Native(nf) => {
                let prev = std::mem::replace(&mut self.vm.constructing, constructing);
                self.vm.depth += 1;
                let r = nf(self, this, args);
                self.vm.depth -= 1;
                self.vm.constructing = prev;
                r
            }
            FuncCode::Avm(af) => self.call_avm(fid, &fd, &af, this, args, home),
        }
    }

    pub(super) fn call_avm(
        &mut self,
        fid: ObjId,
        fd: &Func,
        af: &Arc<AvmFunction>,
        this: Value,
        args: &[Value],
        home: Value,
    ) -> Result<Value, Fault> {
        let act = self.vm.heap.alloc(Object::new(Value::Null, Kind::Activation));
        let scope = Arc::new(Scope { obj: act, kind: ScopeKind::Local, parent: fd.scope.clone() });
        let nregs = af.v2.map_or(4, |(n, _)| usize::from(n).max(1));
        let home = if home.is_undefined() { self.proto_of(&this) } else { home };
        let mut f = Frame {
            code: af.code.clone(),
            pc: af.start,
            pool: af.pool.clone(),
            scope,
            regs: vec![Value::Undefined; nregs],
            this: this.clone(),
            target: fd.base,
            version: af.version,
            movie: af.movie,
            callee: Value::Object(fid),
            home,
            withs: Vec::new(),
        };
        let fl = af.v2.map_or(0, |(_, fl)| fl);
        let v2 = af.v2.is_some();
        let wants_args = !v2 || fl & 0x0C00 != 0x0800;
        let arguments = if wants_args {
            let a = self.new_array(args.to_vec());
            self.define(a, "callee", Value::Object(fid), flags::DONT_ENUM);
            Value::Object(a)
        } else {
            Value::Undefined
        };
        if v2 {
            let mut r = 1;
            let mut preload = |f: &mut Frame, v: Value| {
                if let Some(slot) = f.regs.get_mut(r) {
                    *slot = v;
                }
                r += 1;
            };
            if fl & 0x0100 != 0 {
                preload(&mut f, this.clone());
            }
            if fl & 0x0400 != 0 {
                preload(&mut f, arguments.clone());
            }
            if fl & 0x1000 != 0 {
                let s = self.super_object(&f);
                preload(&mut f, s);
            }
            if fl & 0x4000 != 0 {
                let root = self.root_value(&f);
                preload(&mut f, root);
            }
            if fl & 0x8000 != 0 {
                let parent = self.parent_value(&f);
                preload(&mut f, parent);
            }
            if fl & 0x0001 != 0 {
                preload(&mut f, Value::Object(self.vm.global));
            }
        }
        if wants_args && fl & 0x0400 == 0 {
            self.define(act, "arguments", arguments, 0);
        }
        for (i, (reg, name)) in af.params.iter().enumerate() {
            let v = args.get(i).cloned().unwrap_or_default();
            if *reg != 0 && v2 {
                if let Some(slot) = f.regs.get_mut(usize::from(*reg)) {
                    *slot = v;
                }
            } else {
                self.define(act, name, v, 0);
            }
        }
        let height = self.vm.stack.len();
        self.vm.depth += 1;
        let entry = self.vm.sandbox.charged;
        let r = self.run(&mut f, af.start, af.end);
        self.vm.depth -= 1;
        self.vm.stack.truncate(height);
        if let (None, Err(Fault::Stopped(_))) = (entry, &r) {
            return Ok(Value::Undefined);
        }
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Undefined),
        }
    }

    pub fn super_object(&mut self, f: &Frame) -> Value {
        let base = self.proto_of(&f.home);
        let ctor = match f.home.as_object().and_then(|h| self.vm.heap.get(h)) {
            Some(o) => o.props.get("__constructor__").map(|p| p.value.clone()).unwrap_or_default(),
            None => Value::Undefined,
        };
        let s = Object::new(Value::Null, Kind::Super { this: f.this.clone(), base, ctor });
        Value::Object(self.vm.heap.alloc(s))
    }

    pub fn proto_of(&self, v: &Value) -> Value {
        v.as_object().and_then(|o| self.vm.heap.get(o)).map(|o| o.proto.clone()).unwrap_or_default()
    }
}
