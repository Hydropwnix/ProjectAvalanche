use anyhow::{Result, bail};

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::saveload;
use crate::game::loc::Localization;
use crate::game::values::UiValues;

pub const DP_POPUP: &str = "std_popup";
pub const VIEW: &str = "popup";
pub const POPUP_BODY: u16 = 0x99;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PopupDef {
    pub name: String,
    pub title: String,
    pub body: String,
    pub use_override: bool,
    pub show_objective: bool,
    pub custom: bool,
    pub default_button: i64,
    pub buttons: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Popups {
    pub defs: Vec<PopupDef>,
    pub stack: Vec<String>,
}

pub const DIR: &str = "data/common/fe/popups/";

impl Popups {
    pub fn load(files: &dyn crate::fs::FileSource) -> Self {
        let mut defs = Vec::new();
        for path in files.list(DIR, "pop") {
            let Some(stem) = path.rsplit('/').next().and_then(|f| f.strip_suffix(".pop")) else { continue };
            let Some(text) = files.read(&path).and_then(|b| String::from_utf8(b).ok()) else { continue };
            if let Some(d) = parse(stem, &text) {
                defs.push(d);
            }
        }
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        Popups { defs, stack: Vec::new() }
    }

    pub fn add(&mut self, def: PopupDef) -> Result<()> {
        if self.def(&def.name).is_some() {
            bail!("the popup {} exists", def.name);
        }
        self.defs.push(def);
        self.defs.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(())
    }

    pub fn def(&self, name: &str) -> Option<&PopupDef> {
        self.defs.iter().find(|d| d.name == name)
    }

    pub fn push(&mut self, name: &str, mode: &str) -> bool {
        let first = self.stack.is_empty();
        if mode == "pop_all" {
            self.stack.clear();
        }
        self.stack.push(name.to_string());
        first
    }

    pub fn pop(&mut self, _name: &str) -> bool {
        self.stack.pop();
        self.stack.is_empty()
    }

    pub fn build(
        &self,
        cx: &mut Ctx,
        values: &UiValues,
        loc: &mut Localization,
        objective: Option<(String, String)>,
    ) -> Value {
        let o = Value::Object(cx.new_object());
        let Some(d) = self.stack.last().and_then(|n| self.def(n)) else { return o };
        if d.custom {
            let _ = cx.set_member(&o, "title", Value::str(&loc.text(&d.title)));
            let _ = cx.set_member(&o, "body", Value::str(values.str(saveload::MESSAGE)));
            let default = values.int(saveload::DEFAULT_BUTTON);
            let _ = cx.set_member(&o, "defaultButton", Value::Number(f64::from(default)));
            let _ = cx.set_member(&o, "type", Value::Number(f64::from(values.int(saveload::TYPE))));
            let _ = cx.set_member(&o, "imageAsset", Value::str(values.str(saveload::IMAGE)));
            let mut buttons = Vec::new();
            for b in values.buttons(saveload::BUTTONS) {
                let v = Value::Object(cx.new_object());
                let _ = cx.set_member(&v, "label", Value::str(&b.label));
                let _ = cx.set_member(&v, "event", Value::str(&b.event));
                let _ = cx.set_member(&v, "eventData", Value::Number(f64::from(b.event_data)));
                buttons.push(v);
            }
            let arr = Value::Object(cx.new_array(buttons));
            let _ = cx.set_member(&o, "buttons", arr);
            return o;
        }
        let (title, body) = if d.show_objective {
            let (t, b) = objective.unwrap_or_default();
            (loc.text(&t), loc.text(&b))
        } else {
            let body = if d.use_override { values.str(POPUP_BODY).to_string() } else { loc.text(&d.body) };
            (loc.text(&d.title), body)
        };
        let _ = cx.set_member(&o, "title", Value::str(&title));
        let _ = cx.set_member(&o, "type", Value::Number(0.0));
        let _ = cx.set_member(&o, "body", Value::str(&body));
        let mut buttons = Vec::new();
        for (label, event) in &d.buttons {
            let b = Value::Object(cx.new_object());
            let text = loc.text(label);
            let _ = cx.set_member(&b, "label", Value::str(&text));
            let _ = cx.set_member(&b, "event", Value::str(event));
            buttons.push(b);
        }
        let arr = Value::Object(cx.new_array(buttons));
        let _ = cx.set_member(&o, "buttons", arr);
        let _ = cx.set_member(&o, "defaultButton", Value::Number(d.default_button as f64));
        o
    }
}

pub fn parse(name: &str, text: &str) -> Option<PopupDef> {
    let cleaned = crate::game::store::drop_trailing_commas(text);
    let v: serde_json::Value = serde_json::from_str(&cleaned).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let buttons = v
        .get("buttons")
        .and_then(|b| b.as_array())
        .map(|a| {
            a.iter()
                .map(|b| {
                    let f = |k: &str| b.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
                    (f("label"), f("event"))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PopupDef {
        name: name.to_string(),
        title: s("title"),
        body: s("body"),
        use_override: v.get("useoverride").and_then(|x| x.as_bool()).unwrap_or(false),
        show_objective: v.get("showobjective").and_then(|x| x.as_bool()).unwrap_or(false),
        custom: v.get("custom").and_then(|x| x.as_bool()).unwrap_or(false),
        default_button: v.get("defaultButton").and_then(|x| x.as_i64()).unwrap_or(0),
        buttons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_files_and_the_stack() {
        let d = parse(
            "event_quit",
            r#"{"title": "T", "body": "B", "defaultButton": 1, "buttons": [
                {"label": "TXT_YES", "event":"event_quit_yes"},
                {"label": "TXT_NO", "event":"event_quit_no"},
            ]}"#,
        )
        .unwrap();
        assert_eq!(d.buttons.len(), 2);
        assert_eq!(d.buttons[1].1, "event_quit_no");
        assert_eq!(d.default_button, 1);
        let mut p = Popups::default();
        p.add(d.clone()).unwrap();
        assert!(p.add(d.clone()).is_err(), "a popup's name is taken once");
        assert_eq!(p.def("event_quit").map(|x| x.buttons.len()), Some(2));
        assert!(p.push("a", ""));
        assert!(!p.push("b", ""));
        assert!(!p.pop("b"));
        assert!(p.pop("a"));
    }
}
