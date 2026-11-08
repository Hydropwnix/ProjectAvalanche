use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use naga::valid::ModuleInfo;
use naga::{Block, Expression, Function, Handle, MathFunction, Module, Span, Statement, TypeInner};

use super::terminate::{self, LoopProofs};
use super::{Cost, Limits};

impl Cost {
    fn add(self, o: Cost) -> Cost {
        Cost {
            operations: self.operations.saturating_add(o.operations),
            texture_reads: self.texture_reads.saturating_add(o.texture_reads),
            frame_bytes: self.frame_bytes.max(o.frame_bytes),
        }
    }

    fn max(self, o: Cost) -> Cost {
        Cost {
            operations: self.operations.max(o.operations),
            texture_reads: self.texture_reads.max(o.texture_reads),
            frame_bytes: self.frame_bytes.max(o.frame_bytes),
        }
    }

    fn times(self, n: u64) -> Cost {
        Cost {
            operations: self.operations.saturating_mul(n),
            texture_reads: self.texture_reads.saturating_mul(n),
            frame_bytes: self.frame_bytes,
        }
    }

    fn op(n: u64) -> Cost {
        Cost { operations: n, ..Cost::default() }
    }
}

pub(super) type Refusal = (Span, String);

pub(super) fn components(module: &Module, inner: &TypeInner) -> u64 {
    match inner {
        TypeInner::Vector { size, .. } => *size as u64,
        TypeInner::Matrix { columns, rows, .. } => *columns as u64 * *rows as u64,
        TypeInner::Array { base, size, .. } => match size {
            naga::ArraySize::Constant(n) => {
                u64::from(n.get()).saturating_mul(components(module, &module.types[*base].inner))
            }
            _ => u64::MAX,
        },
        TypeInner::Struct { members, .. } => {
            members.iter().map(|m| components(module, &module.types[m.ty].inner)).fold(0, u64::saturating_add)
        }
        _ => 1,
    }
}

pub(super) fn value_bytes(module: &Module, inner: &TypeInner) -> u64 {
    match inner {
        TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. } => 0,
        _ => u64::from(inner.size(module.to_ctx())),
    }
}

pub(super) struct Hook<'a> {
    pub module: &'a Module,
    pub info: &'a ModuleInfo,
    pub functions: &'a BTreeSet<Handle<Function>>,
    pub limits: &'a Limits,
    done: RefCell<BTreeMap<Handle<Function>, Result<Cost, Refusal>>>,
    open: RefCell<BTreeSet<Handle<Function>>>,
}

impl<'a> Hook<'a> {
    pub fn new(
        module: &'a Module,
        info: &'a ModuleInfo,
        functions: &'a BTreeSet<Handle<Function>>,
        limits: &'a Limits,
    ) -> Self {
        Hook { module, info, functions, limits, done: RefCell::default(), open: RefCell::default() }
    }

    pub fn cost(&self, h: Handle<Function>) -> Result<Cost, Refusal> {
        if let Some(r) = self.done.borrow().get(&h) {
            return r.clone();
        }
        let span = self.module.functions.get_span(h);
        if !self.open.borrow_mut().insert(h) {
            return Err((span, "calls itself (recursion)".into()));
        }
        let r = self.function(h).map_err(|(s, m)| (if s.is_defined() { s } else { span }, m));
        self.open.borrow_mut().remove(&h);
        self.done.borrow_mut().insert(h, r.clone());
        r
    }

