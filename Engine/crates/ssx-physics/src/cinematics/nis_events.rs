use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ssx_assets::nis::{NisScene, ROLE_CAMERA1, ROLE_CHARACTER1, ROLE_HELICOPTER1, ScriptCall};
use ssx_lua::value::Value;

use crate::nis::IntroState;
use crate::tags::{TagEvent, TagTracker, TagWindow};
use crate::tuning::Bundle;

pub const CHUNK: &str = "script_posted_events";

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Post(String),
    Set(String, bool),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostedEventsChunk {
    pub symbols: Vec<String>,
    pub functions: Vec<Vec<Action>>,
}

impl PostedEventsChunk {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let asset = bundle.stream.find(CHUNK).with_context(|| format!("no {CHUNK} in the bundle"))?;
        let o = &asset.object;
        let schema = bundle.schema();
        let symbols: Vec<String> = o
            .get(schema, "SymbolTable")
            .and_then(ssx_formats::ant::Value::as_array)
            .context("no SymbolTable")?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let code = o.get(schema, "ScriptCode").and_then(ssx_formats::ant::Value::as_bytes).context("no ScriptCode")?;
        Self::run(symbols, code)
    }

    pub fn run(symbols: Vec<String>, code: &[u8]) -> Result<Self> {
        use std::cell::RefCell;
        use std::rc::Rc;
        let mut l = ssx_lua::Lua::new();
        ssx_lua::game::open_libs(&mut l, &ssx_lua::game::LIBS_ANIM);
        let done: Rc<RefCell<Vec<Action>>> = Rc::default();
        let d = done.clone();
        let message = l.lib_table("Message");
        l.lib_fn(message, "Send", move |l, a| {
            let name = l.check_str(a, 1)?.to_str_lossy();
            if name == "MsgPostObjectiveEvent"
                && let Some(Value::Table(t)) = a.get(1)
                && let Some(e) = l.heap.table(*t).get_str("mEventName").to_str_coerce()
            {
                d.borrow_mut().push(Action::Post(e.to_str_lossy()));
            }
            Ok(Vec::new())
        });
        let d = done.clone();
        let gs = l.new_table(0, 0);
        let meta = l.new_table(0, 1);
        let newindex = l.create_function("__newindex", move |_, a| {
            if let (Some(Value::Str(k)), Some(v)) = (a.get(1), a.get(2)) {
                d.borrow_mut().push(Action::Set(k.to_str_lossy(), v.truthy()));
            }
            Ok(Vec::new())
        });
        let _ = l.heap.table_mut(meta).set(Value::str("__newindex"), newindex);
        l.heap.table_mut(gs).meta = Some(meta);
        let ant_self = l.new_table(0, 1);
        let _ = l.heap.table_mut(ant_self).set(Value::str("gs"), Value::Table(gs));
        let ant = l.lib_table("Ant");
        let _ = l.heap.table_mut(ant).set(Value::str("self"), Value::Table(ant_self));
        l.exec(code, CHUNK).map_err(|e| anyhow::anyhow!("{CHUNK}: {}", e.message()))?;
        let mut functions = Vec::with_capacity(symbols.len());
        for s in &symbols {
            done.borrow_mut().clear();
            let f = l.get_global(s);
            if !matches!(f, Value::Func(_)) {
                bail!("{CHUNK}: no function {s}");
            }
            l.call(f, &[]).map_err(|e| anyhow::anyhow!("{CHUNK} {s}: {}", e.message()))?;
            functions.push(done.borrow().clone());
        }
        Ok(PostedEventsChunk { symbols, functions })
    }
}

