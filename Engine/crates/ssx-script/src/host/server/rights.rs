use std::sync::{Arc, OnceLock};

use super::super::elements::ClientId;
use super::super::world::World;
use super::super::{ScriptHost, Side};

pub const PRIVILEGED: [&str; 7] =
    ["kickPlayer", "banPlayer", "mutePlayer", "startResource", "stopResource", "restartResource", "executeCommand"];

pub const MAX_COMMANDS_PER_FRAME: usize = 16;

pub const MAX_SECONDS: i64 = 100 * 365 * 86_400;
pub const MAX_REASON: usize = 200;
pub const MAX_LINE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder<'a> {
    Package(&'a str),
    Player(ClientId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub why: String,
}

pub trait Rights: Send + Sync {
    fn check(&self, holder: Holder<'_>, right: &str) -> Result<Decision, String>;
    fn generation(&self) -> u64;
    fn resolve(&self, package: &str, rights: &[String]) -> Result<Vec<Decision>, String> {
        rights.iter().map(|r| self.check(Holder::Package(package), r)).collect()
    }
    fn reason(&self, text: &str) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceAction {
    Start,
    Stop,
    Restart,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationOrder {
    Kick { client: ClientId, reason: String },
    Ban { client: ClientId, seconds: Option<u64>, reason: String },
    Mute { client: ClientId, seconds: Option<u64>, reason: String },
    Resource { action: ResourceAction, name: String },
    Command { line: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Privileged {
    pub package: String,
    pub right: String,
    pub order: OperationOrder,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RightsCounts {
    pub checks: u64,
    pub refused: u64,
    pub ordered: u64,
    pub calls_refused: u64,
    pub resolved: u64,
}

pub fn call_rights() -> &'static [String] {
    static RIGHTS: OnceLock<Vec<String>> = OnceLock::new();
    RIGHTS.get_or_init(|| crate::api::server_calls().iter().map(|s| format!("function.{}", s.name)).collect())
}

#[derive(Clone, Debug)]
pub struct CallRights {
    generation: u64,
    refused: Vec<Option<String>>,
}

pub(crate) fn resolve(w: &mut World, package: &str) {
    let Some(rights) = w.server.rights.clone() else { return };
    if !w.packages.contains_key(package) {
        return;
    }
    let generation = rights.generation();
    let names = call_rights();
    let refused = match rights.resolve(package, names) {
        Ok(d) if d.len() == names.len() => d.into_iter().map(|d| (!d.allowed).then_some(d.why)).collect(),
        Ok(d) => vec![Some(format!("the rights gave {} decisions for {} calls", d.len(), names.len())); names.len()],
        Err(e) => vec![Some(format!("its rights cannot be checked: {e}")); names.len()],
    };
    w.server.rights_counts.resolved += 1;
    if let Some(p) = w.packages.get_mut(package) {
        p.calls = Some(CallRights { generation, refused });
    }
}

pub(crate) fn refresh(w: &mut World) {
    let Some(generation) = w.server.rights.as_ref().map(|r| r.generation()) else { return };
    let stale: Vec<String> = w
        .packages
        .iter()
        .filter(|(_, p)| p.calls.as_ref().is_none_or(|c| c.generation != generation))
        .map(|(n, _)| n.clone())
        .collect();
    for name in stale {
        resolve(w, &name);
    }
}

fn decided(w: &mut World, package: &str, index: usize) -> Option<String> {
    let Some(generation) = w.server.rights.as_ref().map(|r| r.generation()) else {
        let name = crate::api::server_calls().get(index).map_or("", |s| s.name);
        return PRIVILEGED.contains(&name).then(|| "this host has no ACL, so no package holds a privilege".to_string());
    };
    let fresh =
        |w: &World| w.packages.get(package).and_then(|p| p.calls.as_ref()).is_some_and(|c| c.generation == generation);
    if !fresh(w) {
        resolve(w, package);
    }
    match w.packages.get(package).and_then(|p| p.calls.as_ref()) {
        Some(c) => c.refused.get(index).cloned().unwrap_or_else(|| Some("not a call of the server API".into())),
        None => Some(format!("{package} is not running")),
    }
}

pub(crate) fn call_allowed(w: &mut World, package: &str, index: usize) -> Result<(), String> {
    match decided(w, package, index) {
        None => Ok(()),
        Some(why) => {
            w.server.rights_counts.calls_refused += 1;
            let right = call_rights().get(index).map_or("", String::as_str);
            Err(format!("access denied: {package} does not hold {right}: {why}"))
        }
    }
}

pub(crate) fn require(w: &mut World, package: &str, right: &str) -> Result<(), String> {
    w.server.rights_counts.checks += 1;
    let r = match (call_rights().iter().position(|r| r == right), w.server.rights.as_ref()) {
        (Some(index), _) => match decided(w, package, index) {
            None => Ok(()),
            Some(why) => Err(format!("{package} does not hold {right}: {why}")),
        },
        (None, None) => {
            Err(format!("{package} does not hold {right}: this host has no ACL, so no package holds a privilege"))
        }
        (None, Some(rights)) => match rights.check(Holder::Package(package), right) {
            Ok(Decision { allowed: true, .. }) => Ok(()),
            Ok(Decision { allowed: false, why }) => Err(format!("{package} does not hold {right}: {why}")),
            Err(e) => Err(format!("{right} cannot be checked: {e}")),
        },
    };
    if r.is_err() {
        w.server.rights_counts.refused += 1;
    }
    r
}

pub(crate) fn check_reason(w: &World, text: &str) -> Result<(), String> {
    match w.server.rights.as_ref() {
        Some(r) => r.reason(text),
        None => Err("this host has no ACL".into()),
    }
}

pub(crate) fn holds(w: &mut World, holder: Holder<'_>, right: &str) -> Result<bool, String> {
    if let (Holder::Package(name), Some(index)) = (holder, call_rights().iter().position(|r| r == right))
        && w.packages.contains_key(name)
    {
        return Ok(decided(w, name, index).is_none());
    }
    match w.server.rights.as_ref() {
        Some(r) => r.check(holder, right).map(|d| d.allowed),
        None => Ok(false),
    }
}

impl ScriptHost {
    pub fn set_rights(&self, rights: Arc<dyn Rights>) {
        if self.shared.side == Side::Server {
            let mut w = self.shared.lock();
            w.server.rights = Some(rights);
            for p in w.packages.values_mut() {
                p.calls = None;
            }
            refresh(&mut w);
        }
    }

    pub fn rights_counts(&self) -> RightsCounts {
        self.shared.lock().server.rights_counts
    }

    pub fn take_spent(&self) -> Vec<(String, std::time::Duration)> {
        std::mem::take(&mut self.shared.lock().server.spent).into_iter().collect()
    }
}

#[cfg(test)]
mod tests;
