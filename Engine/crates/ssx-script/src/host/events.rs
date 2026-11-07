use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use mlua::RegistryKey;

use super::elements::Tree;
use crate::value::ElementId;

pub const SERVER_EVENTS: &[&str] = &[
    "onResourceStart",
    "onResourceStop",
    "onElementDataChange",
    "onPlayerConnecting",
    "onPlayerJoin",
    "onPlayerQuit",
    "onRiderCheckpoint",
    "onRiderFinish",
    "onRoundStart",
    "onRoundEnd",
    "onMarkerHit",
    "onPlayerTriggerInvalidEvent",
    "onPlayerEventFlood",
    "onResourcePreStart",
    "onResourceStateChange",
    "onPlayerResourceStart",
    "onShutdown",
];
pub const CLIENT_EVENTS: &[&str] = &[
    "onClientResourceStart",
    "onClientResourceStop",
    "onClientElementDataChange",
    "onClientRender",
    "onClientKey",
    "onClientRiderLand",
    "onClientTrick",
    "onClientMarkerHit",
];

pub struct HostEvent {
    pub name: &'static str,
    pub side: crate::api::Sides,
    pub params: &'static [crate::api::Param],
}

const fn arg(name: &'static str, ty: crate::api::Ty) -> crate::api::Param {
    crate::api::Param { name, ty }
}

pub const HOST_EVENTS: &[HostEvent] = {
    use crate::api::{Sides, Ty};
    const KEY: Ty = Ty::Enum(super::game::keys::BINDABLE);
    const JOIN: Ty = Ty::Int { min: 1, max: 1 << 53 };
    &[
        HostEvent { name: "onResourceStart", side: Sides::Server, params: &[] },
        HostEvent { name: "onResourceStop", side: Sides::Server, params: &[] },
        HostEvent {
            name: "onElementDataChange",
            side: Sides::Server,
            params: &[arg("key", Ty::Str { max: 128 }), arg("old", Ty::Value), arg("new", Ty::Value)],
        },
        HostEvent {
            name: "onPlayerConnecting",
            side: Sides::Server,
            params: &[arg("name", Ty::Str { max: 64 }), arg("keyId", Ty::Str { max: 16 }), arg("join", JOIN)],
        },
        HostEvent { name: "onPlayerJoin", side: Sides::Server, params: &[] },
        HostEvent { name: "onPlayerQuit", side: Sides::Server, params: &[arg("reason", Ty::Str { max: 256 })] },
        HostEvent {
            name: "onRiderCheckpoint",
            side: Sides::Server,
            params: &[
                arg("checkpoint", Ty::Opt(&Ty::Element)),
                arg("index", Ty::Int { min: 0, max: 65_535 }),
                arg("clock", Ty::Number { min: 0.0, max: 1.0e9 }),
            ],
        },
        HostEvent {
            name: "onRiderFinish",
            side: Sides::Server,
            params: &[
                arg("clock", Ty::Number { min: 0.0, max: 1.0e9 }),
                arg("points", Ty::Number { min: 0.0, max: 1.0e12 }),
                arg("failed", Ty::Bool),
            ],
        },
        HostEvent {
            name: "onRoundStart",
            side: Sides::Server,
            params: &[arg("round", Ty::Int { min: 0, max: 65_535 }), arg("drop", Ty::Str { max: 256 })],
        },
        HostEvent { name: "onRoundEnd", side: Sides::Server, params: &[arg("round", Ty::Int { min: 0, max: 65_535 })] },
        HostEvent {
            name: "onMarkerHit",
            side: Sides::Server,
            params: &[arg("rider", Ty::Element), arg("forward", Ty::Bool)],
        },
        HostEvent {
            name: "onPlayerTriggerInvalidEvent",
            side: Sides::Server,
            params: &[
                arg("eventName", Ty::Str { max: 128 }),
                arg("isAdded", Ty::Bool),
                arg("isRemote", Ty::Bool),
                arg("package", Ty::Str { max: 64 }),
            ],
        },
        HostEvent {
            name: "onPlayerEventFlood",
            side: Sides::Server,
            params: &[arg("dropped", Ty::Int { min: 1, max: 1 << 53 })],
        },
        HostEvent { name: "onResourcePreStart", side: Sides::Server, params: &[arg("resource", Ty::Str { max: 64 })] },
        HostEvent {
            name: "onResourceStateChange",
            side: Sides::Server,
            params: &[
                arg("resource", Ty::Str { max: 64 }),
                arg("old", Ty::Enum(super::STATES)),
                arg("new", Ty::Enum(super::STATES)),
            ],
        },
        HostEvent {
            name: "onPlayerResourceStart",
            side: Sides::Server,
            params: &[arg("resource", Ty::Str { max: 64 })],
        },
        HostEvent {
            name: "onShutdown",
            side: Sides::Server,
            params: &[arg("roundEndSecs", Ty::Int { min: 0, max: 600 })],
        },
        HostEvent { name: "onClientResourceStart", side: Sides::Client, params: &[] },
        HostEvent { name: "onClientResourceStop", side: Sides::Client, params: &[] },
        HostEvent {
            name: "onClientElementDataChange",
            side: Sides::Client,
            params: &[arg("key", Ty::Str { max: 128 }), arg("old", Ty::Value), arg("new", Ty::Value)],
        },
        HostEvent { name: "onClientRender", side: Sides::Client, params: &[] },
        HostEvent { name: "onClientKey", side: Sides::Client, params: &[arg("key", KEY), arg("down", Ty::Bool)] },
        HostEvent {
            name: "onClientRiderLand",
            side: Sides::Client,
            params: &[arg("airTime", Ty::Number { min: 0.0, max: 1.0e6 }), arg("bail", Ty::Bool)],
        },
        HostEvent {
            name: "onClientTrick",
            side: Sides::Client,
            params: &[arg("name", Ty::Str { max: 128 }), arg("points", Ty::Number { min: -1.0e12, max: 1.0e12 })],
        },
        HostEvent {
            name: "onClientMarkerHit",
            side: Sides::Client,
            params: &[arg("rider", Ty::Element), arg("forward", Ty::Bool)],
        },
    ]
};

