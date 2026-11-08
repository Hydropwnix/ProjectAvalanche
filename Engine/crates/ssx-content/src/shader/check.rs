use std::collections::{BTreeMap, BTreeSet};

use naga::valid::ModuleInfo;
use naga::{Handle, Module, Type, TypeInner};

use super::{
    Accepted, Composed, Cost, HookFamily, INTERFACE_TYPES, Limits, Problem, VERTEX_SIGNATURE, WGSL_FUNCTION_MEMORY,
    analysis,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle_ {
    Texture,
    Sampler,
}

fn surface_handles(family: HookFamily) -> &'static [Handle_] {
    match family {
        HookFamily::Object => {
            &[Handle_::Texture, Handle_::Sampler, Handle_::Texture, Handle_::Texture, Handle_::Sampler]
        }
        HookFamily::Sky => &[Handle_::Texture, Handle_::Sampler],
    }
}

fn prelude_lines(family: HookFamily) -> usize {
    family.types().lines().count() + 1
}

pub fn check_hook(family: HookFamily, bytes: &[u8], limits: &Limits) -> Result<Accepted, Vec<Problem>> {
    if bytes.len() > limits.max_bytes {
        return Err(vec![(0, 0, format!("{} bytes, over the shader cap of {}", bytes.len(), limits.max_bytes))]);
    }
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    if text.contains('\0') {
        return Err(vec![(0, 0, "a NUL character".into())]);
    }
    let source = format!("{}\n{text}", family.types());
    let offset = prelude_lines(family);
    let at = |span: naga::Span| -> (usize, usize) {
        match span.to_range() {
            Some(r) => {
                let (line, col) = crate::manifest::line_col(&source, r.start);
                (line.saturating_sub(offset), col)
            }
            None => (0, 0),
        }
    };
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| {
        let (line, col) = e.labels().next().map_or((0, 0), |(span, _)| at(span));
        let message = if e.message().contains("global directive") {
            "a directive (`enable`, `requires`, `diagnostic`): a hook uses no device feature and cannot switch a \
             check off"
                .to_string()
        } else {
            format!("not valid WGSL: {}", ssx_core::text::message(e.message()))
        };
        vec![(line, col, message)]
    })?;
    let mut problems = Vec::new();
    rules(&module, &mut problems, &at);
    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty());
    let info = match validator.validate(&module) {
        Ok(info) => info,
        Err(e) => {
            let (line, col) =
                e.spans().filter(|(span, _)| span.to_range().is_some()).last().map_or((0, 0), |(s, _)| at(*s));
            let mut message = format!("not valid WGSL: {}", e.as_inner());
            let mut cause = std::error::Error::source(e.as_inner());
            while let Some(c) = cause {
                message.push_str(&format!(": {c}"));
                cause = c.source();
            }
            problems.push((line, col, ssx_core::text::message(&message).into_string()));
            return Err(problems);
        }
    };
    if !problems.is_empty() {
        return Err(problems);
    }
    let functions: BTreeSet<Handle<naga::Function>> = module.functions.iter().map(|(h, _)| h).collect();
    let hook = analysis::Hook::new(&module, &info, &functions, limits);
    let mut hooks = Hooks::default();
    for (h, f) in module.functions.iter() {
        let span = module.functions.get_span(h);
        let name = f.name.as_deref().unwrap_or("");
        let ok = match name {
            "surface" => surface_signature(&module, f, family),
            "vertex_offset" => vertex_signature(&module, f),
            _ => {
                if let Err((span, m)) = hook.cost(h) {
                    let (line, col) = at(span);
                    problems.push((line, col, m));
                }
                continue;
            }
        };
        if !ok {
            let want = if name == "surface" { family.surface_signature() } else { VERTEX_SIGNATURE };
            let (line, col) = at(span);
            problems.push((line, col, format!("`{name}` must be `{want}`")));
            continue;
        }
        match hook.cost(h).and_then(|c| within(name, c, limits).map_err(|m| (span, m))) {
            Ok(c) => hooks.set(name, c),
            Err((span, m)) => {
                let (line, col) = at(span);
                problems.push((line, col, m));
            }
        }
    }
    if hooks.surface.is_none() && hooks.vertex.is_none() && problems.is_empty() {
        problems.push((0, 0, format!("defines no hook: `{}` or `{VERTEX_SIGNATURE}`", family.surface_signature())));
    }
    if !problems.is_empty() {
        problems.sort();
        problems.dedup();
        return Err(problems);
    }
    Ok(Accepted { family, surface: hooks.surface, vertex: hooks.vertex })
}

