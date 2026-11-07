use std::collections::{BTreeSet, HashMap};

use naga::{BinaryOperator, Block, Expression, Function, Handle, Literal, LocalVariable, Module, Statement};

use super::Limits;

pub(super) type LoopProofs = HashMap<*const Statement, Result<u64, String>>;

pub(super) const FOR_ONLY: &str = "loops must be `for (var i = <constant>; i < <constant>; i++)` (or `<=`, `>`, `>=`, \
                                   a constant step): `loop`, `while` and `break if` are refused";

pub(super) fn const_int(module: &Module, exprs: &naga::Arena<Expression>, e: Handle<Expression>) -> Option<i64> {
    let lit = |l: &Literal| match *l {
        Literal::I32(v) => Some(i64::from(v)),
        Literal::U32(v) => Some(i64::from(v)),
        Literal::AbstractInt(v) => Some(v),
        _ => None,
    };
    match &exprs[e] {
        Expression::Literal(l) => lit(l),
        Expression::Constant(c) => match &module.global_expressions[module.constants[*c].init] {
            Expression::Literal(l) => lit(l),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn root_local(exprs: &naga::Arena<Expression>, mut e: Handle<Expression>) -> Option<Handle<LocalVariable>> {
    loop {
        match exprs[e] {
            Expression::LocalVariable(l) => return Some(l),
            Expression::Access { base, .. } | Expression::AccessIndex { base, .. } => e = base,
            _ => return None,
        }
    }
}

struct LoopAt<'a> {
    at: *const Statement,
    body: &'a Block,
    continuing: &'a Block,
    break_if: bool,
}

struct Counted {
    counter: Handle<LocalVariable>,
    op: BinaryOperator,
    bound: i64,
    step: i64,
    increment: *const Statement,
}

struct Write {
    at: *const Statement,
    inside: Vec<*const Statement>,
    what: Written,
}

enum Written {
    Constant(i64),
    NotConstant,
    Pointer,
}

pub(super) fn prove(module: &Module, f: &Function, limits: &Limits) -> LoopProofs {
    let mut loops = Vec::new();
    collect_loops(&f.body, &mut loops);
    let shapes: Vec<Result<Counted, String>> = loops.iter().map(|l| canonical(module, f, l)).collect();
    let mut per_counter: HashMap<Handle<LocalVariable>, usize> = HashMap::new();
    for c in shapes.iter().flatten() {
        *per_counter.entry(c.counter).or_default() += 1;
    }
    let counters: BTreeSet<Handle<LocalVariable>> = per_counter.keys().copied().collect();
    let mut writes: HashMap<Handle<LocalVariable>, Vec<Write>> = HashMap::new();
    collect_writes(module, f, &f.body, &mut Vec::new(), &counters, &mut writes);
    let mut out = LoopProofs::new();
    for (l, shape) in loops.iter().zip(shapes) {
        let proof = shape.and_then(|c| {
            let name = f.local_variables[c.counter].name.as_deref().unwrap_or("?");
            let n = per_counter[&c.counter];
            if n > 1 {
                return Err(format!(
                    "the loop counter `{name}` counts {n} loops: each loop needs a counter of its own"
                ));
            }
            let mut starts = BTreeSet::new();
            match f.local_variables[c.counter].init {
                None => {
                    starts.insert(0);
                }
                Some(init) => match const_int(module, &f.expressions, init) {
                    Some(v) => {
                        starts.insert(v);
                    }
                    None => return Err(not_constant(name)),
                },
            }
            for w in writes.get(&c.counter).into_iter().flatten() {
                if w.at == c.increment {
                    continue;
                }
                match w.what {
                    Written::Pointer => {
                        return Err(format!("a pointer to the loop counter `{name}` is passed to a function"));
                    }
                    _ if w.inside.contains(&l.at) => {
                        return Err(format!("the loop counter `{name}` is changed in the loop's body"));
                    }
                    Written::Constant(v) => {
                        starts.insert(v);
                    }
                    Written::NotConstant => return Err(not_constant(name)),
                }
            }
            let mut trips = 0;
            for s in starts {
                trips = trips.max(trip_count(module, f, &c, s, limits)?);
            }
            Ok(trips)
        });
        out.insert(l.at, proof);
    }
    out
}

fn not_constant(name: &str) -> String {
    format!("the loop counter `{name}` is given a value that is not a constant: its trip count is not proven")
}

fn collect_loops<'a>(block: &'a Block, out: &mut Vec<LoopAt<'a>>) {
    for s in block.iter() {
        match s {
            Statement::Loop { body, continuing, break_if } => {
                out.push(LoopAt { at: s, body, continuing, break_if: break_if.is_some() });
                collect_loops(body, out);
                collect_loops(continuing, out);
            }
            Statement::Block(b) => collect_loops(b, out),
            Statement::If { accept, reject, .. } => {
                collect_loops(accept, out);
                collect_loops(reject, out);
            }
            Statement::Switch { cases, .. } => cases.iter().for_each(|c| collect_loops(&c.body, out)),
            _ => {}
        }
    }
}

fn collect_writes(
    module: &Module,
    f: &Function,
    block: &Block,
    inside: &mut Vec<*const Statement>,
    counters: &BTreeSet<Handle<LocalVariable>>,
    out: &mut HashMap<Handle<LocalVariable>, Vec<Write>>,
) {
    let ex = &f.expressions;
    for s in block.iter() {
        match s {
            Statement::Store { pointer, value } => {
                if let Some(l) = root_local(ex, *pointer).filter(|l| counters.contains(l)) {
                    let what =
                        match (matches!(ex[*pointer], Expression::LocalVariable(_)), const_int(module, ex, *value)) {
                            (true, Some(v)) => Written::Constant(v),
                            _ => Written::NotConstant,
                        };
                    out.entry(l).or_default().push(Write { at: s, inside: inside.clone(), what });
                }
            }
            Statement::Call { arguments, .. } => {
                for a in arguments {
                    if let Some(l) = root_local(ex, *a).filter(|l| counters.contains(l)) {
                        out.entry(l).or_default().push(Write { at: s, inside: inside.clone(), what: Written::Pointer });
                    }
                }
            }
            Statement::Loop { body, continuing, .. } => {
                inside.push(s);
                collect_writes(module, f, body, inside, counters, out);
                collect_writes(module, f, continuing, inside, counters, out);
                inside.pop();
            }
            Statement::Block(b) => collect_writes(module, f, b, inside, counters, out),
            Statement::If { accept, reject, .. } => {
                collect_writes(module, f, accept, inside, counters, out);
                collect_writes(module, f, reject, inside, counters, out);
            }
            Statement::Switch { cases, .. } => {
                for c in cases {
                    collect_writes(module, f, &c.body, inside, counters, out);
                }
            }
            _ => {}
        }
    }
}

fn emitted_in(block: &Block, leading: bool) -> (BTreeSet<Handle<Expression>>, Option<&Statement>) {
    let mut set = BTreeSet::new();
    for s in block.iter() {
        match s {
            Statement::Emit(r) => set.extend(r.clone()),
            other if leading => return (set, Some(other)),
            _ => {}
        }
    }
    (set, None)
}

fn canonical(module: &Module, f: &Function, l: &LoopAt) -> Result<Counted, String> {
    if l.break_if {
        return Err(FOR_ONLY.into());
    }
    let ex = &f.expressions;
    let (pre, guard) = emitted_in(l.body, true);
    let Some(Statement::If { condition, accept, reject }) = guard else { return Err(FOR_ONLY.into()) };
    if !accept.is_empty() || reject.len() != 1 || !matches!(reject.iter().next(), Some(Statement::Break)) {
        return Err(FOR_ONLY.into());
    }
    let load_of = |e: Handle<Expression>| match ex[e] {
        Expression::Load { pointer } => match ex[pointer] {
            Expression::LocalVariable(l) => Some(l),
            _ => None,
        },
        _ => None,
    };
    let Expression::Binary { op, left, right } = ex[*condition] else { return Err(FOR_ONLY.into()) };
    let (load, counter, op, bound) = match (load_of(left), load_of(right)) {
        (Some(c), None) => (left, c, op, right),
        (None, Some(c)) => {
            let flipped = match op {
                BinaryOperator::Less => BinaryOperator::Greater,
                BinaryOperator::LessEqual => BinaryOperator::GreaterEqual,
                BinaryOperator::Greater => BinaryOperator::Less,
                BinaryOperator::GreaterEqual => BinaryOperator::LessEqual,
                _ => return Err(FOR_ONLY.into()),
            };
            (right, c, flipped, left)
        }
        _ => return Err(FOR_ONLY.into()),
    };
    if !matches!(
        op,
        BinaryOperator::Less | BinaryOperator::LessEqual | BinaryOperator::Greater | BinaryOperator::GreaterEqual
    ) {
        return Err(FOR_ONLY.into());
    }
    let mut statements = l.continuing.iter().filter(|s| !matches!(s, Statement::Emit(_)));
    let (Some(store @ Statement::Store { pointer, value }), None) = (statements.next(), statements.next()) else {
        return Err(FOR_ONLY.into());
    };
    if !matches!(ex[*pointer], Expression::LocalVariable(p) if p == counter) {
        return Err(FOR_ONLY.into());
    }
    let name = f.local_variables[counter].name.as_deref().unwrap_or("?");
    if !pre.contains(condition) || !pre.contains(&load) {
        return Err(format!(
            "the loop's condition on `{name}` is evaluated before the loop, not on every pass: its trip count is not \
             proven"
        ));
    }
    let bound = const_int(module, ex, bound).ok_or_else(|| {
        "the loop's bound is not a constant (it is computed, or read from the hook's input or a texture): its trip \
         count is not proven"
            .to_string()
    })?;
    let step_message = || {
        format!(
            "the step of the loop counter `{name}` is not a constant (`i++`, `i--`, `i += <constant>`, `i -= <constant>`)"
        )
    };
    let Expression::Binary { op: step_op, left, right } = ex[*value] else { return Err(step_message()) };
    let (cont, _) = emitted_in(l.continuing, false);
    let step = match (step_op, load_of(left) == Some(counter), load_of(right) == Some(counter)) {
        (BinaryOperator::Add, true, false) => const_int(module, ex, right).map(|s| (left, s)),
        (BinaryOperator::Add, false, true) => const_int(module, ex, left).map(|s| (right, s)),
        (BinaryOperator::Subtract, true, false) => const_int(module, ex, right).map(|s| (left, -s)),
        _ => None,
    };
    let (step_load, step) = step.ok_or_else(step_message)?;
    if !cont.contains(value) || !cont.contains(&step_load) {
        return Err(format!(
            "the step of the loop counter `{name}` is evaluated before the loop, not on every pass: its trip count \
             is not proven"
        ));
    }
    Ok(Counted { counter, op, bound, step, increment: store })
}

fn counter_range(module: &Module, f: &Function, l: Handle<LocalVariable>) -> Option<(i64, i64)> {
    use naga::{Scalar, TypeInner};
    match module.types[f.local_variables[l].ty].inner {
        TypeInner::Scalar(Scalar::I32) => Some((i64::from(i32::MIN), i64::from(i32::MAX))),
        TypeInner::Scalar(Scalar::U32) => Some((0, i64::from(u32::MAX))),
        _ => None,
    }
}

fn trip_count(module: &Module, f: &Function, c: &Counted, start: i64, limits: &Limits) -> Result<u64, String> {
    let (min, max) = counter_range(module, f, c.counter)
        .ok_or_else(|| "the loop counter must be an `i32` or a `u32`".to_string())?;
    let up = matches!(c.op, BinaryOperator::Less | BinaryOperator::LessEqual);
    if (up && c.step <= 0) || (!up && c.step >= 0) {
        return Err(format!("the counter steps by {} away from its bound: the loop would not end", c.step));
    }
    let (first, last) = match c.op {
        BinaryOperator::Less => (start, c.bound - 1),
        BinaryOperator::LessEqual => (start, c.bound),
        BinaryOperator::Greater => (c.bound + 1, start),
        _ => (c.bound, start),
    };
    if first > last {
        return Ok(0);
    }
    let step = c.step.unsigned_abs();
    let span = (last - first).unsigned_abs();
    let trips = span / step + 1;
    let moved = i64::try_from(trips).ok().and_then(|t| t.checked_mul(c.step.abs()));
    let after = moved.and_then(|m| if up { start.checked_add(m) } else { start.checked_sub(m) });
    if after.is_none_or(|a| a < min || a > max) {
        return Err(format!("the counter would wrap past its type's range after {trips} passes"));
    }
    if trips > limits.max_trips {
        return Err(format!("{trips} passes, over the cap of {} per loop", limits.max_trips));
    }
    Ok(trips)
}
