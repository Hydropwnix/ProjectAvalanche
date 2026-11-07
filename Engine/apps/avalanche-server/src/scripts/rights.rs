use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, PoisonError, RwLock};

use bevy::prelude::*;
use ssx_net::address::canonical;
use ssx_net::server::{KickReason, Leaving, PlayerSession, leave};
use ssx_net_ops::acl::{RequestState, Source, Subject, Verdict};
use ssx_net_ops::command::Who;
use ssx_net_ops::{Range, Target};
use ssx_script::host::server::rights::{self, Decision, Holder, OperationOrder, Privileged, ResourceAction, Rights};

use crate::operation::commands::{Caller, OPERATION_LOG, OperationRes};
use crate::operation::store::{Operation, unix_now};
use crate::resources::{PermissionRequests, ResourceCommand, ServerResources};

const _: () = assert!(rights::MAX_SECONDS as u64 == ssx_net_ops::rules::MAX_DURATION_SECS);
const _: () = assert!(rights::MAX_REASON == ssx_net_ops::rules::MAX_REASON_BYTES);
const _: () = assert!(ssx_content::delivery::MAX_NAME == ssx_net_ops::acl::MAX_RESOURCE_BYTES);

#[derive(Resource, Clone, Default)]
pub struct Identities(Arc<RwLock<Known>>);

type Known = BTreeMap<u32, ([u8; 32], IpAddr)>;

impl Identities {
    pub fn insert(&self, client: u32, id: [u8; 32], address: IpAddr) {
        self.0.write().unwrap_or_else(PoisonError::into_inner).insert(client, (id, canonical(address)));
    }

    pub fn forget(&self, client: u32) {
        self.0.write().unwrap_or_else(PoisonError::into_inner).remove(&client);
    }

    fn get(&self, client: u32) -> Option<([u8; 32], IpAddr)> {
        self.0.read().unwrap_or_else(PoisonError::into_inner).get(&client).copied()
    }
}

pub struct OperationRights {
    pub operation: Arc<Operation>,
    pub players: Identities,
}

impl Rights for OperationRights {
    fn check(&self, holder: Holder<'_>, right: &str) -> Result<Decision, String> {
        ssx_net_ops::acl::right(right)?;
        let acl = self.operation.acl();
        match holder {
            Holder::Package(name) => {
                ssx_net_ops::acl::resource(name)?;
                Ok(resource_decision(&acl, name, right))
            }
            Holder::Player(c) => {
                let (id, ip) = self.players.get(c.0).ok_or("the player is not connected")?;
                let verdict = acl.check(Subject::Player { id: &id, ip: Some(ip) }, right);
                Ok(Decision { allowed: verdict.allowed, why: decided(&verdict) })
            }
        }
    }

    fn generation(&self) -> u64 {
        self.operation.acl_generation()
    }

    fn resolve(&self, package: &str, rights: &[String]) -> Result<Vec<Decision>, String> {
        ssx_net_ops::acl::resource(package)?;
        for right in rights {
            ssx_net_ops::acl::right(right)?;
        }
        let acl = self.operation.acl();
        let pending: std::collections::HashSet<&str> = acl
            .requests()
            .iter()
            .filter(|q| q.resource == package && q.state == RequestState::Pending)
            .map(|q| q.right.as_str())
            .collect();
        let verdicts = acl.check_all(Subject::Resource(package), rights);
        Ok(rights
            .iter()
            .zip(verdicts)
            .map(|(right, verdict)| {
                let waiting = !verdict.allowed && pending.contains(right.as_str());
                Decision { allowed: verdict.allowed, why: why_resource(&verdict, waiting.then_some((package, right))) }
            })
            .collect())
    }

    fn reason(&self, text: &str) -> Result<(), String> {
        ssx_net_ops::rules::reason(text).map(|_| ())
    }
}

fn resource_decision(acl: &ssx_net_ops::Acl, name: &str, right: &str) -> Decision {
    let verdict = acl.check(Subject::Resource(name), right);
    let waiting = !verdict.allowed
        && acl.requests().iter().any(|q| q.resource == name && q.right == right && q.state == RequestState::Pending);
    Decision { allowed: verdict.allowed, why: why_resource(&verdict, waiting.then_some((name, right))) }
}

fn why_resource(verdict: &Verdict, pending: Option<(&str, &str)>) -> String {
    let mut why = decided(verdict);
    if let Some((name, right)) = pending {
        why.push_str(&format!("; its request is pending until an operator's `aclrequest allow {name} {right}`"));
    }
    why
}

