use std::sync::Arc;

use super::{PackageSource, is_toml};
use crate::id::{ContentId, Kind};
use crate::manifest::Entry;
use crate::model::{self, Model, ModelCost};
use crate::registry::{Asset, LoadError};
use crate::rider::RiderEntry;

pub const ROLES: [&str; 2] = ["model", "data"];

pub fn is_rider_kind(kind: Kind) -> bool {
    matches!(kind, Kind::Character | Kind::Gear | Kind::Board)
}

impl PackageSource {
    fn rider_files(&self, id: &ContentId) -> Result<(Option<String>, Option<String>), LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let model_ext = |f: &str| model::FORMATS.iter().any(|e| f.to_ascii_lowercase().ends_with(&format!(".{e}")));
        match self.manifest.entry(id).ok_or_else(|| LoadError::Missing(id.clone()))? {
            Entry::File(f) if is_toml(f) => Ok((None, Some(f.clone()))),
            Entry::File(f) if model_ext(f) => Ok((Some(f.clone()), None)),
            Entry::File(f) => {
                Err(failed(format!("{f}: a {} is a .glb / .gltf, a .toml, or both by role", id.kind().as_str())))
            }
            Entry::Files(files) => {
                if let Some(role) = files.keys().find(|r| !ROLES.contains(&r.as_str())) {
                    return Err(failed(format!(
                        "the role {}: a {}'s roles are model and data",
                        ssx_core::text::quoted(role),
                        id.kind().as_str()
                    )));
                }
                let model = files.get("model").cloned();
                let data = files.get("data").cloned();
                if model.as_deref().is_some_and(|f| !model_ext(f)) || data.as_deref().is_some_and(|f| !is_toml(f)) {
                    return Err(failed("model is a .glb / .gltf, data a .toml".into()));
                }
                Ok((model, data))
            }
        }
    }

    pub fn rider_entry(&self, id: &ContentId) -> Result<RiderEntry, LoadError> {
        let (model, data) = self.rider_files(id)?;
        if let Some(f) = &model {
            self.tree.file_size(f).map_err(|e| LoadError::Failed { id: id.clone(), message: e.to_string() })?;
        }
        let data = match data {
            Some(f) => Some((f.clone(), self.read_toml(id, &f)?)),
            None => None,
        };
        Ok(RiderEntry { data, model })
    }

    pub fn rider_model_cost(&self, id: &ContentId) -> Result<Option<(String, ModelCost)>, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Some(rel) = self.rider_files(id)?.0 else { return Ok(None) };
        let mut costs = self.skinned_costs.lock().map_err(|_| failed("a model reader panicked".into()))?;
        let cost = costs.entry(rel.clone()).or_insert_with(|| {
            model::probe_file(&rel, &self.tree, &model::Limits::skinned()).map_err(|e| e.to_string())
        });
        cost.clone().map(|c| Some((rel, c))).map_err(failed)
    }

    pub fn rider_model(&self, id: &ContentId) -> Result<Option<Arc<Model>>, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        let Some(rel) = self.rider_files(id)?.0 else { return Ok(None) };
        let cell = {
            let mut models = self.skinned.lock().map_err(|_| failed("a model reader panicked".into()))?;
            models.entry(rel.clone()).or_default().clone()
        };
        let decoded = cell.get_or_init(|| {
            let limits = model::Limits::skinned();
            let bytes = self.tree.read(&rel, limits.max_file_bytes).map_err(|e| e.to_string())?;
            let m = model::decode_file(&rel, &bytes, &self.tree, &limits).map_err(|e| e.to_string())?;
            Ok(Arc::new(m))
        });
        decoded.clone().map(Some).map_err(failed)
    }

    pub(super) fn load_rider(&self, id: &ContentId) -> Result<Asset, LoadError> {
        Ok(Asset::Value(Arc::new(self.rider_entry(id)?)))
    }
}
