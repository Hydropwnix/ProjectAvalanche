use super::*;

impl Ctx<'_> {
    pub fn get_variable(&mut self, f: &mut Frame, name: &str) -> Result<Value, Fault> {
        Ok(self.get_variable_this(f, name)?.0)
    }

    pub fn get_variable_this(&mut self, f: &mut Frame, name: &str) -> Result<(Value, Value), Fault> {
        match name {
            "this" => return Ok((f.this.clone(), Value::Undefined)),
            "_global" => return Ok((Value::Object(self.vm.global), Value::Undefined)),
            "_root" => return Ok((self.root_value(f), Value::Undefined)),
            "super" if !f.home.is_undefined() && !self.has_local(f, "super") => {
                return Ok((self.super_object(f), Value::Undefined));
            }
            _ => {}
        }
        if name.contains(['.', '/', ':']) {
            return Ok((self.resolve_path_variable(f, name)?, Value::Undefined));
        }
        let mut s = Some(f.scope.clone());
        while let Some(sc) = s {
            if self.has_property(sc.obj, name) {
                let this = if sc.kind == ScopeKind::Local { Value::Undefined } else { Value::Object(sc.obj) };
                let v = self.get_member(&Value::Object(sc.obj), name)?;
                return Ok((v, this));
            }
            if sc.kind == ScopeKind::Target
                && let Some(l) = name.strip_prefix("_level").and_then(|n| n.parse::<i32>().ok())
                && let Some(&id) = self.player.levels.get(&l)
            {
                return Ok((Value::Object(self.object_for(id)), Value::Undefined));
            }
            s = sc.parent.clone();
        }
        Ok((Value::Undefined, Value::Undefined))
    }

    pub(super) fn has_local(&mut self, f: &Frame, name: &str) -> bool {
        let mut s = Some(f.scope.clone());
        while let Some(sc) = s {
            if sc.kind == ScopeKind::Local {
                return self.vm.heap.get(sc.obj).is_some_and(|o| o.props.contains(name));
            }
            s = sc.parent.clone();
        }
        false
    }

    pub fn set_variable(&mut self, f: &mut Frame, name: &str, v: Value) -> Result<(), Fault> {
        if name.contains(['.', '/', ':']) {
            return self.set_path_variable(f, name, v);
        }
        let mut s = Some(f.scope.clone());
        while let Some(sc) = s {
            if sc.kind == ScopeKind::Target || sc.kind == ScopeKind::Global || self.has_property(sc.obj, name) {
                return self.set_member(&Value::Object(sc.obj), name, v);
            }
            s = sc.parent.clone();
        }
        Ok(())
    }

    pub fn define_local(&mut self, f: &mut Frame, name: &str, v: Value) {
        let mut s = Some(f.scope.clone());
        while let Some(sc) = s {
            match sc.kind {
                ScopeKind::Local => {
                    self.define(sc.obj, name, v, 0);
                    return;
                }
                ScopeKind::Target | ScopeKind::Global => {
                    let _ = self.set_member(&Value::Object(sc.obj), name, v);
                    return;
                }
                ScopeKind::With => s = sc.parent.clone(),
            }
        }
    }

    pub fn delete_variable(&mut self, f: &mut Frame, name: &str) -> bool {
        let mut s = Some(f.scope.clone());
        while let Some(sc) = s {
            if self.has_property(sc.obj, name) {
                return self.delete_member(&Value::Object(sc.obj), name);
            }
            s = sc.parent.clone();
        }
        false
    }

    pub(super) fn split_path(name: &str) -> (&str, &str) {
        match name.rfind([':', '.']) {
            Some(i) => (&name[..i], &name[i + 1..]),
            None => ("", name),
        }
    }

    pub(super) fn resolve_path_variable(&mut self, f: &mut Frame, name: &str) -> Result<Value, Fault> {
        let (path, member) = Self::split_path(name);
        let Some(t) = self.resolve_target_path(f, path)? else { return Ok(Value::Undefined) };
        self.get_member(&t, member)
    }

    pub(super) fn set_path_variable(&mut self, f: &mut Frame, name: &str, v: Value) -> Result<(), Fault> {
        let (path, member) = Self::split_path(name);
        match self.resolve_target_path(f, path)? {
            Some(t) => self.set_member(&t, member, v),
            None => Ok(()),
        }
    }

    pub fn resolve_target_path(&mut self, f: &mut Frame, path: &str) -> Result<Option<Value>, Fault> {
        let mut cur: Value = f.target.map(Value::Object).unwrap_or_else(|| f.this.clone());
        let mut rest = path;
        if let Some(r) = rest.strip_prefix('/') {
            cur = self.root_value(f);
            rest = r;
        }
        for (i, seg) in rest.split(['.', '/']).enumerate() {
            if seg.is_empty() {
                continue;
            }
            cur = match seg {
                ".." => self.get_member(&cur, "_parent")?,
                _ if i == 0 => self.get_variable(f, seg)?,
                _ => self.get_member(&cur, seg)?,
            };
            if cur.is_nullish() {
                return Ok(None);
            }
        }
        Ok(Some(cur))
    }
}
