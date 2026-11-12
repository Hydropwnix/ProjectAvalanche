use anyhow::{Context, Result};
use ssx_formats::t3db::LocStrings;

use crate::fs::FileSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    English,
    French,
    German,
    Spanish,
    Italian,
    Japanese,
}

impl Language {
    pub const ALL: [Language; 6] = [
        Language::English,
        Language::French,
        Language::German,
        Language::Spanish,
        Language::Italian,
        Language::Japanese,
    ];

    pub fn file(self) -> &'static str {
        ["eng_us", "fre_fr", "ger_de", "spa_es", "ita_it", "jpn_jp"][self as usize]
    }

    pub fn parse(s: &str) -> Option<Language> {
        let s = s.to_ascii_lowercase();
        Language::ALL.into_iter().find(|l| l.file() == s || l.file()[..3] == s || l.file()[4..] == s)
    }

    pub fn font_region(self) -> &'static str {
        if self == Language::Japanese { "jp" } else { "na" }
    }

    fn settings(self) -> [u32; 14] {
        const T: [[u32; 14]; 6] = [
            [0x0, 0x4, 0xA, 0x16, 0x1D, 0x1D, 0x25, 0x25, 0x26, 0x28, 0x2E, 0x32, 0x32, 0x32],
            [0x2, 0x6, 0xA, 0x15, 0x1D, 0x1D, 0x25, 0x25, 0x27, 0x28, 0x2E, 0x33, 0x33, 0x33],
            [0x1, 0x5, 0xE, 0x19, 0x20, 0x20, 0x25, 0x25, 0x27, 0x2B, 0x2E, 0x33, 0x33, 0x33],
            [0x2, 0x6, 0xB, 0x17, 0x20, 0x20, 0x25, 0x25, 0x26, 0x2C, 0x2E, 0x33, 0x33, 0x33],
            [0x1, 0x5, 0xB, 0x15, 0x20, 0x20, 0x25, 0x25, 0x26, 0x2D, 0x2E, 0x31, 0x31, 0x31],
            [0x0, 0x4, 0xB, 0x15, 0x20, 0x20, 0x25, 0x25, 0x26, 0x2D, 0x2E, 0x31, 0x31, 0x31],
        ];
        T[self as usize]
    }
}

pub struct Localization {
    pub language: Language,
    pub strings: LocStrings,
    settings: [u32; 14],
    pub missing: std::collections::BTreeSet<String>,
    package_hashes: std::collections::HashSet<u32>,
    port_hashes: std::collections::HashSet<u32>,
    pub content: Option<std::sync::Arc<ssx_assets::content::GameContent>>,
}

impl Localization {
    pub fn load(files: &dyn FileSource, language: Language) -> Result<Self> {
        let path = format!("data/xenon/loc/{}.db", language.file());
        let data = files.read(&path).with_context(|| format!("{path} not found"))?;
        let strings = LocStrings::parse(&data).with_context(|| format!("parsing {path}"))?;
        Ok(Localization {
            language,
            strings,
            settings: language.settings(),
            missing: Default::default(),
            package_hashes: Default::default(),
            port_hashes: Default::default(),
            content: None,
        })
    }

    pub fn content_lang(&self) -> ssx_content::strings::Lang {
        ssx_content::strings::Lang::from_game_file(self.language.file()).unwrap_or(ssx_content::strings::Lang::En)
    }

    pub fn apply_package_strings(&mut self, strings: &[ssx_assets::content::m6::PackageString]) -> Vec<String> {
        let mut refused = Vec::new();
        for s in strings {
            let hash = ssx_formats::t3db::loc_hash(&s.key);
            match self.strings.by_hash.get_mut(&hash) {
                Some((key, text)) if key.eq_ignore_ascii_case(&s.key) => *text = Some(s.text.clone()),
                Some((key, _)) => {
                    refused.push(format!("{} ({}): its key's hash {hash:#010x} is the key {key}'s", s.key, s.source));
                    continue;
                }
                None => drop(self.strings.by_hash.insert(hash, (s.key.clone(), Some(s.text.clone())))),
            }
            self.package_hashes.insert(hash);
        }
        refused
    }

    #[cfg(test)]
    pub(crate) fn empty(language: Language) -> Self {
        Localization {
            language,
            strings: Default::default(),
            settings: language.settings(),
            missing: Default::default(),
            package_hashes: Default::default(),
            port_hashes: Default::default(),
            content: None,
        }
    }

