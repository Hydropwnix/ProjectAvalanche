use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::loc::Localization;

pub const DP: &str = "dp_credits_data";
pub fn read(files: &dyn crate::fs::FileSource) -> Option<Vec<u8>> {
    files.read(PATH)
}

pub const PATH: &str = "data/common/config/credits.txt";

pub fn lines(text: &[u8]) -> Vec<&[u8]> {
    let text = text.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(text);
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let end = text[at..].iter().position(|&b| b == b'\r' || b == b'\n').map_or(text.len(), |i| at + i);
        out.push(&text[at..end]);
        at = end;
        if at < text.len() {
            at += 1;
            if at < text.len() && (text[at] ^ text[at - 1]) == 7 {
                at += 1;
            }
        }
    }
    out
}

pub fn line(loc: &mut Localization, raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    let Some(dollar) = s.find('$') else { return s.into_owned() };
    let rest = &s[dollar..];
    let end = dollar + rest.find(['<', ' ']).unwrap_or(rest.len());
    let text = loc.text(&s[dollar + 1..end]);
    format!("{}{}{}", &s[..dollar], text, &s[end..])
}

pub fn build(text: &[u8], cx: &mut Ctx, loc: &mut Localization) -> Value {
    let items: Vec<Value> = lines(text).into_iter().map(|l| Value::str(&line(loc, l))).collect();
    Value::Object(cx.new_array(items))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_text_is_cut_at_line_ends() {
        let text = b"\xEF\xBB\xBF<html>\r\na\r\n\r\nb\nc\rd";
        let l: Vec<&[u8]> = lines(text);
        assert_eq!(l, [&b"<html>"[..], b"a", b"", b"b", b"c", b"d"]);
        assert!(lines(b"").is_empty());
    }

    #[test]
    fn a_name_ends_at_a_tag_or_a_space() {
        let mut loc = Localization::empty(crate::game::loc::Language::English);
        assert_eq!(line(&mut loc, b"<b>$TXT_CREDITS</b>"), format!("<b>{}</b>", loc.text("TXT_CREDITS")));
        assert_eq!(line(&mut loc, b"A - $TXT_X and $TXT_Y"), format!("A - {} and $TXT_Y", loc.text("TXT_X")));
        assert_eq!(line(&mut loc, b"plain"), "plain");
    }
}