#[derive(Default)]
struct Hooks {
    surface: Option<Cost>,
    vertex: Option<Cost>,
}

impl Hooks {
    fn set(&mut self, name: &str, c: Cost) {
        if name == "surface" {
            self.surface = Some(c);
        } else {
            self.vertex = Some(c);
        }
    }
}

fn within(name: &str, c: Cost, limits: &Limits) -> Result<Cost, String> {
    if c.operations > limits.max_operations {
        return Err(format!(
            "`{name}`: at most {} scalar operations per invocation, over the cap of {}",
            c.operations, limits.max_operations
        ));
    }
    if c.texture_reads > limits.max_texture_reads {
        return Err(format!(
            "`{name}`: at most {} texture reads per invocation, over the cap of {}",
            c.texture_reads, limits.max_texture_reads
        ));
    }
    Ok(c)
}

pub fn check_composed(
    family: HookFamily,
    module: &Module,
    info: &ModuleInfo,
    hook_item: &dyn Fn(&str) -> Option<String>,
    limits: &Limits,
) -> Result<Composed, Vec<String>> {
    let mut items: BTreeMap<Handle<naga::Function>, String> = BTreeMap::new();
    for (h, f) in module.functions.iter() {
        if let Some(item) = f.name.as_deref().and_then(hook_item) {
            items.insert(h, item);
        }
    }
    let functions: BTreeSet<Handle<naga::Function>> = items.keys().copied().collect();
    let hook = analysis::Hook::new(module, info, &functions, limits);
    let mut problems = Vec::new();
    let mut hooks = Hooks::default();
    for (h, item) in &items {
        match hook.cost(*h) {
            Ok(c) if matches!(item.as_str(), "surface" | "vertex_offset") => match within(item, c, limits) {
                Ok(c) => hooks.set(item, c),
                Err(m) => problems.push(m),
            },
            Ok(_) => {}
            Err((_, m)) => problems.push(format!("`{item}` as composed: {m}")),
        }
    }
    let mut memo = BTreeMap::new();
    let mut work_memo = BTreeMap::new();
    let mut entry_frames = BTreeMap::new();
    let mut entry_work = BTreeMap::new();
    for ep in &module.entry_points {
        entry_work
            .insert(ep.name.clone(), analysis::straight_function(module, info, &ep.function, None, &mut work_memo));
        let own: u64 = ep
            .function
            .local_variables
            .iter()
            .map(|(_, l)| analysis::value_bytes(module, &module.types[l.ty].inner))
            .fold(0, u64::saturating_add);
        let mut callees = BTreeSet::new();
        analysis::calls_in(&ep.function.body, &mut callees);
        let deepest = callees.into_iter().map(|c| analysis::frame_bytes(module, c, &mut memo)).max().unwrap_or(0);
        let bytes = own.saturating_add(deepest);
        if bytes > WGSL_FUNCTION_MEMORY {
            problems.push(format!(
                "the family's entry point `{}` with the hook holds {bytes} bytes of function memory, over WGSL's {}",
                ep.name, WGSL_FUNCTION_MEMORY
            ));
        }
        entry_frames.insert(ep.name.clone(), bytes);
    }
    if !problems.is_empty() {
        problems.sort();
        problems.dedup();
        return Err(problems);
    }
    Ok(Composed {
        accepted: Accepted { family, surface: hooks.surface, vertex: hooks.vertex },
        entry_frames,
        entry_work,
    })
}