    pub fn text(&mut self, key: &str) -> String {
        match self.strings.get(key) {
            Some(t) => t.to_string(),
            None => {
                self.missing.insert(key.to_string());
                key.to_string()
            }
        }
    }

    pub fn add_port_string(&mut self, key: &str, text: &str) -> Result<()> {
        let hash = ssx_formats::t3db::loc_hash(key);
        if self.package_hashes.contains(&hash) || self.port_hashes.contains(&hash) {
            return Ok(());
        }
        if let Some((id, _)) = self.strings.by_hash.get(&hash) {
            anyhow::bail!("the port's string {key} has the hash {hash:#010x} of the table's {id}");
        }
        self.strings.by_hash.insert(hash, (key.to_string(), Some(text.to_string())));
        self.port_hashes.insert(hash);
        Ok(())
    }

    pub fn find(&self, key: &str) -> Option<String> {
        self.strings.get(key).map(str::to_string)
    }

    pub fn format(&mut self, key: &str, args: &[&str]) -> String {
        let t = self.text(key);
        substitute(&t, args)
    }

    pub fn setting(&self, i: usize) -> u32 {
        self.settings[i]
    }

    fn metric(&self) -> bool {
        self.setting(6) == 0x25
    }

    pub fn percent(&self, v: i64) -> String {
        if self.setting(8) == 0x27 { format!("{v} %") } else { format!("{v}%") }
    }

    pub fn int(&self, v: i64) -> String {
        let m = v.unsigned_abs();
        let text = if m < 1_000_000_000 {
            group(&m.to_string(), self.thousands())
        } else {
            let n = m as f32 * f32::from_bits(0x3089_705F);
            let suffix = self.find("TXT_CURRENCY_BILLIONS").unwrap_or_else(|| "TXT_CURRENCY_BILLIONS".into());
            format!("{} {suffix}", self.number(f64::from(n), 1))
        };
        if v < 0 { format!("-{text}") } else { text }
    }

    pub fn abbreviated_int(&mut self, v: u32) -> String {
        if v < 1_000_000_000 {
            let sep = match self.setting(0) {
                0 => Some(','),
                1 => Some('.'),
                2 => Some(' '),
                _ => None,
            };
            return group(&v.to_string(), sep);
        }
        let n = v as f32 * f32::from_bits(0x3089_705F);
        let suffix = self.text("TXT_CURRENCY_BILLIONS");
        format!("{} {suffix}", self.number(f64::from(n), 1))
    }

    fn thousands(&self) -> Option<char> {
        match self.setting(1) {
            4 => Some(','),
            5 => Some('.'),
            6 | 9 => Some(' '),
            _ => None,
        }
    }

    fn decimal_mark(&self) -> char {
        match self.setting(1) {
            5..=8 => ',',
            _ => '.',
        }
    }

    pub fn number(&self, v: f64, decimals: usize) -> String {
        let s = format!("{:.*}", decimals, v.abs());
        let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
        let mut out = group(int, self.thousands());
        if decimals > 0 {
            out.push(self.decimal_mark());
            out.push_str(frac);
        }
        if v < 0.0 && out.chars().any(|c| c.is_ascii_digit() && c != '0') {
            out.insert(0, '-');
        }
        out
    }

    pub fn speed(&self, metres_per_second: f32) -> String {
        let k = if self.metric() { 3.6 } else { 2.236_936_3 };
        self.int((metres_per_second * k) as i32 as i64)
    }

    pub fn speed_unit(&mut self) -> String {
        let key = if self.metric() { "TXT_UNIT_KILOMETRES_PER_HOUR_ABBR" } else { "TXT_UNIT_MILES_PER_HOUR_ABBR" };
        self.text(key)
    }

    pub fn distance(&self, metres: f64) -> String {
        let v = if self.metric() { metres } else { (metres * 3.280_839_9) as f32 as f64 };
        self.number(v, 0)
    }

    pub fn distance_long(&mut self, metres: f64) -> String {
        let n = self.distance(metres);
        match self.setting(6) {
            0x25 => self.format("TXT_METER_LONG_LOWER_CAPS", &[&n]),
            0x23 | 0x24 => self.format("TXT_FEET_LONG_LOWER_CAPS", &[&n]),
            _ => n,
        }
    }

