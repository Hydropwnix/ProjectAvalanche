pub fn to_int(n: f64) -> i32 {
    if n.is_nan() { i32::MIN } else { n as i32 }
}

pub fn strtoul(s: &[u8], base: u32) -> (u32, usize) {
    let mut i = 0;
    while i < s.len() && crate::lexer::is_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let mut base = base;
    if (base == 16 || base == 0) && i + 1 < s.len() && s[i] == b'0' && (s[i + 1] | 0x20) == b'x' {
        let ok = s.get(i + 2).is_some_and(|c| (*c as char).is_ascii_hexdigit());
        if ok {
            i += 2;
            base = 16;
        }
    }
    if base == 0 {
        base = if s.get(i) == Some(&b'0') { 8 } else { 10 };
    }
    let start = i;
    let mut v: u64 = 0;
    let mut over = false;
    while let Some(d) = s.get(i).and_then(|c| (*c as char).to_digit(36)).filter(|d| *d < base) {
        v = v * u64::from(base) + u64::from(d);
        if v > u64::from(u32::MAX) {
            over = true;
            v = u64::from(u32::MAX);
        }
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    let v = if over {
        u32::MAX
    } else if neg {
        (v as u32).wrapping_neg()
    } else {
        v as u32
    };
    (v, i)
}

// TODO(guess): the Xbox 360 runtime's `strtod` is assumed to round correctly and to know no `inf` / `nan` / hex forms (the pre-C99 Microsoft runtime); settle: run 0x82A9DB98's callee.
pub fn strtod(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while i < s.len() && crate::lexer::is_space(s[i]) {
        i += 1;
    }
    let start = i;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        i += 1;
    }
    let digits0 = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let mut ndig = i - digits0;
    if i < s.len() && s[i] == b'.' {
        i += 1;
        let f0 = i;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
        ndig += i - f0;
    }
    if ndig == 0 {
        return (0.0, 0);
    }
    if i < s.len() && (s[i] | 0x20) == b'e' {
        let mut j = i + 1;
        if j < s.len() && (s[j] == b'+' || s[j] == b'-') {
            j += 1;
        }
        let e0 = j;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
        }
        if j > e0 {
            i = j;
        }
    }
    let text = std::str::from_utf8(&s[start..i]).unwrap_or("0");
    let text = text.strip_suffix('.').unwrap_or(text);
    let v = if text.starts_with('.') || text.starts_with("-.") || text.starts_with("+.") {
        text.replacen('.', "0.", 1).parse::<f64>()
    } else {
        text.parse::<f64>()
    };
    (v.unwrap_or(0.0), i)
}

pub fn str2d(s: &[u8]) -> Option<f64> {
    let (mut v, mut end) = strtod(s);
    if end == 0 {
        return None;
    }
    if matches!(s.get(end), Some(b'x') | Some(b'X')) {
        let (u, e) = strtoul(s, 16);
        v = f64::from(u);
        end = e;
    }
    while end < s.len() && crate::lexer::is_space(s[end]) {
        end += 1;
    }
    (end == s.len()).then_some(v)
}

// TODO(guess): which NaN prints as `#IND` vs `#QNAN`, and that precision does not truncate the text under `%.14g`; settle: run the runtime's `_cftog` path (0x83237B20) on these values.
fn special(n: f64) -> Option<String> {
    if n.is_finite() {
        return None;
    }
    let neg = n.is_sign_negative();
    let body = if n.is_infinite() {
        "1.#INF"
    } else if n.to_bits() == 0xFFF8_0000_0000_0000 {
        "1.#IND"
    } else {
        "1.#QNAN"
    };
    Some(if neg { format!("-{body}") } else { body.to_string() })
}

// TODO(guess): the Xbox 360 runtime is assumed to print three exponent digits (`1e+020`) like the Microsoft runtimes before VS2015; settle: run `sprintf` 0x828837C0 on 1e20.
pub const EXP_DIGITS: usize = 3;

