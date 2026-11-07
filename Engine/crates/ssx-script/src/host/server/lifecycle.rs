use super::super::call::{Dispatch, dispatch};
use super::super::elements::{ClientId, ROOT};
use super::super::outbox::{Body, Destination, Message};
use super::super::world::World;
use super::super::{ScriptHost, Side};
use crate::value::NetValue;

pub const STATES: &[&str] = &["loaded", "starting", "running", "stopping", "stopped"];

pub const CANCEL_START_RIGHT: &str = "function.stopResource";

pub(crate) struct PreStart {
    pub frame: usize,
    pub cancelled_by: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LifecycleCounts {
    pub starts_cancelled: u64,
    pub cancels_refused: u64,
    pub acknowledgements: u64,
    pub acknowledgements_refused: u64,
}

pub(crate) fn may_cancel(w: &mut World, package: &str) -> bool {
    let depth = w.frames.len();
    let Some(pre) = w.server.pre_start.as_ref() else { return true };
    if pre.frame + 1 != depth {
        return true;
    }
    if let Err(e) = super::rights::require(w, package, CANCEL_START_RIGHT) {
        w.server.lifecycle.cancels_refused += 1;
        if let Some(p) = w.packages.get(package) {
            p.vm.note(format!("cancelEvent: refused: keeping a resource from starting needs a right: {e}"));
        }
        return false;
    }
    if let Some(pre) = w.server.pre_start.as_mut() {
        pre.cancelled_by.get_or_insert_with(|| package.to_string());
    }
    true
}

impl ScriptHost {
    pub fn lifecycle_counts(&self) -> LifecycleCounts {
        self.shared.lock().server.lifecycle
    }

    pub(crate) fn pre_start(&self, name: &str) -> Result<(), String> {
        if self.shared.side != Side::Server {
            return Ok(());
        }
        let outer = {
            let mut w = self.shared.lock();
            let frame = w.frames.len();
            w.server.pre_start.replace(PreStart { frame, cancelled_by: None })
        };
        let args = [NetValue::String(name.as_bytes().to_vec())];
        let d = Dispatch { name: "onResourcePreStart", source: ROOT, args: &args, only: None, client: None };
        let r = dispatch(&self.shared, &d);
        let by = {
            let mut w = self.shared.lock();
            let mine = std::mem::replace(&mut w.server.pre_start, outer);
            mine.and_then(|p| p.cancelled_by)
        };
        match (r, by) {
            (Err(e), _) => Err(format!("onResourcePreStart: {e}")),
            (Ok(_), Some(by)) => {
                self.shared.lock().server.lifecycle.starts_cancelled += 1;
                Err(format!("{by} cancelled it (onResourcePreStart)"))
            }
            (Ok(_), None) => Ok(()),
        }
    }

    pub(crate) fn state_change(&self, name: &str, old: &'static str, new: &'static str) {
        if self.shared.side != Side::Server {
            return;
        }
        let s = |t: &str| NetValue::String(t.as_bytes().to_vec());
        let args = [s(name), s(old), s(new)];
        let d = Dispatch { name: "onResourceStateChange", source: ROOT, args: &args, only: None, client: None };
        let _ = dispatch(&self.shared, &d);
    }

    pub fn acknowledge_start(&self, package: &str) -> Result<(), String> {
        let mut w = self.shared.lock();
        if self.shared.side != Side::Client || w.running(package).is_none() {
            return Err(format!("{package} does not run on this client"));
        }
        let m = Message { package: Some(package.to_string()), to: Destination::Server, body: Body::Started };
        w.outbox.push(m)
    }

    pub(crate) fn client_started(&self, client: ClientId, package: Option<&str>) -> Result<(), String> {
        let r = self.take_acknowledgement(client, package);
        let player = match r {
            Ok(player) => player,
            Err(e) => {
                self.shared.lock().server.lifecycle.acknowledgements_refused += 1;
                return Err(e);
            }
        };
        let args = [NetValue::String(package.unwrap_or_default().as_bytes().to_vec())];
        self.raise_engine("onPlayerResourceStart", player, &args);
        Ok(())
    }

    fn take_acknowledgement(&self, client: ClientId, package: Option<&str>) -> Result<crate::value::ElementId, String> {
        let package = package.ok_or("an acknowledgement without its package")?;
        let mut w = self.shared.lock();
        let player = w.player_of(client).ok_or("an acknowledgement from a client without its player")?;
        let p = w
            .packages
            .get_mut(package)
            .filter(|p| p.state == super::super::world::PkgState::Running)
            .ok_or_else(|| format!("an acknowledgement of {package}, which does not run here"))?;
        if !p.client_scripts {
            return Err(format!("an acknowledgement of {package}, which has no client scripts"));
        }
        if !p.acknowledged.insert(client) {
            return Err(format!("a repeated acknowledgement of {package}"));
        }
        w.server.lifecycle.acknowledgements += 1;
        Ok(player)
    }
}
