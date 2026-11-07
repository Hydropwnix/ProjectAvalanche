use super::Ctx;
use super::value::Value;

pub const ARG_BYTES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arg {
    Str,
    StrOrNone,
    Num,
    NumOrNone,
    Any,
}

fn contract(object: &str, method: &str) -> Option<&'static [Arg]> {
    use Arg::*;
    Some(match (object, method) {
        ("LocalDataStore", "Subscribe" | "Unsubscribe" | "BindData" | "UnbindData") => &[Str, Str],
        ("Dispatcher", "DispatchEvent" | "DispatchQueryEvent") => &[NumOrNone, StrOrNone],
        ("Dispatcher", "DataAction") => &[Str, Str, Any],
        ("Dispatcher", "Action") => &[Str, Any],
        ("Dispatcher", "RegisterNavActionHandler" | "UnRegisterNavActionHandler") => &[StrOrNone],
        ("InputProvider", "Subscribe" | "Unsubscribe") => &[Str],
        ("ResourceHandler", "AcquireResource") => &[Str, Num, Str],
        _ => return None,
    })
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Undefined => "undefined",
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Object(_) => "an object",
    }
}

pub(crate) fn check(cx: &mut Ctx, object: &str, method: &str, a: &[Value]) -> Result<(), String> {
    let Some(args) = contract(object, method) else { return Err("not a call of the game's".into()) };
    for (i, want) in args.iter().enumerate() {
        let v = a.get(i).unwrap_or(&Value::Undefined);
        let ok = match (want, v) {
            (Arg::Str | Arg::StrOrNone, Value::String(s)) => s.len() <= ARG_BYTES,
            (Arg::StrOrNone, Value::Undefined | Value::Null) => true,
            (Arg::Num | Arg::NumOrNone, Value::Number(n)) => n.is_finite(),
            (Arg::NumOrNone, Value::Undefined | Value::Null) => true,
            (Arg::Any, Value::String(s)) => s.len() <= ARG_BYTES,
            (Arg::Any, Value::Number(n)) => n.is_finite(),
            (Arg::Any, _) => true,
            _ => false,
        };
        if !ok {
            let what = match v {
                Value::String(s) if s.len() > ARG_BYTES => {
                    format!("a string of {} bytes (at most {ARG_BYTES})", s.len())
                }
                Value::Number(n) if !n.is_finite() => "a number that is not finite".to_string(),
                other => kind(other).to_string(),
            };
            return Err(format!("argument {} is {what}", i + 1));
        }
    }
    let Some(m) = cx.vm.sandbox.charged else { return Ok(()) };
    let Some(origin) = cx.lib.origin(m).cloned() else { return Ok(()) };
    let read_only = matches!(object, "LocalDataStore" | "InputProvider" | "ResourceHandler");
    if read_only {
        return Ok(());
    }
    let s = |i: usize| match a.get(i) {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    };
    let names = if origin.id.namespace() == ssx_content::BASE_NAMESPACE {
        cx.lib.files().content().and_then(|c| c.disc_movie_strings(&origin.id))
    } else {
        None
    };
    let Some(names) = names else {
        return Err(format!(
            "{} ({}) may not call {object}.{method}: an added movie (or a port view's replacement) makes the read-only \
             calls only",
            origin.id, origin.source
        ));
    };
    let named = |n: &str| names.contains(n);
    let ok = match method {
        "DataAction" | "Action" => true,
        "DispatchEvent" | "DispatchQueryEvent" => {
            let e = match a.first() {
                Some(Value::Number(n)) => *n,
                _ => f64::NAN,
            };
            let lifecycle = [super::runtime::nav::POST_ON_ENTER_VIEW, super::runtime::nav::POST_ON_EXIT_VIEW]
                .iter()
                .any(|&k| f64::from(k) == e);
            lifecycle || named(&s(1))
        }
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{} ({}) may not call {object}.{method} with {:?}: its disc screen's code names no such nav event",
            origin.id,
            origin.source,
            super::cut(&s(1), 64)
        ))
    }
}
