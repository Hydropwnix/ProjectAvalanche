use std::path::Path;

use anyhow::Result;
use serde_json::{Map, Value as Json};

use super::{DisplaySettings, MEMBER};
use crate::settings_file::{self, UpdateError};

pub fn read_member(path: &Path) -> (DisplaySettings, Vec<String>) {
    match settings_file::read_object(path) {
        Ok(m) => DisplaySettings::from_json(m.get(MEMBER)),
        Err(e) => (DisplaySettings::default(), vec![format!("{e:#}: the display defaults (a window) used")]),
    }
}

pub fn write_member(path: &Path, base: &DisplaySettings, s: &DisplaySettings) -> Result<()> {
    let object = |s: &DisplaySettings| match s.to_json() {
        Json::Object(m) => m,
        _ => Map::new(),
    };
    let (base, ours) = (object(base), object(s));
    let written = settings_file::update_object(path, |m| {
        match m.get_mut(MEMBER) {
            Some(Json::Object(theirs)) => settings_file::merge_changes(theirs, &base, &ours, 1),
            _ => {
                m.insert(MEMBER.into(), Json::Object(ours));
            }
        }
        Ok(())
    });
    written.map(drop).map_err(|e| match e {
        UpdateError::Unreadable(why) => anyhow::anyhow!("the display settings were not kept: {why}"),
        UpdateError::Failed(e) => e.context("the display settings were not kept"),
    })
}
