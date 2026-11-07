use super::class::{bracket, of};
use super::*;
use crate::text::isolate;

fn levels(s: &str, base: Option<u8>) -> Resolved {
    let chars: Vec<char> = s.chars().collect();
    let classes: Vec<Class> = chars.iter().map(|&c| of(c)).collect();
    let brackets: Vec<Bracket> = chars.iter().map(|&c| bracket(c)).collect();
    resolve(&classes, &brackets, base)
}

fn visual(s: &str, base: Option<u8>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let classes: Vec<Class> = chars.iter().map(|&c| of(c)).collect();
    let r = levels(s, base);
    let line = line_levels(&classes, &r.levels, r.base, 0..chars.len());
    visual_order(&line).into_iter().map(|i| chars[i]).collect()
}

#[test]
fn left_to_right_text_stays_at_level_zero() {
    let r = levels("abc 123, (x)", None);
    assert_eq!(r.base, 0);
    assert!(r.levels.iter().all(|&l| l == 0));
}

#[test]
fn right_to_left_paragraph_with_numbers() {
    let r = levels("אב 123", None);
    assert_eq!(r.base, 1);
    assert_eq!(r.levels, [1, 1, 1, 2, 2, 2]);
    assert_eq!(visual("אב 123", None), "123 בא");
}

#[test]
fn arabic_letters_turn_european_digits_arabic() {
    let r = levels("a ب1", Some(0));
    assert_eq!(r.levels, [0, 0, 1, 2]);
}

#[test]
fn embedded_right_to_left_word() {
    assert_eq!(visual("car אבג.", None), "car גבא.");
}

#[test]
fn brackets_take_the_direction_they_enclose() {
    let r = levels("א(b)ג", None);
    assert_eq!(r.levels, [1, 1, 2, 1, 1]);
    let r = levels("אa(b)", None);
    assert_eq!(r.levels, [1, 2, 2, 2, 2]);
}

#[test]
fn trailing_whitespace_takes_the_paragraph_level() {
    let chars: Vec<char> = "abc אב  ".chars().collect();
    let classes: Vec<Class> = chars.iter().map(|&c| of(c)).collect();
    let r = levels("abc אב  ", Some(1));
    let line = line_levels(&classes, &r.levels, r.base, 0..chars.len());
    assert_eq!(&line[line.len() - 2..], &[1, 1]);
}

#[test]
fn overflowing_embeddings_stop_at_the_deepest_level() {
    let s: String = std::iter::repeat_n('\u{202B}', 300).chain("a".chars()).collect();
    let r = levels(&s, Some(0));
    assert!(r.levels.iter().all(|&l| l <= MAX_DEPTH + 2));
    let s: String =
        std::iter::repeat_n('\u{2067}', 300).chain("aא".chars()).chain(std::iter::repeat_n('\u{2069}', 300)).collect();
    let r = levels(&s, Some(0));
    assert!(r.levels.iter().all(|&l| l <= MAX_DEPTH + 2));
}

#[test]
fn first_strong_isolate_skips_nested_isolates() {
    let s = "\u{2068}\u{2067}א\u{2069}b\u{2069}";
    let r = levels(s, Some(0));
    assert_eq!(r.levels[4], 2, "b sits in an LTR isolate at level 2: {:?}", r.levels);
    let s = format!("{}א{}", isolate::OPEN, isolate::CLOSE);
    let r = levels(&s, Some(0));
    assert_eq!(r.levels[1], 1);
}

#[test]
fn many_unmatched_first_strong_isolates_resolve_in_linear_time() {
    let s: String = std::iter::repeat_n('\u{2068}', 200_000).collect();
    let t = std::time::Instant::now();
    let _ = levels(&s, None);
    assert!(t.elapsed().as_secs_f64() < 5.0, "{:?}", t.elapsed());
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn pick<T: Copy>(&mut self, from: &[T]) -> T {
        from[(self.next() % from.len() as u64) as usize]
    }
    fn text(&mut self, from: &[char], max: u64) -> String {
        (0..self.next() % (max + 1)).map(|_| self.pick(from)).collect()
    }
}

const NAME: &[char] = &[
    'a', 'Z', 'א', 'ש', 'ب', 'ﻻ', '1', '9', '+', '-', '%', '$', '١', '٢', '۳', ',', '.', ':', '/', '\u{301}',
    '\u{5B4}', ' ', '!', '"', '?', '(', ')', '[', ']', '{', '}', '〈', '〉', '\u{202A}', '\u{202B}', '\u{202D}',
    '\u{202E}', '\u{202C}', '\u{200E}', '\u{200F}', '\u{61C}',
];
const AROUND: &[char] = &[
    'a', 'k', 'א', 'ب', '1', '+', '%', '١', ',', ':', '\u{300}', ' ', '!', '(', ')', '\u{202B}', '\u{202E}',
    '\u{202C}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{200F}', '\t',
];
const STAND_IN: char = '\u{FFFC}';

fn stand_in(name: &str) -> char {
    if name.chars().all(|c| of(c) == Class::WS || of(c).removed()) { ' ' } else { STAND_IN }
}

