use super::one;
use crate::value::Value;
use crate::vm::{Lua, LuaResult};

// TODO(guess): the game's `rand` state is the runtime's (per thread, shared with other C++ callers), so its draws depend on what else called it; settle: trace `rand`'s callers in the exe. The generator is Microsoft's (x·214013 + 2531011, bits 16…30).
#[derive(Clone, Debug)]
pub struct CRand(pub u32);

impl CRand {
    pub fn draw(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.0 >> 16) & 0x7FFF
    }
}

thread_local! {
    static RAND: std::cell::RefCell<CRand> = const { std::cell::RefCell::new(CRand(1)) };
}

fn f1(l: &mut Lua, name: &str, f: fn(f64) -> f64) {
    let t = l.lib_table("math");
    l.lib_fn(t, name, move |l, a| one(f(l.check_num(a, 1)?)));
}

pub fn open(l: &mut Lua) {
    let t = l.lib_table("math");
    f1(l, "abs", f64::abs);
    f1(l, "acos", f64::acos);
    f1(l, "asin", f64::asin);
    f1(l, "atan", f64::atan);
    f1(l, "ceil", f64::ceil);
    f1(l, "cosh", f64::cosh);
    f1(l, "cos", f64::cos);
    f1(l, "deg", |x| x / (std::f64::consts::PI / 180.0));
    f1(l, "exp", f64::exp);
    f1(l, "floor", f64::floor);
    f1(l, "log10", f64::log10);
    f1(l, "log", f64::ln);
    f1(l, "rad", |x| x * (std::f64::consts::PI / 180.0));
    f1(l, "sinh", f64::sinh);
    f1(l, "sin", f64::sin);
    f1(l, "sqrt", f64::sqrt);
    f1(l, "tanh", f64::tanh);
    f1(l, "tan", f64::tan);
    l.lib_fn(t, "atan2", |l, a| one(l.check_num(a, 1)?.atan2(l.check_num(a, 2)?)));
    l.lib_fn(t, "fmod", |l, a| one(crate::number::fmod(l.check_num(a, 1)?, l.check_num(a, 2)?)));
    l.lib_fn(t, "pow", |l, a| one(l.check_num(a, 1)?.powf(l.check_num(a, 2)?)));
    l.lib_fn(t, "modf", |l, a| {
        let x = l.check_num(a, 1)?;
        let ip = if x.is_infinite() { x } else { x.trunc() };
        let fp = if x.is_infinite() { 0.0f64.copysign(x) } else { x - ip };
        Ok(vec![Value::Num(ip), Value::Num(fp)])
    });
    l.lib_fn(t, "frexp", |l, a| {
        let x = l.check_num(a, 1)?;
        let (m, e) = frexp(x);
        Ok(vec![Value::Num(m), Value::Num(f64::from(e))])
    });
    l.lib_fn(t, "ldexp", |l, a| {
        let m = l.check_num(a, 1)?;
        let e = l.check_int(a, 2)?;
        one(m * 2f64.powi(e))
    });
    l.lib_fn(t, "max", |l, a| minmax(l, a, true));
    l.lib_fn(t, "min", |l, a| minmax(l, a, false));
    l.lib_fn(t, "random", |l, a| {
        let r = RAND.with(|g| g.borrow_mut().draw());
        let r = f64::from(r % 0x7FFF) / f64::from(0x7FFF);
        match a.len() {
            0 => one(r),
            1 => {
                let u = l.check_int(a, 1)?;
                if u < 1 {
                    return Err(l.arg_error(1, "interval is empty"));
                }
                one((r * f64::from(u)).floor() + 1.0)
            }
            2 => {
                let lo = l.check_int(a, 1)?;
                let up = l.check_int(a, 2)?;
                if lo > up {
                    return Err(l.arg_error(2, "interval is empty"));
                }
                one((r * (f64::from(up) - f64::from(lo) + 1.0)).floor() + f64::from(lo))
            }
            _ => Err(l.rt_error("wrong number of arguments")),
        }
    });
    l.lib_fn(t, "randomseed", |l, a| {
        let s = l.check_int(a, 1)?;
        RAND.with(|g| g.borrow_mut().0 = s as u32);
        Ok(Vec::new())
    });
    let _ = l.heap.table_mut(t).set(Value::str("pi"), Value::Num(std::f64::consts::PI));
    let _ = l.heap.table_mut(t).set(Value::str("huge"), Value::Num(f64::INFINITY));
}

fn minmax(l: &mut Lua, a: &[Value], max: bool) -> LuaResult<Vec<Value>> {
    let mut m = l.check_num(a, 1)?;
    for i in 2..=a.len() {
        let d = l.check_num(a, i)?;
        if (max && d > m) || (!max && d < m) {
            m = d;
        }
    }
    one(m)
}

fn frexp(x: f64) -> (f64, i32) {
    if x == 0.0 || !x.is_finite() {
        return (x, 0);
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7FF) as i32;
    if exp == 0 {
        let (m, e) = frexp(x * 2f64.powi(54));
        return (m, e - 54);
    }
    let e = exp - 1022;
    let m = f64::from_bits((bits & !(0x7FF << 52)) | (1022 << 52));
    (m, e)
}