    pub fn distance_with_unit(&mut self, metres: f64) -> String {
        let n = self.distance(metres);
        let key = if self.metric() { "TXT_METER_SHORT_LOWER_CAPS" } else { "TXT_FEET_SHORT_LOWER_CAPS" };
        self.format(key, &[&n])
    }

    pub fn distance_unit(&mut self) -> String {
        let key = if self.metric() { "TXT_LE_RESULTS_PHRASE_UNIT_METERS" } else { "TXT_LE_RESULTS_PHRASE_UNIT_FEET" };
        self.text(key)
    }

    pub fn currency(&mut self, v: i64, include_symbol: bool) -> String {
        let n = self.int(v);
        if !include_symbol {
            return n;
        }
        self.with_currency_symbol(&n)
    }

    pub fn currency_tight(&mut self, n: &str) -> String {
        let symbol = self.text("TXT_DOLLAR");
        match self.setting(11) {
            0x33 => format!("{n}{symbol}"),
            0x31 | 0x32 => format!("{symbol}{n}"),
            _ => String::new(),
        }
    }

    pub fn with_currency_symbol(&mut self, n: &str) -> String {
        let symbol = self.text("TXT_DOLLAR");
        match self.setting(11) {
            0x31 => format!("{symbol} {n}"),
            0x32 => format!("{symbol}{n}"),
            0x33 => format!("{n} {symbol}"),
            _ => String::new(),
        }
    }

    pub fn ordinal(&mut self, n: i64) -> String {
        let s = n.to_string();
        let b = s.as_bytes();
        let last = b[b.len() - 1];
        let prev = if b.len() > 1 { b[b.len() - 2] } else { 0 };
        let key = match (prev, last) {
            (b'1', b'1' | b'2' | b'3') => "TXT_LE_RESULTS_ORDINAL_TH",
            (_, b'1') => "TXT_LE_RESULTS_ORDINAL_ST",
            (_, b'2') => "TXT_LE_RESULTS_ORDINAL_ND",
            (_, b'3') => "TXT_LE_RESULTS_ORDINAL_RD",
            _ => "TXT_LE_RESULTS_ORDINAL_TH",
        };
        let suffix = self.text(key);
        format!("{n}{suffix}")
    }

    pub fn race_time(&self, ms: u32) -> String {
        let (m, s, c) = ((ms / 60_000) % 60, (ms / 1000) % 60, (ms / 10) % 100);
        match self.setting(4) {
            0x20 => format!("{m:02}:{s:02},{c:02}"),
            _ => format!("{m:02}:{s:02}.{c:02}"),
        }
    }
}

fn group(digits: &str, sep: Option<char>) -> String {
    let Some(sep) = sep else { return digits.to_string() };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum FormatArg<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
}

pub use ssx_formats::t3db::format::{Placeholder, placeholders};

pub fn format_positional(text: &str, args: &[FormatArg]) -> String {
    let found: Vec<(usize, usize, u8, Option<String>, char)> = placeholders(text)
        .into_iter()
        .filter_map(|p| match p {
            Placeholder::Arg { at, len, number, spec, conversion } => Some((at, len, number, spec, conversion)),
            Placeholder::Text { .. } => None,
        })
        .take(9)
        .collect();
    let mut out = String::with_capacity(text.len() + 16);
    let mut from = 0;
    for &(at, len, number, _, _) in &found {
        let first = found.iter().find(|f| f.2 == number).unwrap_or(&found[0]);
        out.push_str(&text[from..at]);
        from = (at + len).min(text.len());
        match args.get(usize::from(number) - 1) {
            Some(arg) => out.push_str(&printf(first.3.as_deref().unwrap_or(""), first.4, Some(arg))),
            None => out.push_str(&text[at..from]),
        }
    }
    out.push_str(&text[from..]);
    out
}