#[derive(Clone)]
pub struct Callback {
    pub key: Arc<RegistryKey>,
    pub ptr: usize,
    pub at: String,
}

pub struct HandlerRec {
    pub package: String,
    pub event: String,
    pub element: ElementId,
    pub propagate: bool,
    pub callback: Callback,
}

pub struct CommandRec {
    pub package: String,
    pub callback: Callback,
}

#[derive(Default)]
pub struct Events {
    declared: BTreeMap<String, BTreeMap<String, bool>>,
    declared_by: BTreeMap<String, usize>,
    handlers: BTreeMap<u64, HandlerRec>,
    by_event: BTreeMap<String, BTreeSet<u64>>,
    by_package: BTreeMap<String, BTreeSet<u64>>,
    by_element: HashMap<ElementId, BTreeSet<u64>>,
    by_key: HashMap<(String, String, ElementId, usize), u64>,
    next: u64,
    commands: BTreeMap<String, Vec<CommandRec>>,
    commands_by: BTreeMap<String, usize>,
}

impl Events {
    pub fn is_declared(&self, name: &str) -> bool {
        self.declared.get(name).is_some_and(|d| !d.is_empty())
    }

    pub fn declared_by(&self, name: &str, package: &str) -> bool {
        self.declared.get(name).is_some_and(|d| d.contains_key(package))
    }

    pub fn is_remote_for(&self, name: &str, package: &str) -> bool {
        self.declared.get(name).and_then(|d| d.get(package)).copied().unwrap_or(false)
    }

    pub fn declare(&mut self, package: &str, name: &str, remote: bool) {
        let d = self.declared.entry(name.to_string()).or_default();
        if d.insert(package.to_string(), remote).is_none() {
            *self.declared_by.entry(package.to_string()).or_default() += 1;
        }
    }