    fn function(&self, h: Handle<Function>) -> Result<Cost, Refusal> {
        let m = self.module;
        let f = &m.functions[h];
        let span = m.functions.get_span(h);
        if f.diagnostic_filter_leaf.is_some() {
            return Err((span, "a `diagnostic` attribute: the checks cannot be switched off".into()));
        }
        let mut own_bytes = 0u64;
        for (_, l) in f.local_variables.iter() {
            own_bytes = own_bytes.saturating_add(self.value(&m.types[l.ty].inner, span, "a local")?);
        }
        for a in &f.arguments {
            own_bytes = own_bytes.saturating_add(self.value(&m.types[a.ty].inner, span, "an argument")?);
        }
        if let Some(r) = &f.result {
            self.value(&m.types[r.ty].inner, span, "the result")?;
        }
        for (e, expr) in f.expressions.iter() {
            let at = f.expressions.get_span(e);
            if let Some(why) = forbidden_expression(expr) {
                return Err((at, why.into()));
            }
            self.value(self.info[h][e].ty.inner_with(&m.types), at, "a value")?;
        }
        let proofs = terminate::prove(m, f, self.limits);
        let walk = Walk { hook: self, f, h, proofs: &proofs };
        let body = walk.block(&f.body)?;
        let frame = own_bytes.saturating_add(body.frame_bytes);
        if frame > self.limits.max_frame_bytes {
            return Err((
                span,
                format!(
                    "{frame} bytes of locals and arguments along its calls, over the cap of {} per hook",
                    self.limits.max_frame_bytes
                ),
            ));
        }
        Ok(Cost { frame_bytes: frame, ..body })
    }

    fn value(&self, inner: &TypeInner, at: Span, what: &str) -> Result<u64, Refusal> {
        if let Some(why) = super::check::forbidden_type(inner) {
            return Err((at, why));
        }
        let bytes = value_bytes(self.module, inner);
        if bytes > self.limits.max_value_bytes {
            return Err((
                at,
                format!("{what} of {bytes} bytes, over the cap of {} per value", self.limits.max_value_bytes),
            ));
        }
        Ok(bytes)
    }
}

fn forbidden_expression(e: &Expression) -> Option<&'static str> {
    match e {
        Expression::Literal(_)
        | Expression::Constant(_)
        | Expression::ZeroValue(_)
        | Expression::Compose { .. }
        | Expression::Access { .. }
        | Expression::AccessIndex { .. }
        | Expression::Splat { .. }
        | Expression::Swizzle { .. }
        | Expression::FunctionArgument(_)
        | Expression::LocalVariable(_)
        | Expression::Load { .. }
        | Expression::ImageSample { .. }
        | Expression::ImageLoad { .. }
        | Expression::ImageQuery { .. }
        | Expression::Unary { .. }
        | Expression::Binary { .. }
        | Expression::Select { .. }
        | Expression::Derivative { .. }
        | Expression::Relational { .. }
        | Expression::Math { .. }
        | Expression::As { .. }
        | Expression::CallResult(_) => None,
        Expression::GlobalVariable(_) => {
            Some("a module-scope variable or binding: a hook uses the family's bindings only")
        }
        Expression::Override(_) => Some("a pipeline-overridable constant"),
        Expression::AtomicResult { .. } | Expression::WorkGroupUniformLoadResult { .. } => {
            Some("atomics or workgroup memory (compute and storage only)")
        }
        Expression::ArrayLength(_) => Some("a runtime-sized array (storage only)"),
        Expression::RayQueryVertexPositions { .. }
        | Expression::RayQueryProceedResult
        | Expression::RayQueryGetIntersection { .. } => Some("ray tracing"),
        Expression::SubgroupBallotResult | Expression::SubgroupOperationResult { .. } => {
            Some("a subgroup operation (a device feature)")
        }
        Expression::CooperativeLoad { .. } | Expression::CooperativeMultiplyAdd { .. } => {
            Some("a cooperative matrix (a device feature)")
        }
    }
}

struct Walk<'a, 'b> {
    hook: &'b Hook<'a>,
    f: &'b Function,
    h: Handle<Function>,
    proofs: &'b LoopProofs,
}

impl Walk<'_, '_> {
    fn ty(&self, e: Handle<Expression>) -> &TypeInner {
        self.hook.info[self.h][e].ty.inner_with(&self.hook.module.types)
    }

    fn n(&self, e: Handle<Expression>) -> u64 {
        components(self.hook.module, self.ty(e))
    }