fn printf(spec: &str, conversion: char, arg: Option<&FormatArg>) -> String {
    let b = spec.as_bytes();
    let mut i = 0;
    let (mut left, mut plus, mut space, mut alt, mut zero) = (false, false, false, false, false);
    while let Some(&c) = b.get(i) {
        match c {
            b'-' => left = true,
            b'+' => plus = true,
            b' ' => space = true,
            b'#' => alt = true,
            b'0' => zero = true,
            _ => break,
        }
        i += 1;
    }
    let digits = |i: &mut usize| {
        let s = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        spec[s..*i].parse::<usize>().ok()
    };
    let width = digits(&mut i).unwrap_or(0);
    let precision = if b.get(i) == Some(&b'.') {
        i += 1;
        Some(digits(&mut i).unwrap_or(0))
    } else {
        None
    };
    let sign = |neg: bool| {
        if neg {
            "-"
        } else if plus {
            "+"
        } else if space {
            " "
        } else {
            ""
        }
    };
    let (prefix, body, numeric) = match (conversion, arg) {
        (_, Some(FormatArg::Str(t))) if conversion != 's' => (String::new(), t.to_string(), false),
        ('s', Some(FormatArg::Str(t))) => {
            (String::new(), precision.map_or(t.to_string(), |p| t.chars().take(p).collect()), false)
        }
        ('s', _) => (String::new(), String::new(), false),
        ('c', Some(FormatArg::Int(v))) => (String::new(), char::from(*v as u8).to_string(), false),
        ('d', Some(FormatArg::Int(v))) => {
            let v = *v as i32;
            let mut d = v.unsigned_abs().to_string();
            if let Some(p) = precision {
                d = if p == 0 && v == 0 { String::new() } else { format!("{d:0>p$}") };
            }
            (sign(v < 0).to_string(), d, precision.is_none())
        }
        ('u', Some(FormatArg::Int(v))) => {
            let mut d = (*v as u32).to_string();
            if let Some(p) = precision {
                d = if p == 0 && *v as u32 == 0 { String::new() } else { format!("{d:0>p$}") };
            }
            (String::new(), d, precision.is_none())
        }
        ('f', Some(FormatArg::Float(v))) => {
            let p = precision.unwrap_or(6);
            let mut d = format!("{:.p$}", v.abs());
            if alt && p == 0 {
                d.push('.');
            }
            (sign(v.is_sign_negative() && *v != 0.0).to_string(), d, true)
        }
        ('f', Some(FormatArg::Int(v))) => (String::new(), v.to_string(), false),
        ('d' | 'u' | 'c', Some(FormatArg::Float(v))) => (String::new(), v.to_string(), false),
        _ => (String::new(), String::new(), false),
    };
    let len = prefix.chars().count() + body.chars().count();
    let pad = width.saturating_sub(len);
    if left {
        format!("{prefix}{body}{}", " ".repeat(pad))
    } else if zero && numeric {
        format!("{prefix}{}{body}", "0".repeat(pad))
    } else {
        format!("{}{prefix}{body}", " ".repeat(pad))
    }
}

pub fn substitute(text: &str, args: &[&str]) -> String {
    let args: Vec<FormatArg> = args.iter().map(|a| FormatArg::Str(a)).collect();
    format_positional(text, &args)
}

