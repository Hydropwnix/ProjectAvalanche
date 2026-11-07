use super::*;

fn required() -> Vec<char> {
    let mut out: Vec<u32> = (0..0x20).chain([0x7F]).chain(0x80..0xA0).collect();
    out.extend(0x202A..=0x202E);
    out.extend(0x2066..=0x2069);
    out.extend([0x200E, 0x200F, 0x061C, 0x200B, 0x200C, 0x200D, 0x2060, 0xFEFF, 0x2028, 0x2029]);
    out.into_iter().filter_map(char::from_u32).collect()
}

fn every_hidden() -> Vec<char> {
    (0..=0x10FFFF).filter_map(char::from_u32).filter(|&c| is_hidden(c)).collect()
}

#[test]
fn the_required_characters_are_escaped() {
    for c in required() {
        assert!(is_hidden(c), "U+{:04X}", u32::from(c));
        let s = format!("a{c}b");
        let out = shown(&s).to_string();
        assert!(!out.contains(c), "U+{:04X} written as {out:?}", u32::from(c));
    }
    assert_eq!(every_hidden().len(), 65 + 170 + 2, "Cc 65, Cf 170 (Unicode 16.0), Zl, Zp");
}

#[test]
fn std_debug_escapes_every_character_of_the_rule() {
    for c in every_hidden() {
        let debug = format!("{:?}", format!("a{c}b"));
        assert!(!debug.contains(c), "std's Debug writes U+{:04X} raw: {debug:?}", u32::from(c));
        assert!(!format!("{c:?}").contains(c), "std's Debug of a char writes U+{:04X} raw", u32::from(c));
        let escape_debug: String = format!("a{c}b").escape_debug().collect();
        assert!(!escape_debug.contains(c), "escape_debug writes U+{:04X} raw", u32::from(c));
    }
}

#[test]
fn the_format_table_is_ordered() {
    for w in FORMAT.windows(2) {
        assert!(w[0].0 <= w[0].1 && w[0].1 + 1 < w[1].0, "{w:x?}");
    }
}

#[test]
fn printable_text_is_unchanged() {
    for s in [
        "plain ASCII: a-z 0-9 ~!@#$%^&*()_+{}|:<>?`-=[];',./",
        "Élise Rösti ñ ç ß",
        "雪山 スキー",
        "ascii \"quotes\" \\",
    ] {
        assert_eq!(shown_max(s, 1000).to_string(), s);
        assert_eq!(escape(s), s);
        assert!(matches!(escape(s), Cow::Borrowed(_)));
    }
}

#[test]
fn escapes_read_as_escape_default_writes_them() {
    let s = "\u{1b}]0;T\u{7}\t\n\r\0\u{9b}\u{202e}\u{200b}\u{e0041}";
    assert_eq!(shown(s).to_string(), "\\u{1b}]0;T\\u{7}\\t\\n\\r\\u{0}\\u{9b}\\u{202e}\\u{200b}\\u{e0041}");
    let std_default: String = s.chars().filter(|c| c.is_control()).flat_map(char::escape_default).collect();
    assert_eq!(std_default, "\\u{1b}\\u{7}\\t\\n\\r\\u{0}\\u{9b}");
}

#[test]
fn escaping_is_idempotent() {
    let hostile = "a\u{1b}[2J\\u{1b} \"q\" \u{202e}x\u{200d}\n";
    let once = escape(hostile).into_owned();
    assert_eq!(escape(&once), once);
    assert_eq!(Text::new(Text::new(hostile)), Text::new(hostile));
    assert_eq!(message(message(hostile).as_str()).as_str(), message(hostile).as_str());
    let quoted_by_std = format!("unknown field {hostile:?}");
    assert_eq!(Text::new(&quoted_by_std), quoted_by_std.as_str());
}

#[test]
fn quoted_is_std_debug_for_printable_and_control_text() {
    for s in ["abc", "a \"b\" \\c", "tab\there", "line\nbreak", "\u{1b}[31m", "\u{202e}rev", "Élise"] {
        assert_eq!(quoted(s).to_string(), format!("{s:?}"), "{s:?}");
    }
    assert_eq!(quoted("\0").to_string(), "\"\\u{0}\"");
}

