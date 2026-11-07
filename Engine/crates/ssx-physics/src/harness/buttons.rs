use crate::rider::TrickInput;

pub const MAX_PRESSES: usize = 8;

const FIRST_AT: f32 = 4.0;
const TAP: f32 = 4.0;

pub fn parse(v: &str) -> Result<Vec<char>, String> {
    let ok = !v.is_empty() && v.chars().count() <= MAX_PRESSES && v.chars().all(|c| "yxba".contains(c));
    if !ok {
        let shown: String = v.chars().take(16).flat_map(char::escape_default).collect();
        return Err(format!("\"{shown}\": 1 to {MAX_PRESSES} of y, x, b, a (pressed in turn, the last held)"));
    }
    Ok(v.chars().collect())
}

pub fn pressed(seq: &[char], air_time: f32) -> Option<char> {
    let t = air_time - FIRST_AT;
    if t.is_nan() || t < 0.0 {
        return None;
    }
    let k = (t / (2.0 * TAP)) as usize;
    match seq.len().checked_sub(1) {
        Some(last) if k >= last => seq.get(last).copied(),
        Some(_) if t % (2.0 * TAP) < TAP => seq.get(k).copied(),
        _ => None,
    }
}

pub fn press(tricks: &mut TrickInput, c: Option<char>) {
    match c {
        Some('y') => tricks.y = true,
        Some('x') => tricks.x = true,
        Some('b') => tricks.b = true,
        Some('a') => tricks.a = true,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_is_two_taps_then_held() {
        let seq = parse("yy").unwrap();
        let at = |t: f32| pressed(&seq, t);
        assert_eq!(
            (at(0.0), at(3.0), at(4.0), at(7.0), at(8.0), at(11.0)),
            (None, None, Some('y'), Some('y'), None, None)
        );
        assert_eq!((at(12.0), at(40.0), at(f32::NAN)), (Some('y'), Some('y'), None));
    }

    #[test]
    fn a_bad_sequence_is_refused() {
        for v in ["", "yyyyyyyyy", "yz", "y\u{1b}"] {
            assert!(parse(v).is_err(), "{v:?}");
        }
        assert!(parse("y\u{1b}[31m").unwrap_err().contains("\\u{1b}"));
    }
}