#[rustfmt::skip]
const CAPITALS: [(u8, u8, u16, u16); 216] = [
    (8, 7, 0x00E0, 0x0041), (8, 7, 0x00E1, 0x0041), (8, 7, 0x00E2, 0x0041), (8, 7, 0x00E3, 0x0041),
    (8, 7, 0x00E4, 0x0041), (8, 7, 0x00E5, 0x0041), (8, 7, 0x00E7, 0x0043), (8, 7, 0x00E8, 0x0045),
    (8, 7, 0x00E9, 0x0045), (8, 7, 0x00EA, 0x0045), (8, 7, 0x00EB, 0x0045), (8, 7, 0x00EC, 0x0049),
    (8, 7, 0x00ED, 0x0049), (8, 7, 0x00EE, 0x0049), (8, 7, 0x00EF, 0x0049), (8, 7, 0x00F1, 0x004E),
    (8, 7, 0x00F2, 0x004F), (8, 7, 0x00F3, 0x004F), (8, 7, 0x00F4, 0x004F), (8, 7, 0x00F6, 0x004F),
    (8, 7, 0x00F8, 0x004F), (8, 7, 0x00F9, 0x0055), (8, 7, 0x00FA, 0x0055), (8, 7, 0x00FB, 0x0055),
    (8, 7, 0x00FC, 0x0055), (8, 7, 0x00FD, 0x0059), (8, 7, 0x0105, 0x0041), (8, 7, 0x010D, 0x0043),
    (8, 7, 0x0142, 0x004C), (8, 7, 0x0144, 0x004E), (8, 7, 0x0161, 0x0053), (8, 7, 0x017C, 0x005A),
    (8, 7, 0x0119, 0x0045), (8, 7, 0x017A, 0x005A), (8, 7, 0x011B, 0x0045), (8, 7, 0x017E, 0x005A),
    (8, 7, 0x015F, 0x0053), (8, 7, 0x0148, 0x004E), (8, 7, 0x011F, 0x0047), (8, 7, 0x0107, 0x0043),
    (8, 7, 0x0131, 0x0049), (8, 7, 0x015B, 0x0053), (8, 7, 0x0159, 0x0052), (8, 7, 0x016F, 0x0055),
    (8, 7, 0x0165, 0x0054), (8, 7, 0x0151, 0x004F), (8, 7, 0x0117, 0x0045), (8, 7, 0x0163, 0x0054),
    (8, 7, 0x0103, 0x0041), (8, 7, 0x010F, 0x0044), (8, 7, 0x00C0, 0x0041), (8, 7, 0x00C1, 0x0041),
    (8, 7, 0x00C2, 0x0041), (8, 7, 0x00C3, 0x0041), (8, 7, 0x00C4, 0x0041), (8, 7, 0x00C5, 0x0041),
    (8, 7, 0x00C7, 0x0043), (8, 7, 0x00C8, 0x0045), (8, 7, 0x00C9, 0x0045), (8, 7, 0x00CA, 0x0045),
    (8, 7, 0x00CC, 0x0045), (8, 7, 0x00CC, 0x0049), (8, 7, 0x00CD, 0x0049), (8, 7, 0x00CE, 0x0049),
    (8, 7, 0x00CF, 0x0049), (8, 7, 0x00D1, 0x004E), (8, 7, 0x00D2, 0x004F), (8, 7, 0x00D3, 0x004F),
    (8, 7, 0x00D4, 0x004F), (8, 7, 0x00D6, 0x004F), (8, 7, 0x00D8, 0x004F), (8, 7, 0x00D9, 0x0055),
    (8, 7, 0x00DA, 0x0055), (8, 7, 0x00DB, 0x0055), (8, 7, 0x00DC, 0x0055), (8, 7, 0x00DD, 0x0059),
    (8, 7, 0x0104, 0x0041), (8, 7, 0x010C, 0x0043), (8, 7, 0x0141, 0x004C), (8, 7, 0x0143, 0x004E),
    (8, 7, 0x0160, 0x0053), (8, 7, 0x017B, 0x005A), (8, 7, 0x0118, 0x0045), (8, 7, 0x0179, 0x005A),
    (8, 7, 0x011A, 0x0045), (8, 7, 0x017D, 0x005A), (8, 7, 0x015E, 0x0053), (8, 7, 0x0147, 0x004E),
    (8, 7, 0x011E, 0x0047), (8, 7, 0x0106, 0x0043), (8, 7, 0x015A, 0x0053), (8, 7, 0x0158, 0x0052),
    (8, 7, 0x016E, 0x0055), (8, 7, 0x0164, 0x0054), (8, 7, 0x0150, 0x004F), (8, 7, 0x0116, 0x004F),
    (8, 7, 0x0162, 0x0054), (8, 7, 0x0102, 0x0041), (8, 7, 0x010E, 0x0044), (0, 0, 0x00A0, 0x00A0),
    (0, 0, 0x00A1, 0x00A1), (0, 0, 0x00A3, 0x00A3), (0, 0, 0x00AA, 0x00AA), (0, 0, 0x00B0, 0x00B0),
    (0, 0, 0x00BA, 0x00BA), (0, 0, 0x00BF, 0x00BF), (0, 0, 0x00E6, 0x00C6), (0, 0, 0x00E0, 0x00C0),
    (0, 0, 0x00E1, 0x00C1), (0, 0, 0x00E2, 0x00C2), (0, 0, 0x00E3, 0x00C3), (0, 0, 0x00E4, 0x00C4),
    (0, 0, 0x00E5, 0x00C5), (0, 0, 0x00E7, 0x00C7), (0, 0, 0x00E8, 0x00C8), (0, 0, 0x00E9, 0x00C9),
    (0, 0, 0x00EA, 0x00CA), (0, 0, 0x00EB, 0x00CB), (0, 0, 0x00EC, 0x00CC), (0, 0, 0x00ED, 0x00CD),
    (0, 0, 0x00EE, 0x00CE), (0, 0, 0x00EF, 0x00CF), (0, 0, 0x00F1, 0x00D1), (0, 0, 0x00F2, 0x00D2),
    (0, 0, 0x00F3, 0x00D3), (0, 0, 0x00F4, 0x00D4), (0, 0, 0x00F5, 0x00D5), (0, 0, 0x00F6, 0x00D6),
    (0, 0, 0x00F8, 0x00D8), (0, 0, 0x00F9, 0x00D9), (0, 0, 0x00FA, 0x00DA), (0, 0, 0x00FB, 0x00DB),
    (0, 0, 0x00FC, 0x00DC), (0, 0, 0x00FD, 0x00DD), (0, 0, 0x0105, 0x0104), (0, 0, 0x010D, 0x010C),
    (0, 0, 0x0119, 0x0118), (0, 0, 0x013A, 0x0139), (0, 0, 0x0142, 0x0141), (0, 0, 0x0144, 0x0143),
    (0, 0, 0x0153, 0x0152), (0, 0, 0x0161, 0x0160), (0, 0, 0x0171, 0x0170), (0, 0, 0x017C, 0x017B),
    (0, 0, 0x017A, 0x0179), (0, 0, 0x011B, 0x011A), (0, 0, 0x017E, 0x017D), (0, 0, 0x015F, 0x015E),
    (0, 0, 0x0148, 0x0147), (0, 0, 0x011F, 0x011E), (0, 0, 0x0107, 0x0106), (0, 0, 0x0131, 0x0049),
    (0, 0, 0x015B, 0x015A), (0, 0, 0x0159, 0x0158), (0, 0, 0x016F, 0x016E), (0, 0, 0x0165, 0x0164),
    (0, 0, 0x0151, 0x0150), (0, 0, 0x0117, 0x0116), (0, 0, 0x0163, 0x0162), (0, 0, 0x0103, 0x0102),
    (0, 0, 0x010F, 0x010E), (0, 0, 0x02DD, 0x02DC), (0, 0, 0x2013, 0x2013), (0, 0, 0x2019, 0x2019),
    (0, 0, 0x201C, 0x201C), (0, 0, 0x201D, 0x201D), (0, 0, 0x2026, 0x2026), (0, 0, 0x2122, 0x2122),
    (0, 0, 0x0430, 0x0410), (0, 0, 0x0431, 0x0411), (0, 0, 0x0432, 0x0412), (0, 0, 0x0433, 0x0413),
    (0, 0, 0x0434, 0x0414), (0, 0, 0x0435, 0x0415), (0, 0, 0x0436, 0x0416), (0, 0, 0x0437, 0x0417),
    (0, 0, 0x0438, 0x0418), (0, 0, 0x0439, 0x0419), (0, 0, 0x043A, 0x041A), (0, 0, 0x043B, 0x041B),
    (0, 0, 0x043C, 0x041C), (0, 0, 0x043D, 0x041D), (0, 0, 0x043E, 0x041E), (0, 0, 0x043F, 0x041F),
    (0, 0, 0x0440, 0x0420), (0, 0, 0x0441, 0x0421), (0, 0, 0x0442, 0x0422), (0, 0, 0x0443, 0x0423),
    (0, 0, 0x0444, 0x0424), (0, 0, 0x0445, 0x0425), (0, 0, 0x0446, 0x0426), (0, 0, 0x0447, 0x0427),
    (0, 0, 0x0448, 0x0428), (0, 0, 0x0449, 0x0429), (0, 0, 0x044A, 0x042A), (0, 0, 0x044B, 0x042B),
    (0, 0, 0x044C, 0x042C), (0, 0, 0x044D, 0x042D), (0, 0, 0x044E, 0x042E), (0, 0, 0x044F, 0x042F),
    (0, 0, 0x0451, 0x0401), (0, 0, 0x00E1, 0x00C1), (0, 0, 0x010D, 0x010C), (0, 0, 0x010F, 0x010E),
    (0, 0, 0x00E9, 0x00C9), (0, 0, 0x011B, 0x011A), (0, 0, 0x00ED, 0x00CD), (0, 0, 0x0148, 0x0147),
    (0, 0, 0x00F3, 0x00D3), (0, 0, 0x0159, 0x0158), (0, 0, 0x0161, 0x0160), (0, 0, 0x0165, 0x0164),
    (0, 0, 0x00FA, 0x00DA), (0, 0, 0x016F, 0x016E), (0, 0, 0x00FD, 0x00DD), (0, 0, 0x017E, 0x017D),
];

