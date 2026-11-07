use std::sync::{Arc, Mutex};

use ssx_assets::content::GameContent;
use ssx_assets::content::server::SessionMount;
use ssx_content::data::SimContentId;
use ssx_content::delivery::{self, LevelId, LevelName, MAX_LEVELS};
use ssx_content::policy::ClientMods;
use ssx_content::{ContentIndex, PackageSource};

pub struct IdsInput {
    pub content: Arc<GameContent>,
    pub index: Result<ContentIndex, String>,
    pub levels: Vec<String>,
    pub mount: Mutex<Option<SessionMount>>,
}

#[derive(Clone, Debug, Default)]
pub struct Ids {
    pub levels: Vec<LevelId>,
    pub content_id: Option<SimContentId>,
    pub problem: Option<String>,
}

pub fn configured(config: &crate::config::Config) -> Vec<String> {
    use ssx_net_proto::instance::Level;
    let mut out: Vec<String> = Vec::new();
    for g in &config.instances {
        let levels = g.rounds.as_ref().map_or_else(|| vec![g.spec.level.clone()], |(_, r)| r.clone());
        for level in levels {
            let name = match &level {
                Level::Drop(d) => LevelName::drop_name(d),
                Level::Map { id, event_type, gamemode } => {
                    let Some(mode) = gamemode.as_deref().or(ssx_net_proto::instance::built_in_mode(*event_type)) else {
                        continue;
                    };
                    LevelName::map_name(id, mode)
                }
            };
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

pub fn compute(input: &IdsInput, sources: Vec<PackageSource>) -> Ids {
    let mounted = input.content.mount_session(ClientMods::Presentation, sources);
    *input.mount.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(mounted);
    let index = match &input.index {
        Ok(i) => i,
        Err(e) => return Ids { problem: Some(format!("the rules data's index: {e}")), ..Ids::default() },
    };
    if input.levels.len() > MAX_LEVELS {
        let problem = format!("{} levels: a server states the ids of at most {MAX_LEVELS} levels", input.levels.len());
        return Ids { problem: Some(problem), ..Ids::default() };
    }
    let mut levels = Vec::new();
    let all = input.levels.iter().map(|level| input.content.level_id(index, level));
    for level in all {
        match level {
            Ok(l) => levels.push(l),
            Err(e) => return Ids { problem: Some(e), ..Ids::default() },
        }
    }
    let pairs: Vec<(String, SimContentId)> = levels.iter().map(|l| (l.level.clone(), l.id)).collect();
    match delivery::content_id(&pairs) {
        Some(id) => Ids { levels, content_id: Some(id), problem: None },
        None => Ids { problem: Some("a level is named twice".into()), ..Ids::default() },
    }
}