fn decided(v: &Verdict) -> String {
    let level = v.level.as_deref().unwrap_or_default();
    let (one, many, done) = if v.allowed { ("allows", "allow", "allowed") } else { ("denies", "deny", "denied") };
    match &v.source {
        Source::Console => "the console holds every right".into(),
        Source::Group(g) => format!("the group {g} {one} {level}"),
        Source::Request => format!("the operator {done} its request for {level}"),
        Source::Default => format!("the built-in defaults {many} {level} to every resource"),
        Source::Nothing => "nothing in the ACL grants it".into(),
    }
}

#[derive(Resource, Default)]
pub struct OperationOrders {
    pub orders: Vec<Privileged>,
    pub forget: Vec<u32>,
}

#[derive(Resource, Default, Debug)]
pub struct OrderStats {
    pub applied: u64,
    pub refused: u64,
    pub requests_recorded: u64,
}

pub fn apply_orders(world: &mut World) {
    let (orders, forget) = {
        let mut o = world.resource_mut::<OperationOrders>();
        (std::mem::take(&mut o.orders), std::mem::take(&mut o.forget))
    };
    let operation = world.get_resource::<OperationRes>().map(|o| Arc::clone(&o.0));
    for p in orders {
        let outcome = match &operation {
            Some(op) => held_again(op, &p).and_then(|()| apply(world, &p)),
            None => Err("the operation's files are not loaded".into()),
        };
        let what = describe(&p.order);
        let mut stats = world.resource_mut::<OrderStats>();
        match outcome {
            Ok(lines) => {
                stats.applied += 1;
                info!(target: OPERATION_LOG, "resource {}: {what}: done{}", p.package, joined(&lines));
            }
            Err(e) => {
                stats.refused += 1;
                warn!(target: OPERATION_LOG, "resource {}: {what}: not done: {}", p.package, ssx_core::text::shown(&e));
            }
        }
    }
    if let Some(ids) = world.get_resource::<Identities>() {
        forget.into_iter().for_each(|c| ids.forget(c));
    }
}

fn joined(lines: &[String]) -> String {
    if lines.is_empty() { String::new() } else { format!(" ({})", lines.join("; ")) }
}

fn describe(order: &OperationOrder) -> String {
    let shown = |s: &str| ssx_core::text::quoted_max(s, 200).to_string();
    match order {
        OperationOrder::Kick { client, .. } => format!("kickPlayer of client {}", client.0),
        OperationOrder::Ban { client, .. } => format!("banPlayer of client {}", client.0),
        OperationOrder::Mute { client, .. } => format!("mutePlayer of client {}", client.0),
        OperationOrder::Resource { action, name } => format!("{action:?} {name}").to_lowercase(),
        OperationOrder::Command { line } => format!("executeCommand {}", shown(line)),
    }
}

fn held_again(op: &Operation, p: &Privileged) -> Result<(), String> {
    let acl = op.acl();
    let mut rights = vec![p.right.clone()];
    if let OperationOrder::Command { line } = &p.order {
        rights.push(format!("command.{}", line.split_whitespace().next().unwrap_or_default()));
    }
    for right in rights {
        let v = acl.check(Subject::Resource(&p.package), &right);
        if !v.allowed {
            return Err(format!("{} no longer holds {right}: {}", p.package, decided(&v)));
        }
    }
    Ok(())
}

fn link_of(world: &mut World, client: u32) -> Option<Entity> {
    let links = world.get_resource::<super::world::Pending>()?.link(client)?;
    world.get::<PlayerSession>(links)?;
    world.get::<Leaving>(links).is_none().then_some(links)
}

fn identity(world: &World, client: u32) -> Result<([u8; 32], IpAddr), String> {
    let found = world.get_resource::<Identities>().and_then(|i| i.get(client));
    found.ok_or_else(|| "the player left before the order".into())
}

