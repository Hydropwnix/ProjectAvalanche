use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::Clip;
use crate::game::values::id;

#[derive(Default)]
pub struct RewindIndicator {
    clip: Option<Clip>,
    bound: bool,
}

impl Component for RewindIndicator {
    fn bindings(&self) -> &'static [&'static str] {
        &["db_hud_rewindindicator"]
    }

    fn bind(&mut self, cx: &mut Ctx, env: &mut HudEnv, name: &str, mut clip: Clip) {
        if name == "db_hud_rewindindicator" {
            let text = env.loc.text("TXT_PLACE_GEOTAG");
            clip.invoke(cx, "SetButtonText", &[Value::str(&text)]);
            self.clip = Some(clip);
        }
        self.bound = true;
    }

    fn unbind(&mut self, _name: &str) {
        self.bound = false;
        self.clip = None;
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        let v = &*env.values;
        let changed = v.is_dirty(0x129) || v.is_dirty(id::IS_ENABLED);
        if self.bound
            && changed
            && let Some(c) = self.clip.as_mut()
        {
            c.set_visible(cx, v.bool(id::IS_ENABLED) && v.bool(0x129));
        }
    }
}
