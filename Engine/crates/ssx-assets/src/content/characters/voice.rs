use ssx_content::rider::CharacterDef;
use ssx_content::{BASE_NAMESPACE, ContentId, Kind};

use crate::content::GameContent;

impl GameContent {
    fn launched_def(&self, name: &str) -> Option<(ContentId, CharacterDef)> {
        if !self.provides_riders() {
            return None;
        }
        let id = match name.parse::<ContentId>() {
            Ok(id) if id.kind() == Kind::Character => id,
            _ => ContentId::new(BASE_NAMESPACE, Kind::Character, name).ok()?,
        };
        let def = self.character_def(&id).ok()?.value;
        Some((id, def))
    }

    pub fn voice_of(&self, name: &str) -> String {
        match self.launched_def(name) {
            Some((id, def)) => match def.voice.as_ref().or(def.stands_in_for.as_ref()) {
                Some(v) => v.path().to_string(),
                None if id.namespace() == BASE_NAMESPACE => id.path().to_string(),
                None => name.to_string(),
            },
            None => name.to_string(),
        }
    }

    pub fn voice_lines(&self, name: &str) -> (Option<ContentId>, Vec<(ContentId, ContentId)>) {
        match self.launched_def(name) {
            Some((id, def)) if !def.voice_lines.is_empty() => (Some(id), def.voice_lines.into_iter().collect()),
            _ => (None, Vec::new()),
        }
    }
}