fn apply(world: &mut World, p: &Privileged) -> Result<Vec<String>, String> {
    let caller = Caller::Resource(p.package.clone());
    match &p.order {
        OperationOrder::Kick { client, reason } => {
            let link = link_of(world, client.0).ok_or("the player left before the order")?;
            let text = if reason.is_empty() { format!("kicked by the server ({})", p.package) } else { reason.clone() };
            let now = world.resource::<Time<Real>>().elapsed();
            leave(&mut world.commands(), link, KickReason::Operator, &text, now);
            world.flush();
            Ok(vec![format!("client {} kicked", client.0)])
        }
        OperationOrder::Ban { client, seconds, reason } => {
            let (id, ip) = identity(world, client.0)?;
            let mut lines = Vec::new();
            for target in [Target::Player(id), Target::Address(Range::address(ip))] {
                let ban =
                    ssx_net_ops::Command::Ban { who: Who::Target(target), seconds: *seconds, reason: reason.clone() };
                let (ok, said) = crate::operation::commands::run(world, ban, &caller);
                if !ok {
                    return Err(said.join("; "));
                }
                lines.extend(said);
            }
            Ok(lines)
        }
        OperationOrder::Mute { client, seconds, reason } => {
            let (id, _) = identity(world, client.0)?;
            let mute = ssx_net_ops::Command::Mute {
                who: Who::Target(Target::Player(id)),
                seconds: *seconds,
                reason: reason.clone(),
            };
            match crate::operation::commands::run(world, mute, &caller) {
                (true, said) => Ok(said),
                (false, said) => Err(said.join("; ")),
            }
        }
        OperationOrder::Resource { action, name } => {
            let word = match action {
                ResourceAction::Start => "start",
                ResourceAction::Stop => "stop",
                ResourceAction::Restart => "restart",
            };
            let command = ResourceCommand::parse(word, name).ok_or("not a resource command")??;
            let mut resources = world.get_resource_mut::<ServerResources>().ok_or("this server runs no resources")?;
            resources.push(command).map(|n| vec![format!("queued ({n} waiting)")])
        }
        OperationOrder::Command { line } => {
            let command = match crate::console::parse(line) {
                Some(Ok(c)) => c,
                Some(Err(e)) => return Err(e),
                None => return Err("an empty command line".into()),
            };
            let (ok, lines) = crate::systems::run_command_for(world, command, &caller);
            if ok { Ok(lines) } else { Err(lines.join("; ")) }
        }
    }
}

pub fn requests_at_start(running: &[crate::resources::Started]) -> PermissionRequests {
    let by_resource = running
        .iter()
        .filter(|s| !s.loaded.manifest.permissions.request.is_empty())
        .map(|s| {
            let version = s.loaded.manifest.resource.version;
            (s.loaded.name.clone(), (version, s.loaded.manifest.permissions.request.clone()))
        })
        .collect();
    PermissionRequests { by_resource, generation: 1 }
}

pub fn record_requests(
    requests: Res<PermissionRequests>,
    operation: Option<Res<OperationRes>>,
    mut seen: Local<Option<u64>>,
    mut stats: ResMut<OrderStats>,
) {
    if *seen == Some(requests.generation) {
        return;
    }
    *seen = Some(requests.generation);
    let Some(operation) = operation else { return };
    let missing: Vec<(String, String)> = {
        let acl = operation.0.acl();
        requests
            .by_resource
            .iter()
            .flat_map(|(name, (_, rights))| rights.iter().map(move |r| (name.clone(), r.clone())))
            .filter(|(name, right)| !acl.requests().iter().any(|q| q.resource == *name && q.right == *right))
            .collect()
    };
    let (recordable, outside): (Vec<_>, Vec<_>) = missing
        .into_iter()
        .partition(|(name, right)| ssx_net_ops::acl::resource(name).is_ok() && ssx_net_ops::acl::right(right).is_ok());
    for (name, right) in outside {
        let why = ssx_net_ops::acl::right(&right).err().unwrap_or_else(|| format!("{name} is not a resource name"));
        warn!(target: OPERATION_LOG, "{name}'s permission request {}: not recorded: {why}", ssx_core::text::quoted_max(&right, 128));
    }
    if recordable.is_empty() {
        return;
    }
    let now = unix_now();
    let made = operation.0.change_acl(|acl| {
        let mut recorded = Vec::new();
        let mut refused = Vec::new();
        for (name, right) in &recordable {
            match acl.request(name, right, now) {
                Ok(_) => recorded.push(format!("{name}: {right}")),
                Err(e) => refused.push(format!("{name}: {right}: {e}")),
            }
        }
        if recorded.is_empty() { Err(refused.join("; ")) } else { Ok((recorded, refused)) }
    });
    match made {
        Ok((recorded, refused)) => {
            stats.requests_recorded += recorded.len() as u64;
            info!(target: OPERATION_LOG, "permission requests pending (`aclrequest list`): {}", recorded.join(", "));
            for r in refused {
                warn!(target: OPERATION_LOG, "a permission request not recorded: {r}");
            }
        }
        Err(e) => warn!(target: OPERATION_LOG, "the permission requests not recorded: {}", ssx_core::text::shown(&e)),
    }
}

#[cfg(test)]
mod tests;
