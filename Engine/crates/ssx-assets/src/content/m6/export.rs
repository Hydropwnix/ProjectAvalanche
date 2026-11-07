use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_content::strings::Lang;
use ssx_content::{ContentId, Kind};

use super::{DiscStrings, DiscUi};

fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn files(big_root: &Path, id: &ContentId) -> Result<Vec<(String, Vec<u8>)>> {
    match id.kind() {
        Kind::Ui => {
            let disc = DiscUi::index(big_root)?;
            let f = disc.files.get(id).with_context(|| format!("{id}: no such screen on the disc"))?;
            let bytes = std::fs::read(f).with_context(|| format!("reading {}", f.display()))?;
            let name = id.path().rsplit('/').next().unwrap_or("movie");
            Ok(vec![(format!("{name}.gfx"), bytes)])
        }
        Kind::String => {
            let disc = DiscStrings::index(big_root)?;
            let prefix = id.path();
            let keys: Vec<(&ContentId, &super::disc::KeyTexts)> =
                disc.texts.iter().filter(|(k, _)| k.path().starts_with(prefix)).collect();
            if keys.is_empty() {
                bail!("{id}: no string's key begins with {prefix:?} (`ssx:string/txt_` is the whole table)");
            }
            let mut out = Vec::new();
            for lang in Lang::ALL {
                let mut text = format!(
                    "# The game's strings in {} from your disc ({} keys), a translation's start for your own work: \
                     keep each string's placeholders\n# (%1s, %2d …) and button tokens (^A^ …). This file itself may \
                     not be shipped (`mod check` refuses its bytes).\n",
                    lang.game_file(),
                    keys.len()
                );
                for (_, (key, by_lang)) in &keys {
                    if let Some(t) = by_lang.get(&lang) {
                        text.push_str(&format!("{key} = {}\n", toml_string(t)));
                    }
                }
                out.push((format!("strings/{}.toml", lang.code()), text.into_bytes()));
            }
            Ok(out)
        }
        Kind::Font => bail!(
            "{id}: fonts are not exported — the font library's fonts are a commercial typeface's outlines (DIN, \
             Linotype), and a font package replaces a font whole: bring a TTF / OTF of your own"
        ),
        k => bail!("{id}: {} is not a screen or a string", k.as_str()),
    }
}
