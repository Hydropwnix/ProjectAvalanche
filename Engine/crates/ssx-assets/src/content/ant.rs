use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::Kind;

use super::vaults::rel;
use super::{Catalogue, segment};

pub(crate) const BUNDLE: &str = "cache0/data/common/anim/anim_all.cba";

const SCORING: [&str; 3] = ["ScoringSceneOpAsset", "TrickSceneOpAsset", "ScoringAsset"];

const TUNING: [&str; 20] = [
    "SurfacePropertiesAsset",
    "BoardChatterPropertiesAsset",
    "IceAxePropertiesAsset",
    "AvalancheCarvingPropertiesAsset",
    "GrindPropertiesAsset",
    "CharacterPropertiesAsset",
    "CarvingControllerAsset",
    "AerialControllerAsset",
    "BallisticControllerAsset",
    "BonkControllerAsset",
    "WingsuitControllerAsset",
    "PidAsset",
    "CurveAsset",
    "PhysicsCharacterAsset",
    "MusculatureAsset",
    "MuscleAsset",
    "MuscleGroupAsset",
    "PartProperty",
    "PivotProperty",
    "DriveAsset",
];

pub(crate) fn asset_id(schema: &ssx_formats::ant::Schema, object: &ssx_formats::ant::Object) -> Option<(Kind, String)> {
    let class = object.class_chain(schema).first().copied().unwrap_or("");
    let kind = kind_of(class)?;
    let name = segment(object.name(schema).unwrap_or(""))?;
    Some((kind, format!("{}/{name}", class.to_ascii_lowercase())))
}

pub(crate) fn kind_of(class: &str) -> Option<Kind> {
    if SCORING.contains(&class) {
        Some(Kind::Scoring)
    } else if TUNING.contains(&class) || class.ends_with("SceneOpAsset") {
        Some(Kind::Tuning)
    } else {
        None
    }
}

pub fn add(c: &mut Catalogue, big: &Path, want: &dyn Fn(Kind) -> bool) -> Result<()> {
    let path = big.join(BUNDLE);
    let s =
        ssx_formats::ant::Stream::parse(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("parsing {}", path.display()))?;
    let file = rel(big, &path);
    for a in &s.assets {
        let class = a.object.class_chain(&s.schema).first().copied().unwrap_or("");
        let Some(kind) = kind_of(class).filter(|&k| want(k)) else { continue };
        let name = a.object.name(&s.schema).unwrap_or("");
        let guid = a.object.guid(&s.schema).map(|g| g.to_string()).unwrap_or_default();
        let source = format!("{file}#{class} {name}");
        match segment(name) {
            Some(id) => c.add_record(
                kind,
                &format!("{}/{id}", class.to_ascii_lowercase()),
                format!("{source} {{{guid}}}"),
                source,
            ),
            None => c.note(kind, format!("{class} {name:?} {{{guid}}}: name outside the id alphabet (left out)")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(kind_of("RewindManagementSceneOpAsset"), Some(Kind::Tuning));
        assert_eq!(kind_of("ScoringSceneOpAsset"), Some(Kind::Scoring));
        assert_eq!(kind_of("SurfacePropertiesAsset"), Some(Kind::Tuning));
        assert_eq!(kind_of("SurfacePropertiesValueAsset"), None);
        assert_eq!(kind_of("VbrAnimationAsset"), None);
    }
}