fn rules(module: &Module, problems: &mut Vec<Problem>, at: &dyn Fn(naga::Span) -> (usize, usize)) {
    let mut push = |span: naga::Span, m: String| {
        let (line, col) = at(span);
        problems.push((line, col, m));
    };
    for ep in &module.entry_points {
        push(naga::Span::default(), format!("entry point `{}`: a hook module has no entry points", ep.name));
    }
    for (h, g) in module.global_variables.iter() {
        let name = g.name.as_deref().unwrap_or("?");
        let what = if g.binding.is_some() { "a binding" } else { "a module-scope variable" };
        push(module.global_variables.get_span(h), format!("`{name}`: {what}; a hook uses the family's bindings only"));
    }
    for (h, o) in module.overrides.iter() {
        let name = o.name.as_deref().unwrap_or("?");
        push(
            module.overrides.get_span(h),
            format!("override `{name}`: pipeline-overridable constants are not allowed"),
        );
    }
    if module.diagnostic_filter_leaf.is_some() || !module.diagnostic_filters.is_empty() {
        push(naga::Span::default(), "a `diagnostic` filter: the checks cannot be switched off".into());
    }
    for (h, ty) in module.types.iter() {
        let span = module.types.get_span(h);
        if let Some(why) = forbidden_type(&ty.inner) {
            let name = ty.name.as_deref().map_or(String::new(), |n| format!("`{n}`: "));
            push(span, format!("{name}{why}"));
        }
        if let (Some(name), TypeInner::Struct { .. }) = (&ty.name, &ty.inner)
            && !INTERFACE_TYPES.contains(&name.as_str())
            && name.starts_with("__")
        {
            push(span, format!("`{name}`: names starting with `__` are reserved"));
        }
    }
    for (h, f) in module.functions.iter() {
        let span = module.functions.get_span(h);
        let name = f.name.as_deref().unwrap_or("?");
        if f.arguments.iter().any(|a| a.binding.is_some()) || f.result.as_ref().is_some_and(|r| r.binding.is_some()) {
            push(span, format!("`{name}`: a built-in or location attribute (hooks are no entry points)"));
        }
    }
}

pub(super) fn forbidden_type(inner: &TypeInner) -> Option<String> {
    use naga::{AddressSpace, ImageClass, Scalar, ScalarKind};
    let scalar = |s: Scalar| match (s.kind, s.width) {
        (ScalarKind::Float, 2) => Some("f16 (a device feature)".to_string()),
        (_, 8) => Some("a 64-bit type (a device feature)".to_string()),
        _ => None,
    };
    match inner {
        TypeInner::Scalar(s) | TypeInner::Vector { scalar: s, .. } | TypeInner::Matrix { scalar: s, .. } => scalar(*s),
        TypeInner::Atomic(_) => Some("an atomic type (storage only)".into()),
        TypeInner::Pointer { space, .. } | TypeInner::ValuePointer { space, .. } => match space {
            AddressSpace::Function | AddressSpace::Private => None,
            other => Some(format!("a pointer into {other:?} memory (bindings only)")),
        },
        TypeInner::Image { class: ImageClass::Storage { .. }, .. } => Some("a storage texture".into()),
        TypeInner::Image { class: ImageClass::External, .. } => Some("an external texture".into()),
        TypeInner::BindingArray { .. } => Some("a binding array".into()),
        TypeInner::AccelerationStructure { .. } | TypeInner::RayQuery { .. } => Some("ray tracing".into()),
        TypeInner::CooperativeMatrix { .. } => Some("a cooperative matrix (a device feature)".into()),
        _ => None,
    }
}

fn struct_named(module: &Module, ty: Handle<Type>, name: &str) -> bool {
    let t = &module.types[ty];
    t.name.as_deref() == Some(name) && matches!(t.inner, TypeInner::Struct { .. })
}

fn is_handle(module: &Module, ty: Handle<Type>, want: Handle_) -> bool {
    use naga::{ImageClass, ImageDimension, ScalarKind};
    matches!(
        (&module.types[ty].inner, want),
        (
            TypeInner::Image {
                dim: ImageDimension::D2,
                arrayed: false,
                class: ImageClass::Sampled { kind: ScalarKind::Float, multi: false },
            },
            Handle_::Texture,
        ) | (TypeInner::Sampler { comparison: false }, Handle_::Sampler)
    )
}

fn surface_signature(module: &Module, f: &naga::Function, family: HookFamily) -> bool {
    let handles = surface_handles(family);
    f.arguments.len() == 2 + handles.len()
        && struct_named(module, f.arguments[0].ty, "SurfaceIn")
        && struct_named(module, f.arguments[1].ty, "Surface")
        && f.arguments[2..].iter().zip(handles).all(|(a, h)| is_handle(module, a.ty, *h))
        && f.result.as_ref().is_some_and(|r| struct_named(module, r.ty, "Surface"))
}

fn vertex_signature(module: &Module, f: &naga::Function) -> bool {
    use naga::{Scalar, VectorSize};
    f.arguments.len() == 1
        && struct_named(module, f.arguments[0].ty, "VertexIn")
        && f.result.as_ref().is_some_and(|r| {
            matches!(module.types[r.ty].inner, TypeInner::Vector { size: VectorSize::Tri, scalar: Scalar::F32 })
        })
}
