use super::{Align, CharFormat, ParaFormat, Paragraph, Run};

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || lower.as_bytes()[at - 1].is_ascii_whitespace();
        let rest = lower[at + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let vstart = tag.len() - rest.len() + 1;
            let v = tag[vstart..].trim_start();
            let q = v.chars().next()?;
            return if q == '"' || q == '\'' {
                v[1..].split(q).next().map(str::to_string)
            } else {
                v.split_whitespace().next().map(str::to_string)
            };
        }
        from = at + name.len();
    }
    None
}

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => ent
                .strip_prefix("#x")
                .or_else(|| ent.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn parse_hex(v: &str) -> Option<u32> {
    let h = v.strip_prefix('#')?;
    if h.is_empty() {
        return None;
    }
    h.chars().try_fold(0u32, |acc, c| Some(acc.wrapping_shl(4) | c.to_digit(16)?))
}

fn parse_color(v: &str, current: [f32; 4]) -> Option<[f32; 4]> {
    let c = parse_hex(v)?;
    let [r, g, b] = [(c >> 16) & 255, (c >> 8) & 255, c & 255].map(|v| v as f32 / 255.0);
    Some([r, g, b, current[3]])
}

pub fn parse_html(src: &str, ch: &CharFormat, para: &ParaFormat) -> Vec<Paragraph> {
    let mut paras: Vec<Paragraph> = Vec::new();
    let mut open = false;
    let mut fmt: Vec<CharFormat> = vec![ch.clone()];
    let mut pfmt = *para;
    let mut rest = src;
    let start = |paras: &mut Vec<Paragraph>, f: ParaFormat| paras.push(Paragraph { format: f, runs: Vec::new() });
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('<') {
            let end = stripped.find('>').unwrap_or(stripped.len());
            let tag = &stripped[..end];
            rest = stripped.get(end + 1..).unwrap_or("");
            let name = tag.split_whitespace().next().unwrap_or("").trim_end_matches('/').to_ascii_lowercase();
            let top = fmt.last().cloned().unwrap_or_else(|| ch.clone());
            match name.as_str() {
                "p" => {
                    pfmt.align = attr(tag, "align").and_then(|a| Align::from_html(&a)).unwrap_or(para.align);
                    start(&mut paras, pfmt);
                    open = true;
                }
                "/p" => open = false,
                "br" => {
                    if !open {
                        start(&mut paras, pfmt);
                    }
                    start(&mut paras, pfmt);
                    open = true;
                }
                "font" => {
                    let mut f = top;
                    if let Some(face) = attr(tag, "face") {
                        f.face = Some(face);
                    }
                    if let Some(size) = attr(tag, "size").and_then(|s| s.trim().parse::<f32>().ok()) {
                        f.size = size * 20.0;
                    }
                    if let Some(c) = attr(tag, "color").and_then(|c| parse_color(&c, f.color)) {
                        f.color = c;
                    }
                    if let Some(a) = attr(tag, "alpha").and_then(|a| parse_hex(&a)) {
                        f.color[3] = (a & 255) as f32 / 255.0;
                    }
                    if let Some(ls) = attr(tag, "letterspacing").and_then(|s| s.trim().parse::<f32>().ok()) {
                        f.letter_spacing = ls * 20.0;
                    }
                    if let Some(k) = attr(tag, "kerning") {
                        f.kerning = k.trim() != "0";
                    }
                    fmt.push(f);
                }
                "b" | "i" | "u" => {
                    let mut f = top;
                    match name.as_str() {
                        "b" => f.bold = true,
                        "i" => f.italic = true,
                        _ => f.underline = true,
                    }
                    fmt.push(f);
                }
                "/font" | "/b" | "/i" | "/u" => {
                    if fmt.len() > 1 {
                        fmt.pop();
                    }
                }
                "textformat" => {
                    let px = |n: &str| attr(tag, n).and_then(|s| s.trim().parse::<f32>().ok()).map(|v| v * 20.0);
                    pfmt.leading = px("leading").unwrap_or(pfmt.leading);
                    pfmt.left_margin = px("leftmargin").unwrap_or(pfmt.left_margin);
                    pfmt.right_margin = px("rightmargin").unwrap_or(pfmt.right_margin);
                    pfmt.indent = px("indent").unwrap_or(pfmt.indent);
                }
                "/textformat" => pfmt = ParaFormat { align: pfmt.align, ..*para },
                _ => {}
            }
            continue;
        }
        let end = rest.find('<').unwrap_or(rest.len());
        let text = decode_entities(&rest[..end]);
        rest = &rest[end..];
        if !open {
            start(&mut paras, pfmt);
            open = true;
        }
        for (i, piece) in text.replace("\r\n", "\n").split(['\r', '\n']).enumerate() {
            if i > 0 {
                start(&mut paras, pfmt);
            }
            if !piece.is_empty() {
                let f = fmt.last().cloned().unwrap_or_else(|| ch.clone());
                paras.last_mut().expect("a paragraph").runs.push(Run { text: piece.to_string(), format: f });
            }
        }
    }
    if paras.is_empty() {
        paras.push(Paragraph { format: *para, runs: Vec::new() });
    }
    paras
}
