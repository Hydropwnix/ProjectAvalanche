pub use ssx_rules::vault_db::*;

pub trait MaterialTextures {
    fn texture_id(&self, param: &str) -> Option<ssx_content::ContentId>;
}

impl MaterialTextures for Material {
    fn texture_id(&self, param: &str) -> Option<ssx_content::ContentId> {
        crate::content::textures::texture_id(self.textures.get(param)?)
    }
}