const CAPITALS_TWO: (u8, u8, char, [char; 2]) = (9, 8, '\u{DF}', ['S', 'S']);

fn capital(c: char, sel: (u32, u32), out: &mut String) {
    if c.is_ascii() {
        out.push(c.to_ascii_uppercase());
        return;
    }
    let fits = |s1: u8, s2: u8| (s1 == 0 || u32::from(s1) == sel.0) && (s2 == 0 || u32::from(s2) == sel.1);
    let u = u32::from(c);
    let one = CAPITALS
        .iter()
        .find(|&&(s1, s2, small, cap)| fits(s1, s2) && (u32::from(small) == u || u32::from(cap) == u))
        .and_then(|&(_, _, _, cap)| char::from_u32(u32::from(cap)));
    match one {
        Some(cap) => out.push(cap),
        None => {
            let (s1, s2, small, two) = CAPITALS_TWO;
            if fits(s1, s2) && small == c { out.extend(two) } else { out.push(c) }
        }
    }
}

impl Language {
    pub fn case_selectors(self) -> (u32, u32) {
        match self {
            Language::French => (8, 7),
            Language::German => (9, 8),
            Language::Spanish => (0x13, 0x13),
            Language::Italian => (0xD, 0xC),
            Language::Japanese => (0xE, 0xD),
            Language::English => (0x16, 0x17),
        }
    }
}