pub fn role_entity(role: i64) -> String {
    match role {
        ROLE_CHARACTER1 => "Player1".into(),
        ROLE_CAMERA1 => "TheCamera".into(),
        ROLE_HELICOPTER1 => "DropHeli".into(),
        // TODO(guess): the other roles' entities (lhotse's `Player2` …) by the binding's actor table (no consumer reads them); settle: the binding's `actors` entries for the role.
        r => {
            crate::guess_hit!();
            format!("role {r}")
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Tag {
    enter: i64,
    exit: i64,
    owners: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
struct TagSet {
    tracker: TagTracker,
    tags: Vec<Tag>,
}

impl TagSet {
    fn new<'a>(calls: impl Iterator<Item = (&'a ScriptCall, Vec<String>)>) -> Self {
        let (windows, tags): (Vec<TagWindow>, Vec<Tag>) = calls
            .filter(|(c, _)| c.script == CHUNK)
            .map(|(c, owners)| {
                let w = TagWindow::new(c.window.time, c.window.duration, c.window.always_active);
                (w, Tag { enter: c.enter, exit: c.exit, owners })
            })
            .unzip();
        TagSet { tracker: TagTracker::new(&windows), tags }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct SceneTags {
    key: (String, Option<usize>),
    actors: TagSet,
    node: TagSet,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NisEvents {
    pub chunk: Arc<PostedEventsChunk>,
    scene: Option<SceneTags>,
    leaving: Vec<TagSet>,
    pub gs: BTreeMap<(String, String), bool>,
    pub posted: Vec<String>,
    pub ran: Vec<String>,
}

impl NisEvents {
    pub fn new(chunk: Arc<PostedEventsChunk>) -> Self {
        NisEvents { chunk, ..Default::default() }
    }

    pub fn value(&self, entity: &str, name: &str) -> Option<bool> {
        self.gs.get(&(entity.to_string(), name.to_string())).copied()
    }

    fn run(&mut self, ordinal: i64, owners: &[String]) {
        let Some(actions) = usize::try_from(ordinal).ok().and_then(|i| self.chunk.functions.get(i)).cloned() else {
            return;
        };
        let name = usize::try_from(ordinal).ok().and_then(|i| self.chunk.symbols.get(i)).cloned().unwrap_or_default();
        for owner in owners {
            self.ran.push(format!("{owner}: {name}"));
            for a in &actions {
                match a {
                    Action::Post(e) => self.posted.push(e.clone()),
                    Action::Set(var, v) => {
                        self.gs.insert((owner.clone(), var.clone()), *v);
                    }
                }
            }
        }
    }

    fn dispatch(&mut self, events: Vec<(usize, TagEvent)>, tags: &[Tag]) {
        for (i, e) in events {
            let t = &tags[i];
            match e {
                TagEvent::Enter => self.run(t.enter, &t.owners),
                TagEvent::Leave(_) => self.run(t.exit, &t.owners),
                TagEvent::Apply(_) => {}
            }
        }
    }

    pub fn update(&mut self, intro: Option<&IntroState>) {
        self.ran.clear();
        self.posted.clear();
        for set in std::mem::take(&mut self.leaving) {
            let mut tracker = set.tracker;
            let events = tracker.exit();
            self.dispatch(events, &set.tags);
        }
        let key = intro.map(|i| (i.current().node.clone(), i.cabin.map(|c| c.index)));
        if self.scene.as_ref().map(|s| &s.key) != key.as_ref() {
            if let Some(old) = self.scene.take() {
                self.leaving.extend([old.actors, old.node]);
            }
            if let (Some(i), Some(key)) = (intro, key) {
                self.scene = Some(scene_tags(i.current(), key));
            }
        }
        if let (Some(i), Some(mut sc)) = (intro, self.scene.take()) {
            let t = i.sequence_time(i.ticks());
            let node_t = (i.ticks() - i.dt).max(0.0);
            for (set, time) in [(&mut sc.actors, t), (&mut sc.node, node_t)] {
                if set.tracker.update(time) {
                    let events = set.tracker.process();
                    let tags = set.tags.clone();
                    self.dispatch(events, &tags);
                }
            }
            self.scene = Some(sc);
        }
    }
}

fn scene_tags(sc: &NisScene, key: (String, Option<usize>)) -> SceneTags {
    let actors = TagSet::new(sc.actors.iter().flat_map(|a| {
        let owner = role_entity(a.role);
        a.tags.scripts.iter().map(move |c| (c, vec![owner.clone()]))
    }));
    let mut bound: Vec<String> = sc.actors.iter().map(|a| role_entity(a.role)).collect();
    bound.sort_unstable();
    bound.dedup();
    let node = TagSet::new(sc.node_tags.scripts.iter().map(|c| (c, bound.clone())));
    SceneTags { key, actors, node }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &[u8] = br#"local function PostObjectiveEvent(eventName)
	local message = { mEventName = eventName }
	Message.Send("MsgPostObjectiveEvent", message)
end
function PostEvent_NIS_Foot_Impact_Snow_Left()
	PostObjectiveEvent("NIS_Foot_Impact_Snow_Left")
end
function PostEvent_HeliDoorWarningLightOn()
	local gs = Ant.self.gs
	gs.BLN_HeliDoorWarningLightOn = true
end
"#;

    #[test]
    fn the_chunk_functions_run_by_symbol_order() {
        let symbols = vec!["PostEvent_NIS_Foot_Impact_Snow_Left".into(), "PostEvent_HeliDoorWarningLightOn".into()];
        let c = PostedEventsChunk::run(symbols, CODE).expect("runs");
        assert_eq!(c.functions[0], vec![Action::Post("NIS_Foot_Impact_Snow_Left".into())]);
        assert_eq!(c.functions[1], vec![Action::Set("BLN_HeliDoorWarningLightOn".into(), true)]);
        assert!(PostedEventsChunk::run(vec!["Missing".into()], CODE).is_err());
        let mut n = NisEvents::new(Arc::new(c));
        n.run(1, &["DropHeli".to_string()]);
        n.run(0, &["Player1".to_string(), "TheCamera".to_string()]);
        assert_eq!(n.value("DropHeli", "BLN_HeliDoorWarningLightOn"), Some(true));
        assert_eq!(n.posted, ["NIS_Foot_Impact_Snow_Left", "NIS_Foot_Impact_Snow_Left"]);
    }
}
