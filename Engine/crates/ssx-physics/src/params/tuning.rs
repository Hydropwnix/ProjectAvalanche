use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::ant::Stream;

pub use ssx_rules::tuning::*;

pub fn read_check(stream: &Stream) -> std::result::Result<(), String> {
    let bundle = Bundle::from_stream(stream.clone());
    let run = || -> Result<()> {
        let tuning = crate::character::Tuning::load(&bundle)?;
        tuning.resolve(crate::character::DEFAULT_GEAR_MIX)?;
        tuning.resolve_gear(&crate::character::GearStats::uniform(0.0))?;
        tuning.resolve_gear(&crate::character::GearStats::uniform(1.0))?;
        crate::helicopter::HeliUpdateTuning::load(&bundle)?;
        crate::helicopter::HeliMoveTuning::load(&bundle, "heli_move_to_follow")?;
        crate::helicopter::HeliMoveTuning::load(&bundle, "heli_move_to_position")?;
        Ok(())
    };
    run().map_err(|e| format!("{e:#}"))
}

pub fn load(big_root: &Path) -> Result<Bundle> {
    ssx_assets::content::data::tuning::set_bundle_reader(read_check);
    let stream = ssx_assets::content::data::tuning::anim_all(big_root)
        .with_context(|| format!("loading {}", big_root.join(BUNDLE_PATH).display()))?;
    Ok(Bundle::from_stream(stream).with_data_root(big_root))
}
