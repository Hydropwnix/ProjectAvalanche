use std::sync::Arc;

use bevy::prelude::*;
use ssx_net::address::canonical;
use ssx_net::server::{Instances, KickReason, Leaving, PlayerSession, RiderLink, leave};
use ssx_net_ops::command::{Command, Unban, Who};
use ssx_net_ops::{Ban, Mute, Range, Target};
use ssx_net_proto::operator::{Expiry, span};
use ssx_net_proto::rcon::OperatorKey;

use super::store::{Operation, ban_text, unix_now};

#[derive(Resource, Clone)]
pub struct OperationRes(pub Arc<Operation>);

#[derive(Clone, Debug)]
pub enum Caller {
    Console,
    Operator(OperatorKey),
    Resource(String),
}

impl Caller {
    pub fn by(&self) -> String {
        match self {
            Self::Console => "console".into(),
            Self::Operator(k) => format!("operator {}", k.id().short()),
            Self::Resource(name) => format!("resource {name}"),
        }
    }
}

pub type Answer = (bool, Vec<String>);

pub struct Found {
    pub link: Entity,
    pub session: PlayerSession,
    pub rider: u16,
}

pub fn find_riders(world: &mut World, who: &str) -> Vec<Found> {
    let mut query = world.query_filtered::<(Entity, &PlayerSession, &RiderLink), Without<Leaving>>();
    let prefix = (who.len() >= 8 && who.chars().all(|c| c.is_ascii_hexdigit())).then(|| who.to_ascii_lowercase());
    query
        .iter(world)
        .filter(|(_, s, r)| {
            who.parse::<u16>().ok() == Some(r.rider().0)
                || prefix.as_ref().is_some_and(|p| s.id.to_hex().starts_with(p.as_str()))
                || s.name == who
        })
        .map(|(link, s, r)| Found { link, session: s.clone(), rider: r.rider().0 })
        .collect()
}

fn one_rider(world: &mut World, who: &str) -> Result<Found, String> {
    let mut found = find_riders(world, who);
    match found.len() {
        0 => Err(format!("no rider {}", ssx_core::text::quoted_max(who, 64))),
        1 => Ok(found.remove(0)),
        n => Err(format!("{} names {n} riders; use the rider number", ssx_core::text::quoted_max(who, 64))),
    }
}

pub fn end_text(until: Expiry, now: u64) -> String {
    match until {
        Expiry::Never => "forever".into(),
        Expiry::At(_) => format!("until {} (in {})", until.describe(), span(until.left(now).unwrap_or(0))),
    }
}

pub fn kick_banned(world: &mut World, operation: &Operation) -> Vec<String> {
    let now = unix_now();
    let mut query = world.query_filtered::<(Entity, &PlayerSession), Without<Leaving>>();
    let held: Vec<(Entity, String, Ban)> = query
        .iter(world)
        .filter_map(|(e, s)| operation.ban_of(Some(&s.id.0), Some(s.address), now).map(|b| (e, s.name.clone(), b)))
        .collect();
    let elapsed = world.resource::<Time<Real>>().elapsed();
    let mut names = Vec::new();
    for (link, name, ban) in held {
        let mut commands = world.commands();
        leave(&mut commands, link, KickReason::Banned(ban.until), ban_text(&ban), elapsed);
        names.push(name);
    }
    world.flush();
    names
}

pub fn run(world: &mut World, command: Command, caller: &Caller) -> Answer {
    let Some(operation) = world.get_resource::<OperationRes>().map(|o| Arc::clone(&o.0)) else {
        return (false, vec!["the operation's files are not loaded".into()]);
    };
    match command {
        Command::Instances => (true, instances(world)),
        Command::Ban { who, seconds, reason } => ban(world, &operation, who, seconds, reason, caller),
        Command::Unban(which) => unban(&operation, which),
        Command::Bans { reload: false } => (true, list_bans(&operation)),
        Command::Bans { reload: true } => match operation.reload_bans() {
            Ok((bans, mutes)) => {
                let mut lines = vec![format!("{} read: {bans} bans, {mutes} mutes", operation.bans_path().display())];
                lines.extend(kick_banned(world, &operation).into_iter().map(|n| kicked_line(&n)));
                (true, lines)
            }
            Err(e) => {
                (false, vec![format!("not reloaded (the bans in force are kept): {}", ssx_core::text::message(&e))])
            }
        },
        Command::Mute { who, seconds, reason } => mute(world, &operation, who, seconds, reason, caller),
        Command::Unmute(who) => unmute(world, &operation, who),
        Command::Mutes => (true, list_mutes(&operation)),
        Command::Acl(c) => super::acl_commands::acl(&operation, c, caller),
        Command::AclRequest(c) => super::acl_commands::request(&operation, c, caller),
    }
}

fn kicked_line(name: &str) -> String {
    format!("kicked {} (banned)", ssx_core::text::quoted_max(name, 64))
}

fn instances(world: &World) -> Vec<String> {
    let Some(instances) = world.get_resource::<Instances>() else { return vec!["no instances".into()] };
    instances
        .list
        .iter()
        .map(|s| {
            let i = &s.instance;
            format!(
                "instance {}: {} on {}, {} round {}; {} players",
                i.id(),
                i.spec().mode.name(),
                i.spec().level,
                i.state().name(),
                i.round(),
                i.len()
            )
        })
        .collect()
}

fn targets(world: &mut World, who: &Who) -> Result<(Vec<Target>, Option<String>), String> {
    match who {
        Who::Target(t) => Ok((vec![*t], None)),
        Who::Rider(text) => {
            let found = one_rider(world, text)?;
            let address = Range::address(canonical(found.session.address));
            Ok((vec![Target::Player(found.session.id.0), Target::Address(address)], Some(found.session.name)))
        }
    }
}