    fn weight(&self, e: Handle<Expression>) -> u64 {
        let n = self.n(e).max(1);
        let pointer = matches!(self.ty(e), TypeInner::Pointer { .. } | TypeInner::ValuePointer { .. });
        match &self.f.expressions[e] {
            Expression::Binary { op: naga::BinaryOperator::Multiply, left, right } => {
                match (self.ty(*left), self.ty(*right)) {
                    (TypeInner::Matrix { columns: c1, rows: r1, .. }, TypeInner::Matrix { columns: c2, .. }) => {
                        *r1 as u64 * *c1 as u64 * *c2 as u64
                    }
                    (TypeInner::Matrix { columns, rows, .. }, TypeInner::Vector { .. })
                    | (TypeInner::Vector { .. }, TypeInner::Matrix { columns, rows, .. }) => {
                        *columns as u64 * *rows as u64
                    }
                    _ => n,
                }
            }
            Expression::Math { fun, arg, .. } => {
                let a = self.n(*arg);
                let dim = match self.ty(*arg) {
                    TypeInner::Matrix { columns, .. } => *columns as u64,
                    _ => 1,
                };
                match fun {
                    MathFunction::Dot => a,
                    MathFunction::Cross => 6,
                    MathFunction::Length | MathFunction::Distance | MathFunction::Normalize => a.saturating_mul(2),
                    MathFunction::FaceForward | MathFunction::Reflect | MathFunction::Refract => a.saturating_mul(3),
                    MathFunction::Determinant | MathFunction::Inverse => dim.pow(3),
                    MathFunction::Transpose => dim.pow(2),
                    _ => n.max(a),
                }
            }
            Expression::Access { base, .. } if !pointer => self.n(*base).max(1),
            Expression::Access { .. } | Expression::AccessIndex { .. } if pointer => 1,
            Expression::ImageSample { .. } | Expression::ImageLoad { .. } => 4,
            Expression::ImageQuery { .. } => 2,
            _ => n,
        }
    }

    fn block(&self, block: &Block) -> Result<Cost, Refusal> {
        let mut total = Cost::default();
        for (s, span) in block.span_iter() {
            total = total.add(self.statement(s, *span)?);
        }
        Ok(total)
    }

    fn statement(&self, s: &Statement, span: Span) -> Result<Cost, Refusal> {
        let refuse = |m: &str| Err((span, m.to_string()));
        Ok(match s {
            Statement::Emit(r) => {
                let mut c = Cost::default();
                for e in r.clone() {
                    c.operations = c.operations.saturating_add(self.weight(e));
                    if matches!(self.f.expressions[e], Expression::ImageSample { .. } | Expression::ImageLoad { .. }) {
                        c.texture_reads += 1;
                    }
                }
                c
            }
            Statement::Block(b) => self.block(b)?,
            Statement::If { accept, reject, .. } => self.block(accept)?.max(self.block(reject)?).add(Cost::op(1)),
            Statement::Switch { cases, .. } => {
                let mut c = Cost::op(1);
                for case in cases {
                    c = c.add(self.block(&case.body)?);
                }
                c
            }
            Statement::Loop { body, continuing, .. } => {
                let trips = match self.proofs.get(&(s as *const Statement)) {
                    Some(Ok(t)) => *t,
                    Some(Err(why)) => return Err((span, why.clone())),
                    None => return refuse(terminate::FOR_ONLY),
                };
                let once = self.block(body)?.add(self.block(continuing)?).add(Cost::op(1));
                once.times(trips.max(1)).add(Cost::op(1))
            }
            Statement::Store { value, .. } => Cost::op(self.n(*value).max(1)),
            Statement::Call { function, arguments, result } => {
                if !self.hook.functions.contains(function) {
                    let name = self.hook.module.functions[*function].name.as_deref().unwrap_or("?");
                    return Err((
                        span,
                        format!("calls `{name}`, which is not the hook's: a hook calls its own functions only"),
                    ));
                }
                let callee =
                    self.hook.cost(*function).map_err(|(_, m)| (span, format!("a call to a refused function: {m}")))?;
                let copied: u64 = arguments.iter().map(|a| self.n(*a)).fold(0, u64::saturating_add);
                let returned = result.map_or(0, |r| self.n(r));
                Cost::op(copied.saturating_add(returned).saturating_add(1)).add(callee)
            }
            Statement::Return { value } => Cost::op(value.map_or(1, |v| self.n(v).max(1))),
            Statement::Break | Statement::Continue | Statement::Kill => Cost::op(1),
            Statement::ControlBarrier(_) | Statement::MemoryBarrier(_) => return refuse("a barrier (compute only)"),
            Statement::ImageStore { .. } | Statement::ImageAtomic { .. } | Statement::Atomic { .. } => {
                return refuse("a store to a texture or an atomic (bindings only)");
            }
            Statement::WorkGroupUniformLoad { .. } => return refuse("workgroup memory (compute only)"),
            Statement::SubgroupBallot { .. }
            | Statement::SubgroupGather { .. }
            | Statement::SubgroupCollectiveOperation { .. } => {
                return refuse("a subgroup operation (a device feature)");
            }
            Statement::RayQuery { .. } | Statement::RayPipelineFunction(_) => return refuse("ray tracing"),
            Statement::CooperativeStore { .. } => return refuse("a cooperative matrix (a device feature)"),
        })
    }
}

