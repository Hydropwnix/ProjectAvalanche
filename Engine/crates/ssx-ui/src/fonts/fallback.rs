use std::sync::{Arc, OnceLock};

use ssx_content::font::Limits;
use ssx_content::font::lazy::Bytes;
use ssx_content::font::shipped::SHIPPED;

use super::ot::OtFont;

static OPENED: [OnceLock<Option<Arc<OtFont>>>; SHIPPED.len()] = [const { OnceLock::new() }; SHIPPED.len()];

pub fn shipped(i: usize) -> Option<Arc<OtFont>> {
    let s = SHIPPED.get(i)?;
    OPENED[i]
        .get_or_init(|| {
            let bytes = match ssx_content::font::shipped::read(i) {
                Ok(b) => b,
                Err(e) => {
                    ssx_core::eprintln!("fonts: {} unavailable: {e}", s.id);
                    return None;
                }
            };
            match OtFont::open(s.id, Bytes(Arc::from(bytes)), &Limits::shipped()) {
                Ok(f) => Some(Arc::new(f)),
                Err(e) => {
                    ssx_core::eprintln!("fonts: {} unavailable: {e}", s.id);
                    None
                }
            }
        })
        .clone()
}

pub fn all() -> impl Iterator<Item = Arc<OtFont>> {
    (0..SHIPPED.len()).filter_map(shipped)
}

pub fn for_cluster(script: &str, chars: &[char]) -> Option<Arc<OtFont>> {
    let draws = |f: &OtFont| chars.iter().all(|&c| f.covers(c) || ignorable(c));
    let own = SHIPPED.iter().position(|s| s.scripts.contains(&script));
    own.and_then(shipped)
        .filter(|f| draws(f))
        .or_else(|| (0..SHIPPED.len()).filter(|&i| Some(i) != own).filter_map(shipped).find(|f| draws(f)))
}

pub fn draws(c: char) -> bool {
    all().any(|f| f.covers(c))
}

pub fn ignorable(c: char) -> bool {
    icu_properties::CodePointSetData::new::<icu_properties::props::DefaultIgnorableCodePoint>().contains(c)
}
