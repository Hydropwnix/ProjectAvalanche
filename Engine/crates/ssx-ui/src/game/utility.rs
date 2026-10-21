use super::loc::Localization;
use crate::avm1::Ctx;
use crate::avm1::value::Value;

pub fn count_steps(start: i32, end: i32, static_length: bool, length: i32) -> Vec<i32> {
    let range = (end - start) as f32;
    let (steps, inc) = if static_length {
        (length, range / length as f32)
    } else {
        ((range / length as f32).ceil() as i32, length as f32)
    };
    let mut out = vec![start];
    let mut v = start as f32;
    for _ in 1..steps {
        v += inc;
        out.push(v as i32);
    }
    out.push(end);
    out
}

pub fn count_list(
    loc: &mut Localization,
    start: i32,
    end: i32,
    static_length: bool,
    length: i32,
    symbol: bool,
) -> Vec<String> {
    count_steps(start, end, static_length, length).into_iter().map(|v| loc.currency(i64::from(v), symbol)).collect()
}

pub fn currency_count_anim_data(cx: &mut Ctx, loc: &mut Localization, requests: &Value) -> Value {
    let n = cx.get_member(requests, "length").map(|v| cx.to_number(&v)).unwrap_or(0.0) as usize;
    let mut items = Vec::new();
    for k in 0..n {
        let r = cx.get_member(requests, &k.to_string()).unwrap_or_default();
        let mut num = |name: &str| {
            let v = cx.get_member(&r, name).unwrap_or_default();
            cx.to_number(&v)
        };
        let (start, end, length) = (num("startValue") as i32, num("endValue") as i32, num("lengthOrIncrement") as i32);
        let flag = |cx: &mut Ctx, name: &str| {
            let v = cx.get_member(&r, name).unwrap_or_default();
            cx.to_bool(&v)
        };
        let (static_length, symbol) = (flag(cx, "isStaticLength"), flag(cx, "includeSymbol"));
        let id = cx.get_member(&r, "id").unwrap_or_default();
        let list: Vec<Value> =
            count_list(loc, start, end, static_length, length, symbol).iter().map(|s| Value::str(s)).collect();
        let o = Value::Object(cx.new_object());
        let arr = Value::Object(cx.new_array(list));
        let _ = cx.set_member(&o, "countList", arr);
        let _ = cx.set_member(&o, "id", id);
        items.push(o);
    }
    Value::Object(cx.new_array(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_of_both_modes() {
        assert_eq!(count_steps(0, 100, true, 4), vec![0, 25, 50, 75, 100]);
        assert_eq!(count_steps(0, 1000, false, 300), vec![0, 300, 600, 900, 1000]);
        assert_eq!(count_steps(5, 5, true, 1), vec![5, 5]);
        assert_eq!(count_steps(0, 10, true, 3), vec![0, 3, 6, 10]);
    }
}
