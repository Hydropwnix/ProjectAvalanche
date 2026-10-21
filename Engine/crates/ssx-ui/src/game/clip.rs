use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::display::InstanceId;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplayInfo {
    pub x: f64,
    pub y: f64,
    pub rotation: f64,
    pub xscale: f64,
    pub yscale: f64,
    pub alpha: f64,
    pub visible: bool,
}

pub const DI_X: u16 = 0x01;
pub const DI_Y: u16 = 0x02;
pub const DI_ROTATION: u16 = 0x04;
pub const DI_XSCALE: u16 = 0x08;
pub const DI_YSCALE: u16 = 0x10;
pub const DI_ALPHA: u16 = 0x20;
pub const DI_VISIBLE: u16 = 0x40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub path: String,
    id: Option<InstanceId>,
}

impl Clip {
    pub fn resolve(cx: &mut Ctx, path: &str) -> Option<Clip> {
        let id = Self::find(cx, path)?;
        Some(Clip { path: path.to_string(), id: Some(id) })
    }

    pub fn bind(cx: &mut Ctx, path: &str) -> Clip {
        Clip { path: path.to_string(), id: Self::find(cx, path) }
    }

    fn find(cx: &mut Ctx, path: &str) -> Option<InstanceId> {
        match cx.instance_at_path(path) {
            Some(id) => Some(id),
            None => {
                let v = super::resolve_path(cx, path)?;
                cx.display_of(&v)
            }
        }
    }

    pub fn instance(&mut self, cx: &mut Ctx) -> Option<InstanceId> {
        if let Some(id) = self.id
            && cx.player.get(id).is_some()
        {
            return Some(id);
        }
        self.id = Self::find(cx, &self.path);
        self.id
    }

    fn object(&mut self, cx: &mut Ctx) -> Option<Value> {
        let id = self.instance(cx)?;
        Some(Value::Object(cx.object_for(id)))
    }

    pub fn get(&mut self, cx: &mut Ctx, name: &str) -> Value {
        let Some(o) = self.object(cx) else { return Value::Undefined };
        cx.get_member(&o, name).unwrap_or_default()
    }

    pub fn get_number(&mut self, cx: &mut Ctx, name: &str) -> f64 {
        let v = self.get(cx, name);
        cx.to_number(&v)
    }

    pub fn set(&mut self, cx: &mut Ctx, name: &str, v: Value) {
        if let Some(o) = self.object(cx) {
            let _ = cx.set_member(&o, name, v);
        }
    }

    pub fn invoke(&mut self, cx: &mut Ctx, method: &str, args: &[Value]) -> Value {
        let Some(o) = self.object(cx) else { return Value::Undefined };
        let Ok((f, _home)) = cx.get_method(&o, method) else { return Value::Undefined };
        if !cx.is_function(&f) {
            return Value::Undefined;
        }
        cx.call_top(&f, o, args)
    }

    pub fn info(&mut self, cx: &mut Ctx) -> DisplayInfo {
        let Some(id) = self.instance(cx) else { return DisplayInfo::default() };
        let mut n = |name: &str| -> f64 {
            let v = cx.display_get(id, name).ok().flatten().unwrap_or_default();
            cx.to_number(&v)
        };
        let (x, y, rotation, xscale, yscale, alpha) =
            (n("_x"), n("_y"), n("_rotation"), n("_xscale"), n("_yscale"), n("_alpha"));
        let visible = cx.player.get(id).is_some_and(|i| i.visible);
        DisplayInfo { x, y, rotation, xscale, yscale, alpha, visible }
    }

    pub fn set_info(&mut self, cx: &mut Ctx, info: &DisplayInfo, set: u16) {
        let Some(id) = self.instance(cx) else { return };
        let fields = [
            (DI_X, "_x", info.x),
            (DI_Y, "_y", info.y),
            (DI_ROTATION, "_rotation", info.rotation),
            (DI_XSCALE, "_xscale", info.xscale),
            (DI_YSCALE, "_yscale", info.yscale),
            (DI_ALPHA, "_alpha", info.alpha),
        ];
        for (bit, name, v) in fields {
            if set & bit != 0 {
                let _ = cx.display_set(id, name, &Value::Number(v));
            }
        }
        if set & DI_VISIBLE != 0 {
            let _ = cx.display_set(id, "_visible", &Value::Bool(info.visible));
        }
    }

    pub fn set_visible(&mut self, cx: &mut Ctx, visible: bool) {
        self.set_info(cx, &DisplayInfo { visible, ..Default::default() }, DI_VISIBLE);
    }

    pub fn visible(&mut self, cx: &mut Ctx) -> bool {
        self.instance(cx).and_then(|id| cx.player.get(id)).is_some_and(|i| i.visible)
    }

    pub fn set_position(&mut self, cx: &mut Ctx, x: f64, y: f64) {
        self.set_info(cx, &DisplayInfo { x, y, ..Default::default() }, DI_X | DI_Y);
    }

    pub fn set_text(&mut self, cx: &mut Ctx, text: &str) {
        self.set(cx, "text", Value::str(text));
    }

    pub fn text(&mut self, cx: &mut Ctx) -> String {
        let v = self.get(cx, "text");
        cx.to_string(&v).map(|s| s.to_string()).unwrap_or_default()
    }

    pub fn goto_label(&mut self, cx: &mut Ctx, label: &str, stop: bool) {
        if let Some(id) = self.instance(cx) {
            let _ = cx.goto_value_with(id, &Value::str(label), Some(!stop), false);
        }
    }

    pub fn goto_frame(&mut self, cx: &mut Ctx, frame: u32, stop: bool) {
        if let Some(id) = self.instance(cx) {
            let _ = cx.goto_value(id, &Value::Number(f64::from(frame)), Some(!stop));
        }
    }

    pub fn current_frame(&mut self, cx: &mut Ctx) -> i32 {
        self.instance(cx).and_then(|id| cx.player.frames(id)).map_or(0, |(c, _)| i32::from(c))
    }
}
