use ssx_net_ops::acl::{Access, Object, RequestState, Source, Subject};
use ssx_net_ops::command::{AclCommand, RequestCommand};

use super::commands::{Answer, Caller, OPERATION_LOG};
use super::store::{Operation, unix_now};

fn changed(op: &Operation, line: String, change: impl FnOnce(&mut ssx_net_ops::Acl) -> Result<(), String>) -> Answer {
    match op.change_acl(change) {
        Ok(()) => {
            bevy::log::info!(target: OPERATION_LOG, "acl: {line}");
            (true, vec![line])
        }
        Err(e) => (false, vec![format!("acl: {}", ssx_core::text::message(&e))]),
    }
}

pub fn acl(op: &Operation, command: AclCommand, caller: &Caller) -> Answer {
    let by = caller.by();
    match command {
        AclCommand::List => {
            let acl = op.acl();
            let mut lines = vec![format!("{} groups:", acl.groups().len())];
            for g in acl.groups() {
                lines.push(format!(
                    "  {}: {} objects, {} allowed, {} denied",
                    g.name,
                    g.objects.len(),
                    g.allow.len(),
                    g.deny.len()
                ));
            }
            lines.push(format!(
                "the remote console is {}",
                if acl.has_operators() { "on (operator keys in the ACL)" } else { "off (no operator key in the ACL)" }
            ));
            (true, lines)
        }
        AclCommand::Show(name) => {
            let acl = op.acl();
            let Some(g) = acl.group(&name) else { return (false, vec![format!("acl: no group {name}")]) };
            let mut lines = vec![format!("group {}:", g.name)];
            lines.extend(g.objects.iter().map(|o| format!("  object {o}")));
            lines.extend(g.allow.iter().map(|r| format!("  allow {r}")));
            lines.extend(g.deny.iter().map(|r| format!("  deny {r}")));
            (true, lines)
        }
        AclCommand::Check { object, right } => {
            let acl = op.acl();
            let verdict = match &object {
                Object::Operator(k) => acl.check(Subject::Operator(k), &right),
                Object::Player(id) => acl.check(Subject::Player { id, ip: None }, &right),
                Object::Resource(name) => acl.check(Subject::Resource(name), &right),
                Object::Players | Object::Resources | Object::Ip(_) => {
                    return (false, vec!["acl check: name one operator.<key>, player.<id> or resource.<name>".into()]);
                }
            };
            let what = if verdict.allowed { "allowed" } else { "denied" };
            let why = match (&verdict.source, &verdict.level) {
                (Source::Group(g), Some(level)) => format!("by group {g} at {level}"),
                (Source::Request, Some(level)) => format!("by the resource's decided request at {level}"),
                (Source::Default, Some(level)) => format!("by the resources' defaults at {level}"),
                _ => "nothing grants it".to_string(),
            };
            (true, vec![format!("{object} {right}: {what} ({why})")])
        }
        AclCommand::Create(g) => changed(op, format!("group {g} made by {by}"), |a| a.create_group(&g).map_err(err)),
        AclCommand::Delete(g) => {
            changed(op, format!("group {g} deleted by {by}"), |a| a.delete_group(&g).map(drop).map_err(err))
        }
        AclCommand::Add { group, object } => {
            let line = format!("{object} added to {group} by {by}");
            changed(op, line, |a| a.add_object(&group, object).map_err(err))
        }
        AclCommand::Remove { group, object } => {
            let line = format!("{object} removed from {group} by {by}");
            changed(op, line, |a| a.remove_object(&group, &object).map_err(err))
        }
        AclCommand::Allow { group, right } => {
            let line = format!("{group} allows {right} (by {by})");
            changed(op, line, |a| a.set_right(&group, &right, Access::Allow).map_err(err))
        }
        AclCommand::Deny { group, right } => {
            let line = format!("{group} denies {right} (by {by})");
            changed(op, line, |a| a.set_right(&group, &right, Access::Deny).map_err(err))
        }
        AclCommand::Revoke { group, right } => {
            let line = format!("{group} no longer names {right} (by {by})");
            changed(op, line, |a| a.revoke(&group, &right).map_err(err))
        }
        AclCommand::Reload => match op.reload_acl() {
            Ok((groups, requests)) => {
                let line = format!("{} read: {groups} groups, {requests} requests", op.acl_path().display());
                bevy::log::info!(target: OPERATION_LOG, "acl: {line} (by {by})");
                (true, vec![line])
            }
            Err(e) => {
                (false, vec![format!("not reloaded (the ACL in force is kept): {}", ssx_core::text::message(&e))])
            }
        },
    }
}

fn err(e: ssx_net_ops::acl::AclError) -> String {
    e.to_string()
}

pub fn request(op: &Operation, command: RequestCommand, caller: &Caller) -> Answer {
    let by = caller.by();
    let decide = |resource: String, right: Option<String>, allow: bool| {
        let verb = if allow { "allowed" } else { "denied" };
        let what = right.clone().unwrap_or_else(|| "every pending request".into());
        match op.change_acl(|a| a.decide(&resource, right.as_deref(), allow, &by, unix_now()).map_err(err)) {
            Ok(n) => {
                let line = format!("{resource}: {what} {verb} by {by} ({n} decided)");
                bevy::log::info!(target: OPERATION_LOG, "aclrequest: {line}");
                (true, vec![line])
            }
            Err(e) => (false, vec![format!("aclrequest: {}", ssx_core::text::message(&e))]),
        }
    };
    match command {
        RequestCommand::List(resource) => {
            let acl = op.acl();
            let shown: Vec<_> =
                acl.requests().iter().filter(|q| resource.as_ref().is_none_or(|r| q.resource == *r)).collect();
            let mut lines = vec![format!("{} requests:", shown.len())];
            for q in shown {
                let decided = match q.state {
                    RequestState::Pending => String::new(),
                    _ => format!(
                        " by {} at {}",
                        ssx_core::text::quoted_max(&q.by, 64),
                        ssx_net_proto::operator::utc(q.at)
                    ),
                };
                lines.push(format!(
                    "  {} {}: {} (asked {}){decided}",
                    q.resource,
                    q.right,
                    q.state.name(),
                    ssx_net_proto::operator::utc(q.asked)
                ));
            }
            (true, lines)
        }
        RequestCommand::Allow { resource, right } => decide(resource, right, true),
        RequestCommand::Deny { resource, right } => decide(resource, right, false),
    }
}
