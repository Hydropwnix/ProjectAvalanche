use super::{Builder, R, arg};
use crate::avm1::Ctx;
use crate::avm1::object::{Kind, ObjId};
use crate::avm1::value::Value;

pub(crate) fn install(b: &mut Builder, global: ObjId) {
    let mouse = b.obj();
    b.value(global, "Mouse", Value::Object(mouse));
    b.methods(
        mouse,
        &[
            ("hide", |_, _, _| Ok(Value::Number(1.0))),
            ("show", |_, _, _| Ok(Value::Number(0.0))),
            ("addListener", add_listener),
            ("removeListener", remove_listener),
        ],
    );
    let key = b.obj();
    b.value(global, "Key", Value::Object(key));
    b.methods(
        key,
        &[
            ("isDown", |_, _, _| Ok(Value::Bool(false))),
            ("isToggled", |_, _, _| Ok(Value::Bool(false))),
            ("getCode", |_, _, _| Ok(Value::Number(0.0))),
            ("getAscii", |_, _, _| Ok(Value::Number(0.0))),
            ("addListener", add_listener),
            ("removeListener", remove_listener),
        ],
    );
    for (n, v) in [
        ("BACKSPACE", 8),
        ("TAB", 9),
        ("ENTER", 13),
        ("SHIFT", 16),
        ("CONTROL", 17),
        ("ESCAPE", 27),
        ("SPACE", 32),
        ("PGUP", 33),
        ("PGDN", 34),
        ("END", 35),
        ("HOME", 36),
        ("LEFT", 37),
        ("UP", 38),
        ("RIGHT", 39),
        ("DOWN", 40),
        ("INSERT", 45),
        ("DELETEKEY", 46),
    ] {
        b.value(key, n, Value::from(v));
    }
    let stage = b.obj();
    b.value(global, "Stage", Value::Object(stage));
    b.value(stage, "scaleMode", Value::str("noBorder"));
    b.value(stage, "align", Value::str(""));
    b.value(stage, "width", Value::Number(1280.0));
    b.value(stage, "height", Value::Number(720.0));
    b.value(stage, "showMenu", Value::Bool(false));
    b.methods(stage, &[("addListener", add_listener), ("removeListener", remove_listener)]);
}

pub(crate) fn add_listener(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let l = arg(a, 0);
    let list = match cx.get_member(&this, "_listeners")? {
        v @ Value::Object(_) if matches!(cx.kind_of(&v), Some(Kind::Array(_))) => v,
        _ => {
            let arr = Value::Object(cx.new_array(Vec::new()));
            if let Some(o) = this.as_object() {
                cx.define(o, "_listeners", arr.clone(), crate::avm1::object::flags::DONT_ENUM);
            }
            arr
        }
    };
    if let Some(o) = list.as_object().and_then(|o| cx.vm.heap.get_mut(o))
        && let Kind::Array(v) = &mut o.kind
        && !v.iter().any(|x| x.same(&l))
    {
        v.push(l);
    }
    Ok(Value::Bool(true))
}

pub(crate) fn remove_listener(cx: &mut Ctx, this: Value, a: &[Value]) -> R {
    let l = arg(a, 0);
    let list = cx.get_member(&this, "_listeners")?;
    let mut removed = false;
    if let Some(o) = list.as_object().and_then(|o| cx.vm.heap.get_mut(o))
        && let Kind::Array(v) = &mut o.kind
        && let Some(i) = v.iter().position(|x| x.same(&l))
    {
        v.remove(i);
        removed = true;
    }
    Ok(Value::Bool(removed))
}
