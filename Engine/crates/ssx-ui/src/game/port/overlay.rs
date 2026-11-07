use anyhow::{Context, Result, bail};
use serde_json::Value as Json;

#[derive(Clone, Debug)]
pub struct Overlay {
    pub states: Vec<AddState>,
    pub transitions: Vec<AddTransition>,
}

#[derive(Clone, Debug)]
pub struct AddState {
    pub parent: String,
    pub state: Json,
}

#[derive(Clone, Debug)]
pub struct AddTransition {
    pub state: String,
    pub transition: Json,
}

fn members<'a>(o: &'a Json, what: &str, allowed: &[&str]) -> Result<&'a serde_json::Map<String, Json>> {
    let m = o.as_object().with_context(|| format!("{what}: not an object"))?;
    if let Some(k) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        bail!("{what}: unknown member `{k}`");
    }
    Ok(m)
}

fn string(m: &serde_json::Map<String, Json>, key: &str, what: &str) -> Result<String> {
    m.get(key).and_then(Json::as_str).map(str::to_string).with_context(|| format!("{what}: `{key}` must be a string"))
}

const MAX_ADDS: usize = 64;

impl Overlay {
    pub fn parse(text: &str) -> Result<Overlay> {
        let j: Json = serde_json::from_str(text).context("the nav overlay")?;
        let m = members(&j, "the nav overlay", &["format", "states", "transitions"])?;
        if m.get("format").and_then(Json::as_u64) != Some(1) {
            bail!("the nav overlay: `format` must be 1");
        }
        let list = |key: &str| -> Result<Vec<Json>> {
            match m.get(key) {
                None => Ok(Vec::new()),
                Some(Json::Array(a)) => Ok(a.clone()),
                Some(_) => bail!("the nav overlay: `{key}` must be an array"),
            }
        };
        let mut states = Vec::new();
        for s in list("states")? {
            let m = members(&s, "an added state", &["parent", "state"])?;
            let state =
                m.get("state").filter(|v| v.is_object()).context("an added state: `state` must be an object")?;
            states.push(AddState { parent: string(m, "parent", "an added state")?, state: state.clone() });
        }
        let mut transitions = Vec::new();
        for t in list("transitions")? {
            let m = members(&t, "an added transition", &["state", "transition"])?;
            let tr = m
                .get("transition")
                .filter(|v| v.is_object())
                .context("an added transition: `transition` must be an object")?;
            transitions
                .push(AddTransition { state: string(m, "state", "an added transition")?, transition: tr.clone() });
        }
        if states.len() + transitions.len() > MAX_ADDS {
            bail!("the nav overlay: more than {MAX_ADDS} additions");
        }
        Ok(Overlay { states, transitions })
    }

    pub fn apply(&self, root: &Json) -> Result<Json> {
        let mut root = root.clone();
        for add in &self.states {
            let name = add.state.get("name").and_then(Json::as_str).context("an added state without a name")?;
            if find(&root, name).is_some() {
                bail!("the nav overlay adds {name}, which the statechart has");
            }
            let parent = find_mut(&mut root, &add.parent).with_context(|| format!("no state {}", add.parent))?;
            let obj = parent.as_object_mut().context("a state is not an object")?;
            let states = obj.entry("states").or_insert_with(|| Json::Array(Vec::new()));
            states.as_array_mut().context("`states` is not an array")?.push(add.state.clone());
        }
        for add in &self.transitions {
            let state = find_mut(&mut root, &add.state).with_context(|| format!("no state {}", add.state))?;
            let obj = state.as_object_mut().context("a state is not an object")?;
            let list = obj.entry("transitions").or_insert_with(|| Json::Array(Vec::new()));
            let list = list.as_array_mut().context("`transitions` is not an array")?;
            let event = add.transition.get("event").and_then(Json::as_str).context("a transition without an event")?;
            if list.iter().any(|t| t.get("event").and_then(Json::as_str) == Some(event)) {
                bail!("the nav overlay adds {event} to {}, which has a transition on it", add.state);
            }
            list.push(add.transition.clone());
        }
        Ok(root)
    }
}

fn find<'a>(s: &'a Json, name: &str) -> Option<&'a Json> {
    if s.get("name").and_then(Json::as_str) == Some(name) {
        return Some(s);
    }
    s.get("states").and_then(Json::as_array)?.iter().find_map(|c| find(c, name))
}

