pub mod event_fields;
pub mod events;
pub mod export;
pub mod kinds;
pub mod simid;
pub mod tuning;

use std::sync::atomic::{AtomicU8, Ordering};

use ssx_content::policy::ClientMods;
use ssx_content::{ContentId, Kind, Layer};

use super::store::GameContent;

pub const DATA_KINDS: [Kind; 4] = [Kind::Tuning, Kind::Scoring, Kind::Event, Kind::Drop];

pub const LIGHTING_SIM_FIELDS: [&str; 4] =
    ["dirLight_Direction", "dirLight_Offset", "isNightCondition", "headLampScaleFactor"];

pub const SIM_KINDS: [Kind; 12] = [
    Kind::Tuning,
    Kind::Scoring,
    Kind::Event,
    Kind::Drop,
    Kind::Lighting,
    Kind::Object,
    Kind::Model,
    Kind::Character,
    Kind::Gear,
    Kind::Board,
    Kind::Anim,
    Kind::Trick,
];

#[derive(Default)]
pub struct DataCache {
    pub(in crate::content) tuning: tuning::TuningCache,
    pub(in crate::content) events: events::EventCache,
    policy: AtomicU8,
    pub(in crate::content) session: AtomicU8,
}

fn policy_value(p: ClientMods) -> u8 {
    match p {
        ClientMods::Any => 0,
        ClientMods::Presentation => 1,
        ClientMods::None => 2,
    }
}

impl GameContent {
    pub fn session_policy(&self) -> Option<ClientMods> {
        match self.data.session.load(Ordering::Acquire) {
            0 => None,
            1 => Some(ClientMods::Any),
            2 => Some(ClientMods::Presentation),
            _ => Some(ClientMods::None),
        }
    }

    pub(in crate::content) fn set_session_policy(&self, p: Option<ClientMods>) {
        self.data.session.store(p.map_or(0, |p| 1 + policy_value(p)), Ordering::Release);
    }

    pub fn sim_policy(&self) -> ClientMods {
        match self.data.policy.load(Ordering::Acquire) {
            0 => ClientMods::Any,
            1 => ClientMods::Presentation,
            _ => ClientMods::None,
        }
    }

    pub fn set_sim_policy(&self, p: ClientMods) {
        let v = policy_value(p);
        if self.data.policy.swap(v, Ordering::AcqRel) != v {
            self.clear_track_decisions();
        }
    }

    pub fn sim_changed(&self) -> bool {
        SIM_KINDS.iter().any(|&k| self.packages_provide(k))
    }

    pub(in crate::content) fn changed_ids(&self, kinds: &[Kind]) -> Vec<ContentId> {
        self.with_registry(|r| {
            let mut ids = std::collections::BTreeSet::new();
            for &k in kinds {
                ids.extend(r.list_above(Layer::Base, k));
            }
            ids.extend(r.patched_above(Layer::Base).into_iter().filter(|id| kinds.contains(&id.kind())));
            ids.into_iter().collect()
        })
    }

    pub(in crate::content) fn sim_record(
        &self,
        id: &ContentId,
        disc: toml::Table,
    ) -> Result<(toml::Table, String, Layer), String> {
        match self.sim_policy() {
            ClientMods::Any => self.routed_record(id, disc),
            ClientMods::Presentation | ClientMods::None => self.routed_record_below(id, disc, Layer::ClientMods),
        }
    }

    pub(in crate::content) fn record_for(
        &self,
        id: &ContentId,
        disc: toml::Table,
        class: ssx_content::data::Class,
    ) -> Result<(toml::Table, String, Layer), String> {
        match (class, self.sim_policy()) {
            (ssx_content::data::Class::Presentation, ClientMods::Any | ClientMods::Presentation) => {
                self.routed_record(id, disc)
            }
            _ => self.sim_record(id, disc),
        }
    }