fn laid_out(chars: &[char], base: Option<u8>) -> (Vec<usize>, Vec<u8>) {
    let classes: Vec<Class> = chars.iter().map(|&c| of(c)).collect();
    let brackets: Vec<Bracket> = chars.iter().map(|&c| bracket(c)).collect();
    let r = resolve(&classes, &brackets, base);
    let line = line_levels(&classes, &r.levels, r.base, 0..chars.len());
    let order = visual_order(&line).into_iter().filter(|&i| !classes[i].removed()).collect();
    (order, line)
}

fn isolates_hold(parts: &[String], names: &[String], base: Option<u8>) {
    let (mut with, mut stood): (Vec<char>, Vec<char>) = (Vec::new(), Vec::new());
    let mut outside: Vec<Option<usize>> = Vec::new();
    let mut name_of: Vec<Option<usize>> = Vec::new();
    let mut stand_at: Vec<usize> = Vec::new();
    for (k, part) in parts.iter().enumerate() {
        for c in part.chars() {
            outside.push(Some(stood.len()));
            name_of.push(None);
            with.push(c);
            stood.push(c);
        }
        if let Some(name) = names.get(k) {
            let wrapped = isolate::wrap(name);
            for c in wrapped.chars() {
                outside.push(None);
                name_of.push((!isolate::is_mark(c)).then_some(k));
                with.push(c);
            }
            stand_at.push(stood.len());
            stood.push(stand_in(name));
        }
    }
    let case = || format!("{base:?} {with:?}");
    let (order_w, levels_w) = laid_out(&with, base);
    let (order_s, levels_s) = laid_out(&stood, base);
    for (i, o) in outside.iter().enumerate() {
        if let Some(k) = o
            && !of(with[i]).removed()
        {
            assert_eq!(levels_w[i], levels_s[*k], "level of outside character {i}: {}", case());
        }
    }
    let all_space: Vec<bool> = names.iter().map(|n| stand_in(n) == ' ').collect();
    let mut collapsed: Vec<usize> = Vec::new();
    let mut seen = vec![false; names.len()];
    for &i in &order_w {
        match (outside[i], name_of[i]) {
            (Some(k), _) => collapsed.push(k),
            (None, Some(n)) if !seen[n] && (of(with[i]) != Class::WS || all_space[n]) => {
                seen[n] = true;
                collapsed.push(stand_at[n]);
            }
            _ => {}
        }
    }
    let expected: Vec<usize> =
        order_s.into_iter().filter(|i| stand_at.iter().position(|s| s == i).is_none_or(|n| seen[n])).collect();
    assert_eq!(collapsed, expected, "order: {}", case());
    for n in 0..names.len() {
        let inked: Vec<usize> = order_w
            .iter()
            .enumerate()
            .filter(|(_, i)| name_of[**i] == Some(n) && of(with[**i]) != Class::WS)
            .map(|(p, _)| p)
            .collect();
        if let (Some(&a), Some(&b)) = (inked.first(), inked.last()) {
            assert!(order_w[a..=b].iter().all(|&i| name_of[i] == Some(n)), "name {n} is split: {}", case());
        }
    }
}

#[test]
fn hostile_names_never_reorder_the_text_round_them() {
    let names = [
        "\u{202E}gnp.exe",
        "\u{202B}abc",
        "\u{202C}\u{202C}x",
        "שלום 123, 456",
        "١٢٣-",
        "12",
        "abc (def",
        ")]〉",
        "\u{200F}",
        "\u{301}\u{301}",
    ];
    let frames = ["Player %s joined", "%s: hello", "אורח %s התחבר", "مرحبا %s 3", "1 %s 2", "%s", "(%s)", "%s.%s"];
    for base in [None, Some(0), Some(1)] {
        for frame in frames {
            let parts: Vec<String> = frame.split("%s").map(str::to_string).collect();
            let holes = parts.len() - 1;
            for (a, b) in names.iter().zip(names.iter().cycle().skip(3)) {
                let picked: Vec<String> = [a, b].iter().take(holes).map(|s| s.to_string()).collect();
                isolates_hold(&parts, &picked, base);
            }
        }
    }
}

#[test]
fn random_names_never_reorder_the_text_round_them() {
    let mut rng = Rng(0x5EED_7E47_0001);
    for _ in 0..20_000 {
        let holes = 1 + (rng.next() % 3) as usize;
        let parts: Vec<String> = (0..=holes).map(|_| rng.text(AROUND, 6)).collect();
        let names: Vec<String> = (0..holes).map(|_| rng.text(NAME, 8)).collect();
        let base = rng.pick(&[None, Some(0), Some(1)]);
        isolates_hold(&parts, &names, base);
    }
}

#[test]
fn a_hundred_marks_and_the_longest_text_stay_linear_and_isolated() {
    let zalgo: String = std::iter::once('a').chain(std::iter::repeat_n('\u{301}', 100)).collect();
    let long_rtl: String = "שלום, 123 ".chars().cycle().take(256).collect();
    let long_mix: String = "abc אבג ١٢٣ (x) ".chars().cycle().take(256).collect();
    for name in [zalgo, long_rtl, long_mix] {
        for frame in ["%s: hello", "مرحبا %s 3"] {
            let parts: Vec<String> = frame.split("%s").map(str::to_string).collect();
            isolates_hold(&parts, std::slice::from_ref(&name), None);
        }
    }
}
