use std::fmt::Write as _;

use super::{API_LEVEL, RECORDS, Ty, specs};
use crate::host::Side;
use crate::host::events::HOST_EVENTS;
use crate::host::game::keys::BINDABLE;

pub const HANDLE_TYPES: [&str; 3] = ["Element", "Timer", "LatentHandle"];

pub fn type_text(ty: &Ty) -> String {
    match ty {
        Ty::Nil => "nil".into(),
        Ty::Bool => "boolean".into(),
        Ty::Number { .. } | Ty::Int { .. } => "number".into(),
        Ty::Str { .. } | Ty::Name(_) => "string".into(),
        Ty::Enum(options) => options.iter().map(|o| format!("{o:?}")).collect::<Vec<_>>().join(" | "),
        Ty::Element => "Element".into(),
        Ty::Elements => "Element | { Element }".into(),
        Ty::Function => "(...any) -> ...any".into(),
        Ty::Timer => "Timer".into(),
        Ty::Latent => "LatentHandle".into(),
        Ty::Value => "NetValue".into(),
        Ty::Any => "any".into(),
        Ty::Opt(inner) => {
            let t = type_text(inner);
            if t.contains(' ') { format!("({t})?") } else { format!("{t}?") }
        }
        Ty::List(inner) => format!("{{ {} }}", type_text(inner)),
        Ty::Record(name) => (*name).into(),
        Ty::Vec3 { .. } => "Vec3".into(),
    }
}

fn named_text(ty: &Ty) -> String {
    match ty {
        Ty::Enum(options) if *options == BINDABLE => "BindableKey".into(),
        Ty::Opt(inner @ Ty::Enum(options)) if *options == BINDABLE => format!("{}?", named_text(inner)),
        other => type_text(other),
    }
}

fn handler_alias(event: &str) -> String {
    let mut c = event.chars();
    c.next().map_or(String::new(), |f| f.to_ascii_uppercase().to_string() + c.as_str() + "Handler")
}

fn add_event_handler(side: Side) -> String {
    let mut overloads: Vec<String> = HOST_EVENTS
        .iter()
        .filter(|e| e.side.has(side))
        .map(|e| {
            format!(
                "((name: {:?}, element: Element, handler: {}, propagate: boolean?) -> boolean)",
                e.name,
                handler_alias(e.name)
            )
        })
        .collect();
    overloads
        .push("((name: string, element: Element, handler: (...any) -> ...any, propagate: boolean?) -> boolean)".into());
    format!("declare addEventHandler: {}\n", overloads.join(" & "))
}

fn params(spec: &super::Spec) -> String {
    let mut ps: Vec<String> = spec.params.iter().map(|p| format!("{}: {}", p.name, named_text(&p.ty))).collect();
    if let Some(rest) = &spec.rest {
        ps.push(format!("...: {}", type_text(rest)));
    }
    ps.join(", ")
}

fn returns(spec: &super::Spec) -> String {
    if spec.any_results {
        return "...any".into();
    }
    match spec.returns {
        [] => "()".into(),
        [one] => type_text(one),
        many => format!("({})", many.iter().map(type_text).collect::<Vec<_>>().join(", ")),
    }
}

pub fn generate(side: Side, api: u32) -> String {
    let mut out = String::new();
    for h in HANDLE_TYPES {
        let _ = writeln!(out, "declare extern type {h} with\nend\n");
    }
    out.push_str("type NetValue = nil | boolean | number | string | Element | { [any]: any }\n");
    if side == Side::Client {
        let _ = writeln!(out, "type BindableKey = {}", type_text(&Ty::Enum(BINDABLE)));
    }
    for (name, fields) in RECORDS {
        let fs: Vec<String> = fields.iter().map(|(f, t)| format!("{f}: {}", named_text(t))).collect();
        let _ = writeln!(out, "type {name} = {{ {} }}", fs.join(", "));
    }
    for e in HOST_EVENTS.iter().filter(|e| e.side.has(side)) {
        let ps: Vec<String> = e.params.iter().map(|p| format!("{}: {}", p.name, named_text(&p.ty))).collect();
        let _ = writeln!(out, "type {} = ({}) -> ()", handler_alias(e.name), ps.join(", "));
    }
    out.push('\n');
    let mut globals = vec![
        ("root", "Element"),
        ("resourceRoot", "Element"),
        ("source", "Element"),
        ("this", "Element"),
        ("eventName", "string"),
    ];
    match side {
        Side::Server => globals.push(("client", "Element?")),
        Side::Client => globals.push(("localPlayer", "Element?")),
    }
    for (name, ty) in globals {
        let _ = writeln!(out, "declare {name}: {ty}");
    }
    out.push_str(
        "declare exports: ((name: string, fn: (...any) -> ...any) -> ()) & { [string]: { [string]: (self: any, ...any) -> ...any } }\n\n",
    );
    let all: Vec<_> = specs(side, api).collect();
    for spec in all.iter().filter(|s| !s.name.contains('.')) {
        if spec.name == "addEventHandler" {
            out.push_str(&add_event_handler(side));
            continue;
        }
        let _ = writeln!(out, "declare function {}({}): {}", spec.name, params(spec), returns(spec));
    }
    let mut libs: Vec<&str> = all.iter().filter_map(|s| s.name.split_once('.').map(|(l, _)| l)).collect();
    libs.sort_unstable();
    libs.dedup();
    for lib in libs {
        let members: Vec<String> = all
            .iter()
            .filter_map(|s| {
                let (l, m) = s.name.split_once('.')?;
                (l == lib).then(|| format!("    {m}: ({}) -> {},", params(s), returns(s)))
            })
            .collect();
        let _ = writeln!(out, "\ndeclare {lib}: {{\n{}\n}}", members.join("\n"));
    }
    out
}

pub fn file_name(side: Side) -> String {
    format!("ssx-api{API_LEVEL}-{}.d.luau", side.name())
}