#[test]
fn a_long_value_is_cut_with_its_length() {
    let key = "k".repeat(1 << 20);
    let s = shown(&key).to_string();
    assert_eq!(s, format!("{}… (1048576 bytes)", "k".repeat(VALUE_CHARS)));
    let q = quoted(&key).to_string();
    assert_eq!(q, format!("\"{}… (1048576 bytes)\"", "k".repeat(VALUE_CHARS)));
    let esc = "\u{1b}".repeat(1 << 20);
    let s = shown_max(&esc, 10).to_string();
    assert_eq!(s, "\\u{1b}… (1048576 bytes)");
    assert_eq!(shown_max("abc", 3).to_string(), "abc");
    assert_eq!(shown_max("abcd", 3).to_string(), "abc… (4 bytes)");
    assert_eq!(shown_max("abc", 0).to_string(), "… (3 bytes)");
}

#[test]
fn text_counts_the_whole_display_and_keeps_its_bound() {
    let long = "x".repeat(3 << 20);
    let t = Text::with_max(format_args!("{long}{long}"), 8);
    assert_eq!(t, format!("{}… ({} bytes)", "x".repeat(8), 6 << 20).as_str());
    let t = Text::new(format_args!("{}", "\u{202e}".repeat(1 << 20)));
    assert!(t.chars().count() <= LINE_CHARS + 24 && !t.chars().any(is_hidden), "{} chars", t.chars().count());
    assert_eq!(message("  unknown field `\u{1b}]0;T\u{7}`\n").as_str(), "unknown field `\\u{1b}]0;T\\u{7}`");
}

#[test]
fn lines_keep_line_breaks_only() {
    assert_eq!(shown_lines("one\ntwo\r\n\u{1b}[2J\u{202e}", 100).to_string(), "one\ntwo\\r\n\\u{1b}[2J\\u{202e}");
}

#[test]
fn the_error_chain_is_escaped_per_cause() {
    #[derive(Debug)]
    struct Outer(std::io::Error);
    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("installing \u{1b}]0;x\u{7}")
        }
    }
    impl std::error::Error for Outer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let e = Outer(std::io::Error::other("bad\u{9b}2J\nname\r"));
    assert_eq!(Chain(&e).to_string(), "installing \\u{1b}]0;x\\u{7}\n\nCaused by:\n    bad\\u{9b}2J\n    name\\r");
    assert_eq!(one_line(&e).as_str(), "installing \\u{1b}]0;x\\u{7}: bad\\u{9b}2J\\nname\\r");
}

#[test]
fn the_error_chain_numbers_several_causes_and_keeps_its_bound() {
    #[derive(Debug)]
    struct Link(String, Option<Box<Link>>);
    impl fmt::Display for Link {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(&self.0)
        }
    }
    impl std::error::Error for Link {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref().map(|l| l as &(dyn std::error::Error + 'static))
        }
    }
    let inner = Link("two\nlines".into(), None);
    let e = Link("top\nnext".into(), Some(Box::new(Link("one".into(), Some(Box::new(inner))))));
    assert_eq!(Chain(&e).to_string(), "top\n    next\n\nCaused by:\n    0: one\n    1: two\n       lines");
    let many = Link("x\n".repeat(10_000), None);
    let out = Chain(&many).to_string();
    assert_eq!(out.lines().count(), CHAIN_LINES + 1, "the lines and the count of the rest");
    assert!(out.ends_with("… (9745 more lines)"), "{}", &out[out.len() - 40..]);
}

#[test]
fn a_list_quotes_each_value_and_keeps_its_bound() {
    assert_eq!(quoted_list(["a", "b\u{1b}[2J"]).as_str(), "[\"a\", \"b\\u{1b}[2J\"]");
    assert_eq!(quoted_list(Vec::<String>::new()).as_str(), "[]");
    let many: Vec<String> = (0..1000).map(|i| format!("{i}\u{202e}{}", "x".repeat(10_000))).collect();
    let shown = quoted_list(&many);
    assert!(shown.ends_with(", … (1000 in all)]"), "{shown}");
    assert_eq!(shown.matches("… (10004 bytes)").count(), LIST_ITEMS, "{shown}");
    assert!(shown.chars().count() < MESSAGE_CHARS * 2 && !shown.chars().any(is_hidden));
}

#[test]
fn a_located_message_keeps_the_line_bound() {
    let place = "p\u{1b}".repeat(5_000);
    let line = located(&place, "m\u{2028}".repeat(5_000));
    assert!(line.starts_with("p\\u{1b}p"), "{line}");
    assert!(line.contains("… (10000 bytes): m\\u{2028}m"), "{line}");
    assert!(line.ends_with("… (21042 bytes)"), "{}", &line[line.len() - 40..]);
    let shown = line.chars().count() - "… (21042 bytes)".chars().count();
    assert!((LINE_CHARS - 8..=LINE_CHARS).contains(&shown), "{shown}");
}