impl Localization {
    pub fn text_capitals(&mut self, key: &str) -> String {
        match self.find(key) {
            Some(t) => capitals(&t, self.language.case_selectors()),
            None => self.text(key),
        }
    }
}

pub fn capitals(text: &str, sel: (u32, u32)) -> String {
    let mut out = String::with_capacity(text.len());
    let (mut in_spec, mut no_spec) = (false, false);
    let (mut state, mut count, mut start) = (0u8, 0u32, 0usize);
    let mut prev = '\0';
    for c in text.chars() {
        if !in_spec && !no_spec && c == '%' {
            (in_spec, state, count, start) = (true, 0, 0, out.len());
        }
        no_spec = false;
        if !in_spec {
            capital(c, sel, &mut out);
            prev = c;
            continue;
        }
        let next = match c {
            '#' | '+' | '-' if state == 0 => Some(1),
            '%' if state == 0 && count == 0 => Some(0),
            '%' if state == 0 && count == 1 => Some(7),
            '*' if state <= 1 => Some(2),
            '.' if state <= 2 => Some(4),
            '0'..='9' if state <= 1 => Some(2),
            '0'..='9' if state == 2 || state == 3 => Some(state),
            '0'..='9' if state == 4 => Some(3),
            's' if state == 1 => (prev != ' ').then_some(7),
            'c' | 'd' | 'e' | 'E' | 'f' | 'g' | 'G' | 'i' | 'n' | 'o' | 'p' | 's' | 'u' | 'x' | 'X'
                if matches!(state, 0 | 1 | 2 | 3 | 5) =>
            {
                Some(7)
            }
            'L' | 'h' | 'l' if state <= 3 => Some(5),
            _ => None,
        };
        match next {
            Some(s) => {
                state = s;
                count += 1;
                out.push(c);
                if state == 7 {
                    in_spec = false;
                }
            }
            None => {
                out.truncate(start);
                (in_spec, no_spec) = (false, true);
            }
        }
        prev = c;
    }
    if in_spec {
        out.truncate(start);
    }
    out
}

