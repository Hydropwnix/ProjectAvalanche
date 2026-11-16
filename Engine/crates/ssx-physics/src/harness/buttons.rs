use crate::rider::TrickInput;
use glam::Vec2;

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
        Some('L') => tricks.right_stick = Vec2::NEG_X,
        Some('R') => tricks.right_stick = Vec2::X,
        Some('U') => tricks.right_stick = Vec2::Y,
        Some('D') => tricks.right_stick = Vec2::NEG_Y,
        _ => {}
    }
}

pub const MAX_SCRIPT: usize = 64;

pub type Script = Vec<(f32, Option<char>)>;

pub fn parse_script(v: &str) -> Result<Script, String> {
    let shown = |x: &str| -> String { x.chars().take(24).flat_map(char::escape_default).collect() };
    let mut out = Script::new();
    for item in v.split(',') {
        let bad = || format!("\"{}\": T:B with T 0 to 3600 s ascending, B one of y x b a rl rr ru rd -", shown(item));
        let (t, b) = item.split_once(':').ok_or_else(bad)?;
        let t: f32 = t.trim().parse().map_err(|_| bad())?;
        let b = match b.trim() {
            "-" => None,
            "y" => Some('y'),
            "x" => Some('x'),
            "b" => Some('b'),
            "a" => Some('a'),
            "rl" => Some('L'),
            "rr" => Some('R'),
            "ru" => Some('U'),
            "rd" => Some('D'),
            _ => return Err(bad()),
        };
        if !(0.0..=3600.0).contains(&t) || out.last().is_some_and(|&(last, _)| t < last) {
            return Err(bad());
        }
        if out.len() == MAX_SCRIPT {
            return Err(format!("more than {MAX_SCRIPT} entries"));
        }
        out.push((t, b));
    }
    Ok(out)
}

pub fn scripted(script: &[(f32, Option<char>)], time: f32) -> Option<char> {
    script.iter().rev().find(|&&(t, _)| t <= time).and_then(|&(_, c)| c)
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
    fn a_timed_script_holds_its_last_entry() {
        let script = parse_script("3.1:x, 3.2:-,3.3:b,4:rl").unwrap();
        let at = |t: f32| scripted(&script, t);
        assert_eq!((at(0.0), at(3.1), at(3.25), at(3.3), at(5.0)), (None, Some('x'), None, Some('b'), Some('L')));
        let mut t = TrickInput::default();
        press(&mut t, at(5.0));
        assert_eq!(t.right_stick, Vec2::NEG_X);
    }

    #[test]
    fn a_bad_script_is_refused() {
        for v in ["", "x", "1:z", "2:x,1:b", "-1:x", "3601:x", "nan:x", "1:x\u{1b}"] {
            assert!(parse_script(v).is_err(), "{v:?}");
        }
        assert!(parse_script("1:\u{1b}[31m").unwrap_err().contains("\\u{1b}"));
        let long = vec!["1:x"; MAX_SCRIPT + 1].join(",");
        assert!(parse_script(&long).is_err());
        assert!(parse_script(&long[4..]).is_ok());
    }

    #[test]
    fn a_bad_sequence_is_refused() {
        for v in ["", "yyyyyyyyy", "yz", "y\u{1b}"] {
            assert!(parse(v).is_err(), "{v:?}");
        }
        assert!(parse("y\u{1b}[31m").unwrap_err().contains("\\u{1b}"));
    }
}