pub(super) fn frame_bytes(module: &Module, h: Handle<Function>, memo: &mut BTreeMap<Handle<Function>, u64>) -> u64 {
    if let Some(b) = memo.get(&h) {
        return *b;
    }
    memo.insert(h, 0);
    let f = &module.functions[h];
    let own: u64 = f
        .local_variables
        .iter()
        .map(|(_, l)| value_bytes(module, &module.types[l.ty].inner))
        .chain(f.arguments.iter().map(|a| value_bytes(module, &module.types[a.ty].inner)))
        .fold(0, u64::saturating_add);
    let mut callees = BTreeSet::new();
    calls_in(&f.body, &mut callees);
    let deepest = callees.into_iter().map(|c| frame_bytes(module, c, memo)).max().unwrap_or(0);
    let total = own.saturating_add(deepest);
    memo.insert(h, total);
    total
}

pub(super) fn calls_in(block: &Block, out: &mut BTreeSet<Handle<Function>>) {
    for s in block.iter() {
        match s {
            Statement::Call { function, .. } => {
                out.insert(*function);
            }
            Statement::Block(b) => calls_in(b, out),
            Statement::If { accept, reject, .. } => {
                calls_in(accept, out);
                calls_in(reject, out);
            }
            Statement::Switch { cases, .. } => cases.iter().for_each(|c| calls_in(&c.body, out)),
            Statement::Loop { body, continuing, .. } => {
                calls_in(body, out);
                calls_in(continuing, out);
            }
            _ => {}
        }
    }
}

pub(super) fn straight_line(
    module: &Module,
    info: &ModuleInfo,
    h: Handle<Function>,
    memo: &mut BTreeMap<Handle<Function>, u64>,
) -> u64 {
    straight_function(module, info, &module.functions[h], Some(h), memo)
}

pub(super) fn straight_function(
    module: &Module,
    info: &ModuleInfo,
    f: &Function,
    h: Option<Handle<Function>>,
    memo: &mut BTreeMap<Handle<Function>, u64>,
) -> u64 {
    if let Some(h) = h {
        if let Some(n) = memo.get(&h) {
            return *n;
        }
        memo.insert(h, 0);
    }
    let fi = match h {
        Some(h) => &info[h],
        None => match module.entry_points.iter().position(|e| std::ptr::eq(&e.function, f)) {
            Some(i) => info.get_entry_point(i),
            None => return 0,
        },
    };
    let ty = |e: Handle<Expression>| fi[e].ty.inner_with(&module.types);
    let mut total = 0u64;
    let mut stack: Vec<&Block> = vec![&f.body];
    while let Some(block) = stack.pop() {
        for s in block.iter() {
            total = total.saturating_add(1);
            match s {
                Statement::Emit(r) => {
                    for e in r.clone() {
                        total = total.saturating_add(components(module, ty(e)).max(1));
                    }
                }
                Statement::Store { value, .. } => total = total.saturating_add(components(module, ty(*value))),
                Statement::Call { function, .. } => {
                    total = total.saturating_add(straight_line(module, info, *function, memo));
                }
                Statement::Block(b) => stack.push(b),
                Statement::If { accept, reject, .. } => {
                    stack.push(accept);
                    stack.push(reject);
                }
                Statement::Switch { cases, .. } => cases.iter().for_each(|c| stack.push(&c.body)),
                Statement::Loop { body, continuing, .. } => {
                    stack.push(body);
                    stack.push(continuing);
                }
                _ => {}
            }
        }
    }
    if let Some(h) = h {
        memo.insert(h, total);
    }
    total
}
