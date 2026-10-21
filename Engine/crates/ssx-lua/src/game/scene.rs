use std::collections::BTreeMap;

use super::graph::{Arg, Graph};

#[derive(Clone, Debug, PartialEq)]
pub struct SceneEntity {
    pub schema: String,
    pub name: Option<String>,
    pub count: f64,
    pub set: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub sets: Vec<(String, f64)>,
    pub entities: Vec<SceneEntity>,
    pub players: Vec<(f64, Option<String>)>,
    pub scripts: Vec<(String, Option<String>)>,
}

impl Scene {
    pub fn from_graph(g: &Graph) -> Self {
        let mut s = Scene { scripts: g.scripts.clone(), ..Default::default() };
        let mut names = BTreeMap::new();
        for c in &g.calls {
            let text = |i: usize| c.arg(i).str().map(str::to_string);
            match c.name() {
                "CreateWorldEntitySet" => s.sets.push((text(1).unwrap_or_default(), c.arg(2).num().unwrap_or(0.0))),
                "CreateWorldEntity" => {
                    if let Some(o) = c.ret {
                        names.insert(o, text(2));
                    }
                    s.entities.push(SceneEntity {
                        schema: text(1).unwrap_or_default(),
                        name: text(2),
                        count: 1.0,
                        set: None,
                    });
                }
                "CreateWorldEntities" => s.entities.push(SceneEntity {
                    schema: text(1).unwrap_or_default(),
                    name: None,
                    count: c.arg(2).num().unwrap_or(0.0),
                    set: text(3),
                }),
                "SetPlayerWorldEntity" => {
                    let entity = c.arg(2).obj().and_then(|o| names.get(&o).cloned()).flatten();
                    s.players.push((c.arg(1).num().unwrap_or(0.0), entity));
                }
                _ => {}
            }
        }
        s
    }

    pub fn schema_of(&self, name: &str) -> Option<&str> {
        self.entities.iter().find(|e| e.name.as_deref() == Some(name)).map(|e| e.schema.as_str())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComponentFactory {
    pub kind: String,
    pub calls: Vec<(String, Vec<Arg>)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub controller: Option<String>,
    pub default_state: Option<String>,
    pub factories: Vec<ComponentFactory>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schemas {
    pub schemas: BTreeMap<String, Schema>,
    pub scripts: Vec<(String, Option<String>)>,
}

impl Schemas {
    pub fn from_graph(g: &Graph) -> Self {
        let mut out = Schemas { scripts: g.scripts.clone(), ..Default::default() };
        let mut by_obj = BTreeMap::new();
        for c in &g.calls {
            match (c.name(), c.ret) {
                ("CreateWorldEntitySchema", Some(o)) => {
                    let name = c.arg(1).str().unwrap_or_default().to_string();
                    out.schemas.insert(name.clone(), Schema::default());
                    by_obj.insert(o, name);
                }
                ("SetController" | "SetDefaultState" | "AddComponentFactory", _) => {
                    let Some(name) = c.arg(1).obj().and_then(|o| by_obj.get(&o)) else { continue };
                    let Some(s) = out.schemas.get_mut(name) else { continue };
                    let text = c.arg(2).str().map(str::to_string);
                    match c.name() {
                        "SetController" => s.controller = text,
                        "SetDefaultState" => s.default_state = text,
                        _ => {
                            let Some(f) = c.arg(2).obj() else { continue };
                            let kind =
                                g.creator(f).map_or("", |k| k.name()).trim_start_matches("CreateComponentFactory_");
                            let calls = g.methods(f).map(|m| (m.name().to_string(), m.args[1..].to_vec())).collect();
                            s.factories.push(ComponentFactory { kind: kind.to_string(), calls });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }
}