pub fn breadcrumb_capitals(text: &str) -> String {
    capitals(text, (0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitals_as_the_string_manager_makes_them() {
        let b = breadcrumb_capitals;
        assert_eq!(b("Whitehorn Mountain"), "WHITEHORN MOUNTAIN");
        assert_eq!(b("Mont Blanc · Aiguille du Goûter"), "MONT BLANC · AIGUILLE DU GOÛTER");
        assert_eq!(b("Großglockner"), "GROßGLOCKNER");
        assert_eq!(b("Эльбрус ёлка ı"), "ЭЛЬБРУС ЁЛКА I");
        assert_eq!(b("ΑΒΓ αβγ 富士山"), "ΑΒΓ αβγ 富士山");
        assert_eq!(b("score %1s of %-5.2f"), "SCORE %1s OF %-5.2f");
        assert_eq!(b("100%% sure"), "100%% SURE");
        assert_eq!(b("50% off"), "50OFF");
        assert_eq!(b("a %"), "A ");
        let fr = Language::French.case_selectors();
        assert_eq!(capitals("Aiguille du Goûter Ì", fr), "AIGUILLE DU GOUTER E");
        assert_eq!(capitals("Großglockner", Language::German.case_selectors()), "GROSSGLOCKNER");
        assert_eq!(capitals("Großglockner é", Language::English.case_selectors()), "GROßGLOCKNER É");
    }

    fn loc(language: Language) -> Localization {
        Localization {
            language,
            strings: Default::default(),
            settings: language.settings(),
            missing: Default::default(),
            package_hashes: Default::default(),
            port_hashes: Default::default(),
            content: None,
        }
    }

    #[test]
    fn formats() {
        let mut en = loc(Language::English);
        assert_eq!(en.int(1_234_567), "1,234,567");
        assert_eq!(en.int(-5), "-5");
        assert_eq!(en.number(1234.5, 1), "1,234.5");
        assert_eq!(en.race_time(83_456), "01:23.45");
        assert_eq!(en.speed(10.0), "36");
        assert_eq!(en.text("TXT_NOT_THERE"), "TXT_NOT_THERE");
        let fr = loc(Language::French);
        assert_eq!(fr.number(1234.5, 1), "1 234,5");
        let de = loc(Language::German);
        assert_eq!(de.int(1234), "1.234");
        assert_eq!(de.race_time(83_456), "01:23,45");
        assert_eq!(substitute("%1s behind %2s", &["2.5", "Zoe"]), "2.5 behind Zoe");
        assert_eq!(substitute("%2s vor %1s", &["Zoe", "2,5"]), "2,5 vor Zoe");
        assert_eq!(substitute("100% %1S", &["x"]), "100% x");
        assert_eq!(substitute("%%1s and %%", &["a"]), "%a and %%");
        assert_eq!(substitute("%1s %3s", &["a"]), "a %3s");
        use super::FormatArg::{Float, Int, Str};
        assert_eq!(format_positional("%1[02]d:%2[02]d", &[Int(5), Int(7)]), "05:07");
        assert_eq!(format_positional("%1[.1]f km", &[Float(2.26)]), "2.3 km");
        assert_eq!(format_positional("%1d / %1[05]d", &[Int(42)]), "42 / 42");
        assert_eq!(format_positional("%1u%2c", &[Int(-1), Int(65)]), "4294967295A");
        assert_eq!(format_positional("%1[-4]s|", &[Str("ab")]), "ab  |");
        assert_eq!(format_positional("%d %1x %0s %1[2d", &[Int(1)]), "%d %1x %0s %1[2d");
        assert_eq!(Language::parse("ger_de"), Some(Language::German));
        assert_eq!(Language::parse("jpn"), Some(Language::Japanese));
    }

    #[test]
    fn ordinals_and_currency() {
        let mut en = loc(Language::English);
        assert_eq!(en.ordinal(1), "1TXT_LE_RESULTS_ORDINAL_ST");
        assert_eq!(en.ordinal(12), "12TXT_LE_RESULTS_ORDINAL_TH");
        assert_eq!(en.ordinal(23), "23TXT_LE_RESULTS_ORDINAL_RD");
        assert_eq!(en.ordinal(10), "10TXT_LE_RESULTS_ORDINAL_TH");
        assert_eq!(en.currency(7500, true), "TXT_DOLLAR7,500");
        assert_eq!(en.currency(7500, false), "7,500");
        let mut de = loc(Language::German);
        assert_eq!(de.currency(7500, true), "7.500 TXT_DOLLAR");
    }

    #[test]
    fn abbreviated_ints() {
        let mut en = loc(Language::English);
        assert_eq!(en.abbreviated_int(999_999_999), "999,999,999");
        assert_eq!(en.abbreviated_int(1_000_000_000), "1.0 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.abbreviated_int(u32::MAX), "4.3 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.abbreviated_int(-1i32 as u32), "4.3 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.abbreviated_int(i32::MIN as u32), "2.1 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.abbreviated_int(1_050_000_000), "1.0 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.abbreviated_int(1_150_000_000), "1.1 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.int(999_999_999), "999,999,999");
        assert_eq!(en.int(1_500_000_000), "1.5 TXT_CURRENCY_BILLIONS");
        assert_eq!(en.int(-2_000_000_000), "-2.0 TXT_CURRENCY_BILLIONS");
        let mut de = loc(Language::German);
        assert_eq!(de.abbreviated_int(1234), "1.234");
        assert_eq!(de.abbreviated_int(1_000_000_000), "1,0 TXT_CURRENCY_BILLIONS");
    }
}