    pub(in crate::content) fn routed_record_below(
        &self,
        id: &ContentId,
        disc: toml::Table,
        skip: Layer,
    ) -> Result<(toml::Table, String, Layer), String> {
        self.with_registry(|r| {
            let winner =
                r.providers(id).find(|(m, _)| m.layer != skip).map(|(m, s)| (m.layer, m.name.to_string(), s.load(id)));
            let patches = r.patches_in(id, |l| l != skip).map_err(|e| e.to_string())?;
            let (mut t, source, layer) = match winner {
                None => return Err(format!("{id}: no source provides it")),
                Some((Layer::Base, name, _)) => (disc, name, Layer::Base),
                Some((layer, name, loaded)) => {
                    let asset = loaded.map_err(|e| e.to_string())?;
                    let t =
                        asset.record().cloned().ok_or_else(|| format!("{id}: the winner's asset is not a record"))?;
                    (t, name, layer)
                }
            };
            for (_, _, p) in &patches {
                ssx_content::patch::merge(&mut t, p);
            }
            let by: Vec<&str> = patches.iter().map(|(n, _, _)| n.as_str()).filter(|n| *n != source).collect();
            let described = if by.is_empty() { source } else { format!("{source}, patched by {}", by.join(", ")) };
            Ok((t, described, layer))
        })
    }
}

impl GameContent {
    pub(in crate::content) fn dev_data_problem(
        &self,
        source: &ssx_content::PackageSource,
        id: &ContentId,
    ) -> Option<String> {
        use ssx_content::Source as _;
        use ssx_content::data::{Mode, check_table_with};
        let tuning = self.disc_tuning();
        let schema = match id.kind() {
            Kind::Tuning | Kind::Scoring => tuning.as_ref()?.schema_of(id),
            _ => self.disc_events()?.schema_of(id),
        };
        let Some(schema) = schema else { return Some(format!("{id}: the disc has no such record")) };
        let spans = ssx_content::data::spans::Spans::default();
        let guid_class = |g: &[u8; 16]| tuning.as_ref().and_then(|t| t.guid_class(g));
        let check = |t: &toml::Table, mode: Mode| check_table_with(&schema, t, &spans, mode, Some(&guid_class));
        let mut problems = Vec::new();
        if source.manifest.entry(id).is_some() {
            match source.load(id) {
                Ok(a) => match a.record() {
                    Some(t) => problems.extend(check(t, Mode::Whole)),
                    None => return Some(format!("{id}: not a record")),
                },
                Err(e) => return Some(e.to_string()),
            }
        }
        match source.patch(id) {
            Ok(Some(p)) => problems.extend(check(&p, Mode::Patch)),
            Ok(None) => {}
            Err(e) => return Some(e.to_string()),
        }
        let shown: Vec<String> = problems.iter().take(8).map(|p| format!("{}: {}", p.path, p.message)).collect();
        if !shown.is_empty() {
            return Some(format!("{id}: {}", shown.join("; ")));
        }
        tuning.as_ref().and_then(|t| dev_linked(t, source, id))
    }
}

fn dev_linked(disc: &tuning::DiscTuning, source: &ssx_content::PackageSource, id: &ContentId) -> Option<String> {
    use ssx_content::Source as _;
    let ours = |x: &ContentId| -> Option<toml::Table> {
        let whole = match source.manifest.entry(x) {
            Some(_) => source.load(x).ok().and_then(|a| a.record().cloned()),
            None => disc.record(x).ok(),
        }?;
        match source.patch(x) {
            Ok(Some(p)) => Some(ssx_content::patch::apply(&whole, [&p])),
            _ => Some(whole),
        }
    };
    for &(a, b, why) in ssx_content::data::ant_schema::LINKED_FLOATS {
        let (Ok(a), Ok(b)) = (a.parse::<ContentId>(), b.parse::<ContentId>()) else { continue };
        if *id != a && *id != b {
            continue;
        }
        let (Some(ta), Some(tb)) = (ours(&a), ours(&b)) else { continue };
        if let Some(d) = ssx_content::data::ant_schema::floats_differ(&ta, &tb) {
            return Some(format!("{a} and {b} differ ({d}): {why}"));
        }
    }
    None
}

#[cfg(test)]
mod tests;