fn find_mut<'a>(s: &'a mut Json, name: &str) -> Option<&'a mut Json> {
    if s.get("name").and_then(Json::as_str) == Some(name) {
        return Some(s);
    }
    s.get_mut("states").and_then(Json::as_array_mut)?.iter_mut().find_map(|c| find_mut(c, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart() -> Json {
        serde_json::json!({"name": "root", "type": "parallel", "states": [
            {"name": "main", "initial": {"targets": ["A"]}, "states": [
                {"name": "A", "transitions": [{"event": "go_b", "targets": ["B"]}]},
                {"name": "B"}
            ]}
        ]})
    }

    #[test]
    fn states_and_transitions_are_added() {
        let o = Overlay::parse(
            r#"{"format": 1,
                "states": [{"parent": "main", "state": {"name": "C", "onEnter": [["loadView", ["c"]]]}}],
                "transitions": [{"state": "main", "transition": {"event": "go_c", "targets": ["C"]}}]}"#,
        )
        .unwrap();
        let j = o.apply(&chart()).unwrap();
        let mut nav = crate::game::nav::Nav::from_json(&j).unwrap();
        while nav.next_action().is_some() {
            nav.complete();
        }
        nav.raise("go_c");
        assert_eq!(nav.next_action().map(|a| a.to_string()), Some("loadView(c)".into()));
        nav.complete();
        assert!(nav.is_active("C"));
    }

    #[test]
    fn the_disc_states_are_only_extended() {
        let dup = r#"{"format": 1, "states": [{"parent": "main", "state": {"name": "A"}}]}"#;
        assert!(Overlay::parse(dup).unwrap().apply(&chart()).is_err());
        let taken = r#"{"format": 1, "transitions": [{"state": "A", "transition": {"event": "go_b", "targets": []}}]}"#;
        assert!(Overlay::parse(taken).unwrap().apply(&chart()).is_err());
        let missing = r#"{"format": 1, "transitions": [{"state": "Z", "transition": {"event": "x"}}]}"#;
        assert!(Overlay::parse(missing).unwrap().apply(&chart()).is_err());
        assert!(Overlay::parse(r#"{"format": 1, "extra": 1}"#).is_err());
        assert!(Overlay::parse(r#"{"format": 2}"#).is_err());
    }

    fn disc_shape(start: &str) -> Json {
        serde_json::json!({"name": "root", "type": "parallel", "states": [
            {"name": "main", "initial": {"targets": [start]}, "states": [
                {"name": "BootFlow", "states": [{"name": "bootlogo"}]},
                {"name": "ActivityFeedEnabled", "states": [
                    {"name": "MainFrontEndGlobe", "states": [{"name": "FrontEndGlobe", "states": [{"name": "MenuHub"}]}]},
                    {"name": "MainActivityFeed", "states": [{"name": "ActivityFeed", "states": [{"name": "ActivityFeedClosed"}]}]}
                ]},
                {"name": "EquipmentLoading"},
                {"name": "Gameplay", "states": [
                    {"name": "HUDLoaded", "states": [{"name": "HUD", "transitions": [
                        {"event": "go_to_quit_stage", "targets": ["QuitStaging"]}
                    ]}]},
                    {"name": "HUDUnloaded", "states": [{"name": "QuitStaging"}]}
                ]}
            ]}
        ]})
    }

    fn settle(nav: &mut crate::game::nav::Nav) {
        while nav.next_action().is_some() {
            nav.complete();
        }
    }

    #[test]
    fn the_maps_page_is_reached_from_a_quit_a_refused_load_and_the_hub() {
        let text = super::super::read_text(
            &super::super::files::PortFiles,
            &format!("{}nav/overlay.json", super::super::ROOT),
        )
        .unwrap();
        let overlay = Overlay::parse(&text).unwrap();
        for (start, before) in [("HUD", Some("go_to_quit_stage")), ("EquipmentLoading", None), ("MenuHub", None)] {
            let mut nav = crate::game::nav::Nav::from_json(&overlay.apply(&disc_shape(start)).unwrap()).unwrap();
            settle(&mut nav);
            if let Some(e) = before {
                nav.raise(e);
                settle(&mut nav);
                assert!(nav.is_active("QuitStaging"));
            }
            nav.raise("go_to_ava_maps");
            settle(&mut nav);
            assert!(nav.is_active("AvaMaps") && nav.is_active("FrontEndGlobe"), "from {start}");
            assert!(!nav.is_active("Gameplay") && !nav.is_active("EquipmentLoading"), "from {start}");
        }
    }
}