pub fn fmt_e(n: f64, prec: usize, upper: bool) -> String {
    let s = format!("{:.*e}", prec, n);
    let (mant, exp) = s.split_once('e').unwrap_or((&s, "0"));
    let e: i32 = exp.parse().unwrap_or(0);
    let mut out = format!(
        "{mant}{}{}{:0w$}",
        if upper { 'E' } else { 'e' },
        if e < 0 { '-' } else { '+' },
        e.abs(),
        w = EXP_DIGITS
    );
    if upper {
        out = out.to_uppercase();
    }
    out
}

pub fn fmt_f(n: f64, prec: usize) -> String {
    format!("{:.*}", prec, n)
}

pub fn fmt_float(n: f64, conv: u8, prec: usize, alt: bool) -> String {
    if let Some(s) = special(n) {
        return s;
    }
    let s = match conv {
        b'e' | b'E' => fmt_e(n, prec, conv == b'E'),
        b'f' => fmt_f(n, prec),
        _ => fmt_g(n, prec, conv == b'G', alt),
    };
    if alt && prec == 0 && matches!(conv, b'e' | b'E' | b'f') && !s.contains('.') {
        match s.find(['e', 'E']) {
            Some(i) => format!("{}.{}", &s[..i], &s[i..]),
            None => format!("{s}."),
        }
    } else {
        s
    }
}

pub fn fmt_g(n: f64, prec: usize, upper: bool, alt: bool) -> String {
    if let Some(s) = special(n) {
        return s;
    }
    let p = if prec == 0 { 1 } else { prec };
    if n == 0.0 {
        let z = if n.is_sign_negative() { "-0" } else { "0" };
        return if alt { format!("{z}.{}", "0".repeat(p - 1)) } else { z.to_string() };
    }
    let es = format!("{:.*e}", p - 1, n);
    let x: i32 = es.split_once('e').map(|(_, e)| e.parse().unwrap_or(0)).unwrap_or(0);
    let mut s =
        if x < -4 || x >= p as i32 { fmt_e(n, p - 1, upper) } else { fmt_f(n, (p as i32 - 1 - x).max(0) as usize) };
    if !alt {
        let (mant, exp) = match s.find(['e', 'E']) {
            Some(i) => (s[..i].to_string(), s[i..].to_string()),
            None => (s.clone(), String::new()),
        };
        let mant = if mant.contains('.') { mant.trim_end_matches('0').trim_end_matches('.').to_string() } else { mant };
        s = mant + &exp;
    }
    s
}

pub fn tostring(n: f64) -> String {
    fmt_g(n, 14, false, false)
}

pub fn fmod(a: f64, b: f64) -> f64 {
    a % b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string() {
        assert_eq!(tostring(1.0), "1");
        assert_eq!(tostring(-0.0), "-0");
        assert_eq!(tostring(0.1), "0.1");
        assert_eq!(tostring(1.0 / 3.0), "0.33333333333333");
        assert_eq!(tostring(1e15), "1e+015");
        assert_eq!(tostring(123456789012345.0), "1.2345678901234e+014");
        assert_eq!(tostring(1e14), "1e+014");
        assert_eq!(tostring(99999999999999.0), "99999999999999");
        assert_eq!(tostring(0.0001), "0.0001");
        assert_eq!(tostring(0.00001), "1e-005");
        assert_eq!(tostring(f64::INFINITY), "1.#INF");
        assert_eq!(tostring(f64::NEG_INFINITY), "-1.#INF");
        assert_eq!(fmt_g(2.5, 6, false, true), "2.50000");
    }

    #[test]
    fn string_to_number() {
        assert_eq!(str2d(b" 10 "), Some(10.0));
        assert_eq!(str2d(b"0x10"), Some(16.0));
        assert_eq!(str2d(b"1e"), None);
        assert_eq!(str2d(b".5"), Some(0.5));
        assert_eq!(str2d(b"5."), Some(5.0));
        assert_eq!(str2d(b"-0x10"), Some(4294967280.0));
        assert_eq!(str2d(b""), None);
        assert_eq!(str2d(b"0x"), None);
        assert_eq!(str2d(b"inf"), None);
        assert_eq!(to_int(-2.7), -2);
        assert_eq!(to_int(1e12), i32::MAX);
        assert_eq!(to_int(f64::NAN), i32::MIN);
    }
}