    pub fn handler(&self, id: u64) -> Option<&HandlerRec> {
        self.handlers.get(&id)
    }

    pub fn add_handler(&mut self, rec: HandlerRec) -> u64 {
        self.next += 1;
        self.by_event.entry(rec.event.clone()).or_default().insert(self.next);
        self.by_package.entry(rec.package.clone()).or_default().insert(self.next);
        self.by_element.entry(rec.element).or_default().insert(self.next);
        self.by_key.insert((rec.package.clone(), rec.event.clone(), rec.element, rec.callback.ptr), self.next);
        self.handlers.insert(self.next, rec);
        self.next
    }

    pub fn find_handler(&self, package: &str, event: &str, element: ElementId, ptr: usize) -> Option<u64> {
        self.by_key.get(&(package.to_string(), event.to_string(), element, ptr)).copied()
    }

    pub fn remove_handler(&mut self, id: u64) {
        let Some(h) = self.handlers.remove(&id) else { return };
        self.by_key.remove(&(h.package.clone(), h.event.clone(), h.element, h.callback.ptr));
        for (index, key) in [(&mut self.by_event, &h.event), (&mut self.by_package, &h.package)] {
            if let Some(set) = index.get_mut(key) {
                set.remove(&id);
                if set.is_empty() {
                    index.remove(key);
                }
            }
        }
        if let Some(set) = self.by_element.get_mut(&h.element) {
            set.remove(&id);
            if set.is_empty() {
                self.by_element.remove(&h.element);
            }
        }
    }

    pub fn reached(&self, tree: &Tree, event: &str, source: ElementId, only: Option<&str>) -> Vec<u64> {
        let ids = self.by_event.get(event).into_iter().flatten();
        ids.filter(|id| {
            self.handlers.get(id).is_some_and(|h| {
                only.is_none_or(|p| p == h.package)
                    && (h.element == source
                        || (h.propagate
                            && (tree.is_ancestor(h.element, source) || tree.is_ancestor(source, h.element))))
            })
        })
        .copied()
        .collect()
    }

    pub fn handlers_of(&self, package: &str) -> usize {
        self.by_package.get(package).map_or(0, BTreeSet::len)
    }

    pub fn declared_of(&self, package: &str) -> usize {
        self.declared_by.get(package).copied().unwrap_or(0)
    }

    pub fn commands_of(&self, package: &str) -> usize {
        self.commands_by.get(package).copied().unwrap_or(0)
    }

    pub fn add_command(&mut self, name: &str, rec: CommandRec) {
        *self.commands_by.entry(rec.package.clone()).or_default() += 1;
        self.commands.entry(name.to_string()).or_default().push(rec);
    }

    pub fn remove_command(&mut self, name: &str, package: &str, ptr: Option<usize>) -> usize {
        let Some(list) = self.commands.get_mut(name) else { return 0 };
        let before = list.len();
        list.retain(|c| !(c.package == package && ptr.is_none_or(|p| p == c.callback.ptr)));
        let gone = before - list.len();
        if list.is_empty() {
            self.commands.remove(name);
        }
        if let Some(n) = self.commands_by.get_mut(package) {
            *n -= gone;
        }
        gone
    }

    pub fn command(&self, name: &str) -> &[CommandRec] {
        self.commands.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn remove_package(&mut self, package: &str) {
        self.declared.retain(|_, d| {
            d.remove(package);
            !d.is_empty()
        });
        self.declared_by.remove(package);
        for id in self.by_package.get(package).cloned().unwrap_or_default() {
            self.remove_handler(id);
        }
        self.commands.retain(|_, list| {
            list.retain(|c| c.package != package);
            !list.is_empty()
        });
        self.commands_by.remove(package);
    }

    pub fn remove_on_elements(&mut self, gone: &[ElementId]) {
        for e in gone {
            for id in self.by_element.get(e).cloned().unwrap_or_default() {
                self.remove_handler(id);
            }
        }
    }
}
