use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use serde_json::Value as Json;

use crate::fs::FileSource;

#[derive(Clone, Debug, Default)]
pub struct ViewModel {
    pub name: String,
    pub view: String,
    pub providers: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct DataDefinition {
    pub actions: Vec<(String, String)>,
    pub data: Vec<(String, String, String)>,
    pub bindings: Vec<(String, String)>,
    pub data_actions: Vec<(String, String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscriber {
    pub path: String,
    pub name: String,
    pub dp: String,
    pub provider: String,
    pub served: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DataStore {
    pub view_models: BTreeMap<String, ViewModel>,
    pub definitions: BTreeMap<String, DataDefinition>,
    pub active: Vec<String>,
    pub loaded_view_models: Vec<String>,
    pub subscribers: Vec<Subscriber>,
    pub lenient: Vec<String>,
    pub unresolved: BTreeMap<String, u32>,
}

const VM_DIR: &str = "data/common/fe/viewmodels/";

impl DataStore {
    pub fn load(files: &dyn FileSource, vm_paths: &[String], dd_paths: &[String]) -> Result<Self> {
        let mut s = DataStore::default();
        for p in vm_paths {
            let j = s.read_json(files, p)?;
            let name = p.rsplit('/').next().unwrap_or(p).trim_end_matches(".vm").to_string();
            s.view_models.insert(name.clone(), view_model(&name, &j));
        }
        for p in dd_paths {
            let j = s.read_json(files, p)?;
            let name = p.rsplit('/').next().unwrap_or(p).trim_end_matches(".dd").to_string();
            s.definitions.insert(name, definition(&j));
        }
        Ok(s)
    }

    pub fn add_view_model(&mut self, name: &str, text: &str) -> Result<()> {
        if self.view_models.contains_key(name) {
            anyhow::bail!("the view model {name} exists on the disc");
        }
        let j: Json = serde_json::from_str(text).with_context(|| format!("parsing {name}.vm"))?;
        self.view_models.insert(name.to_string(), view_model(name, &j));
        Ok(())
    }

    pub fn add_definition(&mut self, name: &str, text: &str) -> Result<()> {
        if self.definitions.contains_key(name) {
            anyhow::bail!("the data definition {name} exists on the disc");
        }
        let j: Json = serde_json::from_str(text).with_context(|| format!("parsing {name}.dd"))?;
        self.definitions.insert(name.to_string(), definition(&j));
        Ok(())
    }

    pub fn extend_view_model(&mut self, view_model: &str, provider: &str) -> Result<()> {
        if !self.definitions.contains_key(provider) {
            anyhow::bail!("the provider {provider} has no data definition");
        }
        let vm = self.view_models.get_mut(view_model).with_context(|| format!("no view model {view_model}"))?;
        if vm.providers.iter().any(|p| p == provider) {
            anyhow::bail!("the view model {view_model} names {provider} already");
        }
        vm.providers.push(provider.to_string());
        Ok(())
    }

    pub fn load_files(files: &dyn FileSource) -> Result<Self> {
        let vms = files.list(VM_DIR, "vm");
        let dds = files.list(&format!("{VM_DIR}datadefinitions/"), "dd");
        Self::load(files, &vms, &dds)
    }

    // TODO(guess): `.dd` files with a trailing comma (`badges.dd`) are read by dropping it; settle: the game's JSON reader for `.dd` files (the provider's loader).
    fn read_json(&mut self, files: &dyn FileSource, path: &str) -> Result<Json> {
        let data = files.read(path).with_context(|| format!("{path} not found"))?;
        let text = String::from_utf8_lossy(&data);
        let text = text.trim_start_matches('\u{feff}');
        match serde_json::from_str(text) {
            Ok(j) => Ok(j),
            Err(_) => {
                let j = serde_json::from_str(&drop_trailing_commas(text)).with_context(|| format!("parsing {path}"))?;
                self.lenient.push(path.to_string());
                Ok(j)
            }
        }
    }

    pub fn activate(&mut self, view_model: &str) -> Option<&ViewModel> {
        let vm = self.view_models.get(view_model)?;
        if !self.loaded_view_models.iter().any(|v| v == view_model) {
            self.loaded_view_models.push(view_model.to_string());
            for p in &vm.providers {
                if !self.active.contains(p) {
                    self.active.push(p.clone());
                }
            }
        }
        self.view_models.get(view_model)
    }

    pub fn deactivate(&mut self, view_model: &str) {
        self.loaded_view_models.retain(|v| v != view_model);
        let still: Vec<String> = self
            .loaded_view_models
            .iter()
            .filter_map(|v| self.view_models.get(v))
            .flat_map(|vm| vm.providers.iter().cloned())
            .collect();
        self.active.retain(|p| still.contains(p));
    }

    pub fn binding(&mut self, name: &str) -> Option<String> {
        let found = self
            .active
            .iter()
            .find_map(|p| self.definitions.get(p)?.bindings.iter().find(|(v, _)| v == name).map(|(_, b)| b.clone()));
        if found.is_none() {
            *self.unresolved.entry(format!("binding {name}")).or_default() += 1;
        }
        found
    }

    pub fn binding_any(&self, name: &str) -> Option<String> {
        self.definitions.values().find_map(|d| d.bindings.iter().find(|(v, _)| v == name).map(|(_, b)| b.clone()))
    }

    pub fn subscribe(&mut self, path: &str, name: &str) -> bool {
        let found = self
            .active
            .iter()
            .find_map(|p| {
                let d = self.definitions.get(p)?;
                d.data.iter().find(|(v, _, _)| v == name).map(|(_, dp, _)| (p.clone(), dp.clone()))
            })
            .or_else(|| (name == super::fe::popup::DP_POPUP).then(|| ("popup".to_string(), name.to_string())));
        match found {
            Some((provider, dp)) => {
                if !self.subscribers.iter().any(|s| s.path == path && s.name == name) {
                    self.subscribers.push(Subscriber {
                        path: path.to_string(),
                        name: name.to_string(),
                        dp,
                        provider,
                        served: false,
                    });
                }
                true
            }
            None => {
                *self.unresolved.entry(format!("data {name}")).or_default() += 1;
                false
            }
        }
    }

    pub fn unsubscribe(&mut self, path: &str, name: &str) {
        self.subscribers.retain(|s| !(s.path == path && s.name == name));
    }

    pub fn action(&self, name: &str) -> Option<&str> {
        self.active
            .iter()
            .find_map(|p| self.definitions.get(p)?.actions.iter().find(|(v, _)| v == name).map(|(_, a)| a.as_str()))
    }

    pub fn data_action(&self, data: &str, action: &str) -> Option<(String, String, String)> {
        self.active.iter().find_map(|p| {
            let d = self.definitions.get(p)?;
            let act = d.data_actions.iter().find(|(v, a, _)| v == data && a == action).map(|(_, _, n)| n)?;
            let dp = d.data.iter().find(|(v, _, _)| v == data).map(|(_, dp, _)| dp.clone())?;
            Some((p.clone(), dp, act.clone()))
        })
    }

    pub fn action_provider(&self, name: &str) -> Option<(String, String)> {
        self.active.iter().find_map(|p| {
            let d = self.definitions.get(p)?;
            d.actions.iter().find(|(v, _)| v == name).map(|(_, a)| (p.clone(), a.clone()))
        })
    }

    pub fn active_bindings(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for p in &self.active {
            if let Some(d) = self.definitions.get(p) {
                for (v, b) in &d.bindings {
                    out.entry(v.clone()).or_insert_with(|| b.clone());
                }
            }
        }
        out
    }
}

fn view_model(name: &str, j: &Json) -> ViewModel {
    let view = j.get("view").and_then(Json::as_str).unwrap_or_default().to_string();
    let providers = j
        .get("dataproviders")
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    ViewModel { name: name.to_string(), view, providers }
}

fn definition(j: &Json) -> DataDefinition {
    let list = |k: &str| j.get(k).and_then(Json::as_array).cloned().unwrap_or_default();
    let field = |o: &Json, k: &str| o.get(k).and_then(Json::as_str).unwrap_or_default().to_string();
    DataDefinition {
        actions: list("actions").iter().map(|o| (field(o, "viewname"), field(o, "actionname"))).collect(),
        data: list("data").iter().map(|o| (field(o, "viewname"), field(o, "dpname"), field(o, "type"))).collect(),
        bindings: list("databindings").iter().map(|o| (field(o, "viewname"), field(o, "bindingname"))).collect(),
        data_actions: list("data")
            .iter()
            .flat_map(|d| {
                let data = field(d, "viewname");
                d.get("actions")
                    .and_then(Json::as_array)
                    .into_iter()
                    .flatten()
                    .map(move |a| (data.clone(), field(a, "viewname"), field(a, "actionname")))
                    .collect::<Vec<_>>()
            })
            .collect(),
    }
}

pub(crate) fn drop_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_str = false;
    let mut escaped = false;
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if in_str {
            out.push(c);
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_str = false,
                _ => {}
            }
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        if c == ',' && chars[i + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|n| matches!(n, ']' | '}')) {
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_port_provider_extends_a_disc_view_model_once() {
        let mut s = DataStore::default();
        s.view_models.insert(
            "mainmenu".into(),
            view_model(
                "mainmenu",
                &serde_json::json!({"view": "mainmenu",
            "dataproviders": ["menuhub"]}),
            ),
        );
        assert!(s.extend_view_model("mainmenu", "avamainmenu").is_err(), "a provider without a definition");
        s.add_definition("avamainmenu", r#"{"actions": [{"viewname": "A_X", "actionname": "A_X"}], "data": []}"#)
            .unwrap();
        assert!(s.add_definition("avamainmenu", "{}").is_err(), "a definition's name is taken once");
        assert!(s.extend_view_model("nomenu", "avamainmenu").is_err());
        s.extend_view_model("mainmenu", "avamainmenu").unwrap();
        assert!(s.extend_view_model("mainmenu", "avamainmenu").is_err());
        assert!(s.extend_view_model("mainmenu", "menuhub").is_err());
        assert_eq!(
            s.activate("mainmenu").map(|v| v.providers.clone()),
            Some(vec!["menuhub".into(), "avamainmenu".into()])
        );
        assert_eq!(s.action_provider("A_X"), Some(("avamainmenu".into(), "A_X".into())));
    }

    #[test]
    fn trailing_commas() {
        let j: Json = serde_json::from_str(&drop_trailing_commas(r#"{"a": [1, 2, ], "b": "x,]", }"#)).unwrap();
        assert_eq!(j["a"].as_array().map(Vec::len), Some(2));
        assert_eq!(j["b"], "x,]");
    }
}
