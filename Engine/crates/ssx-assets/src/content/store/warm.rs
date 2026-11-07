use std::collections::BTreeSet;

use ssx_content::{Kind, Source};

use super::GameContent;

fn kinds_of(sources: &[&dyn Source], wanted: &[Kind]) -> BTreeSet<Kind> {
    let mut kinds = BTreeSet::new();
    for s in sources {
        let patches = s.patches();
        for &k in wanted {
            if !kinds.contains(&k) && (patches.iter().any(|id| id.kind() == k) || !s.list(k).is_empty()) {
                kinds.insert(k);
            }
        }
    }
    kinds
}

const WANTED: [Kind; 20] = [
    Kind::Particles,
    Kind::Sound,
    Kind::Music,
    Kind::Character,
    Kind::Gear,
    Kind::Board,
    Kind::Ui,
    Kind::Font,
    Kind::String,
    Kind::Movie,
    Kind::Tuning,
    Kind::Scoring,
    Kind::Event,
    Kind::Drop,
    Kind::Anim,
    Kind::Object,
    Kind::Material,
    Kind::Lighting,
    Kind::Postfx,
    Kind::Map,
];

impl GameContent {
    pub(in crate::content) fn warm_for(&self, sources: &[&dyn Source]) {
        if sources.is_empty() {
            return;
        }
        let kinds = kinds_of(sources, &WANTED);
        let any = |ks: &[Kind]| ks.iter().any(|k| kinds.contains(k));
        if any(&[Kind::Particles]) {
            let _ = self.particles_index();
        }
        if any(&[Kind::Sound]) {
            let _ = self.disc_sounds();
        }
        if any(&[Kind::Music]) {
            let _ = self.disc_music();
        }
        if any(&crate::content::characters::RIDER_KINDS) {
            let _ = self.disc_riders();
        }
        if any(&[Kind::Ui]) {
            let _ = self.disc_ui();
        }
        if any(&[Kind::Font]) {
            let _ = self.disc_fonts();
        }
        if any(&[Kind::String]) {
            let _ = (self.disc_strings(), self.port_strings());
        }
        if any(&[Kind::Movie]) {
            let _ = self.disc_movies();
        }
        if any(&crate::content::data::tuning::KINDS) {
            let _ = self.disc_tuning();
        }
        if any(&crate::content::data::events::KINDS) {
            let _ = self.disc_events();
        }
        if any(&[Kind::Anim]) {
            let _ = self.disc_anims();
        }
        if any(&[Kind::Object, Kind::Material, Kind::Lighting, Kind::Postfx, Kind::Map])
            && let Some(r) = &self.records
        {
            let _ = r.names();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_index_kind_is_wanted() {
        let mut kinds: Vec<Kind> = vec![Kind::Particles, Kind::Sound, Kind::Music, Kind::Ui, Kind::Font, Kind::String];
        kinds.extend(crate::content::characters::RIDER_KINDS);
        kinds.extend([Kind::Movie, Kind::Anim, Kind::Object, Kind::Material, Kind::Lighting, Kind::Postfx, Kind::Map]);
        kinds.extend(crate::content::data::tuning::KINDS);
        kinds.extend(crate::content::data::events::KINDS);
        for k in kinds {
            assert!(WANTED.contains(&k), "{k:?}");
        }
    }
}