fn ban(world: &mut World, op: &Operation, who: Who, seconds: Option<u64>, reason: String, by: &Caller) -> Answer {
    let (targets, name) = match targets(world, &who) {
        Ok(t) => t,
        Err(e) => return (false, vec![format!("ban: {e}")]),
    };
    let now = unix_now();
    let until = Expiry::after(now, seconds);
    let made = op.change_bans(|store| {
        let mut out = Vec::new();
        for target in &targets {
            let ban = Ban { target: *target, reason: reason.clone(), by: by.by(), at: now, until };
            let replaced = store.ban(ban).map_err(|e| e.to_string())?;
            out.push((*target, replaced.is_some()));
        }
        Ok(out)
    });
    let made = match made {
        Ok(made) => made,
        Err(e) => return (false, vec![format!("not banned: {}", ssx_core::text::message(&e))]),
    };
    let mut lines: Vec<String> = made
        .iter()
        .map(|(t, replaced)| {
            let what = if *replaced { "ban replaced" } else { "banned" };
            format!("{what}: {t} {}{}", end_text(until, now), reason_part(&reason))
        })
        .collect();
    if let Some(name) = name {
        lines.insert(0, format!("rider {}:", ssx_core::text::quoted_max(&name, 64)));
    }
    lines.extend(kick_banned(world, op).into_iter().map(|n| kicked_line(&n)));
    info!(target: OPERATION_LOG, "{} by {}", lines.join("; "), by.by());
    (true, lines)
}

fn reason_part(reason: &str) -> String {
    if reason.is_empty() { String::new() } else { format!(": {}", ssx_core::text::quoted_max(reason, 200)) }
}

pub const OPERATION_LOG: &str = "avalanche_server::operation";

fn unban(op: &Operation, which: Unban) -> Answer {
    let removed = op.change_bans(|store| match &which {
        Unban::Number(n) => {
            n.checked_sub(1).and_then(|k| store.unban_at(k)).ok_or_else(|| format!("no ban number {n} (see bans)"))
        }
        Unban::Target(t) => store.unban(t).ok_or_else(|| format!("no ban of {t}")),
    });
    match removed {
        Ok(ban) => {
            let line = format!("unbanned: {}", ban.target);
            info!(target: OPERATION_LOG, "{line}");
            (true, vec![line])
        }
        Err(e) => (false, vec![format!("unban: {}", ssx_core::text::message(&e))]),
    }
}

fn list_bans(op: &Operation) -> Vec<String> {
    let now = unix_now();
    let store = op.bans();
    let mut lines = vec![format!("{} bans:", store.bans().len())];
    for (k, b) in store.bans().iter().enumerate() {
        let state = if b.until.passed(now) { " (passed)" } else { "" };
        lines.push(format!(
            "  {}. {} {}{state} by {}{}",
            k + 1,
            b.target,
            end_text(b.until, now),
            ssx_core::text::quoted_max(&b.by, 64),
            reason_part(&b.reason)
        ));
    }
    lines
}

fn mute(world: &mut World, op: &Operation, who: Who, seconds: Option<u64>, reason: String, by: &Caller) -> Answer {
    let (player, name) = match who {
        Who::Target(Target::Player(id)) => (id, None),
        Who::Target(Target::Address(_)) => return (false, vec!["mute: a mute is of a player".into()]),
        Who::Rider(text) => match one_rider(world, &text) {
            Ok(f) => (f.session.id.0, Some(f.session.name)),
            Err(e) => return (false, vec![format!("mute: {e}")]),
        },
    };
    let now = unix_now();
    let until = Expiry::after(now, seconds);
    let mute = Mute { player, reason: reason.clone(), by: by.by(), at: now, until };
    match op.change_bans(|store| store.mute(mute).map_err(|e| e.to_string())) {
        Ok(replaced) => {
            let who = name
                .map_or_else(|| Target::Player(player).to_string(), |n| ssx_core::text::quoted_max(&n, 64).to_string());
            let what = if replaced.is_some() { "mute replaced" } else { "muted" };
            let line = format!("{what}: {who} {}{}", end_text(until, now), reason_part(&reason));
            info!(target: OPERATION_LOG, "{line} by {}", by.by());
            (true, vec![line])
        }
        Err(e) => (false, vec![format!("not muted: {}", ssx_core::text::message(&e))]),
    }
}

fn unmute(world: &mut World, op: &Operation, who: Who) -> Answer {
    let player = match who {
        Who::Target(Target::Player(id)) => id,
        Who::Target(Target::Address(_)) => return (false, vec!["unmute: a mute is of a player".into()]),
        Who::Rider(text) => match one_rider(world, &text) {
            Ok(f) => f.session.id.0,
            Err(e) => return (false, vec![format!("unmute: {e}")]),
        },
    };
    match op.change_bans(|store| store.unmute(&player).ok_or_else(|| "no mute of that player".to_string())) {
        Ok(_) => {
            let line = format!("unmuted: {}", Target::Player(player));
            info!(target: OPERATION_LOG, "{line}");
            (true, vec![line])
        }
        Err(e) => (false, vec![format!("unmute: {}", ssx_core::text::message(&e))]),
    }
}

fn list_mutes(op: &Operation) -> Vec<String> {
    let now = unix_now();
    let store = op.bans();
    let mut lines = vec![format!("{} mutes:", store.mutes().len())];
    for m in store.mutes() {
        let state = if m.until.passed(now) { " (passed)" } else { "" };
        lines.push(format!(
            "  {} {}{state} by {}{}",
            Target::Player(m.player),
            end_text(m.until, now),
            ssx_core::text::quoted_max(&m.by, 64),
            reason_part(&m.reason)
        ));
    }
    lines
}
