use crate::acl::{Object, group_name, resource, right};
use crate::bans::Target;
use crate::rules::{duration, reason};

pub const HELP: &str = "instances | ban <rider | player:<id> | ip:<address> | range:<range>> <duration | forever> \
[reason] | unban <ban number | player:<id> | ip:<address> | range:<range>> | bans [reload] | mute <rider | \
player:<id>> <duration | forever> [reason] | unmute <rider | player:<id>> | mutes | acl list | acl show <group> | acl \
check <object> <right> | acl create <group> | acl delete <group> | acl add <group> <object> | acl remove <group> \
<object> | acl allow <group> <right> | acl deny <group> <right> | acl revoke <group> <right> | acl reload | aclrequest \
list [resource] | aclrequest allow <resource> <right | all> | aclrequest deny <resource> <right | all> (a rider: its \
number, a player id prefix of 8 hex digits or more, or its exact name, in double quotes when it has spaces; a \
duration: 30m, 12h, 7d, 2w)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Rider(String),
    Target(Target),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unban {
    Number(usize),
    Target(Target),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AclCommand {
    List,
    Show(String),
    Check { object: Object, right: String },
    Create(String),
    Delete(String),
    Add { group: String, object: Object },
    Remove { group: String, object: Object },
    Allow { group: String, right: String },
    Deny { group: String, right: String },
    Revoke { group: String, right: String },
    Reload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestCommand {
    List(Option<String>),
    Allow { resource: String, right: Option<String> },
    Deny { resource: String, right: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Instances,
    Ban { who: Who, seconds: Option<u64>, reason: String },
    Unban(Unban),
    Bans { reload: bool },
    Mute { who: Who, seconds: Option<u64>, reason: String },
    Unmute(Who),
    Mutes,
    Acl(AclCommand),
    AclRequest(RequestCommand),
}

pub const WORDS: [&str; 9] = ["instances", "ban", "unban", "bans", "mute", "unmute", "mutes", "acl", "aclrequest"];

fn next(rest: &str) -> Result<(String, &str), String> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"').ok_or("a quote is not closed")?;
        let (inside, after) = (&quoted[..end], &quoted[end + 1..]);
        if !after.is_empty() && !after.starts_with(char::is_whitespace) {
            return Err("text right after a closing quote".into());
        }
        if inside.is_empty() {
            return Err("an empty quoted name".into());
        }
        return Ok((inside.to_string(), after.trim_start()));
    }
    let (word, after) = rest.split_once(char::is_whitespace).map_or((rest, ""), |(w, a)| (w, a.trim_start()));
    if word.is_empty() {
        return Err("an argument is missing".into());
    }
    Ok((word.to_string(), after))
}

fn words<const N: usize>(rest: &str, usage: &str) -> Result<[String; N], String> {
    let mut out: [String; N] = std::array::from_fn(|_| String::new());
    let mut r = rest;
    for slot in &mut out {
        let (w, after) = next(r).map_err(|e| format!("{e}; usage: {usage}"))?;
        *slot = w;
        r = after;
    }
    if !r.is_empty() {
        return Err(format!("an argument too many; usage: {usage}"));
    }
    Ok(out)
}

fn who(text: &str) -> Result<Who, String> {
    if ["player:", "ip:", "range:"].iter().any(|p| text.starts_with(p)) {
        return Target::parse(text).map(Who::Target);
    }
    if text.contains(':') {
        return Err(format!(
            "{} is not a rider, player:<id>, ip:<address> or range:<range>",
            ssx_core::text::quoted(text)
        ));
    }
    Ok(Who::Rider(text.to_string()))
}

fn sentence(rest: &str, usage: &str, players_only: bool) -> Result<(Who, Option<u64>, String), String> {
    let (target, after) = next(rest).map_err(|e| format!("{e}; usage: {usage}"))?;
    let target = who(&target)?;
    if players_only && matches!(target, Who::Target(Target::Address(_))) {
        return Err(format!("a mute is of a player (its rider or player:<id>); usage: {usage}"));
    }
    let (time, after) = next(after).map_err(|e| format!("{e}; usage: {usage}"))?;
    let seconds = duration(&time)?;
    let why = reason(after.trim_end())?.to_string();
    Ok((target, seconds, why))
}

pub fn kick(rest: &str) -> Result<(String, String), String> {
    const USAGE: &str = "kick <rider> [reason]";
    let (rider, after) = next(rest.trim()).map_err(|e| format!("{e}; usage: {USAGE}"))?;
    Ok((rider, reason(after.trim_end())?.to_string()))
}

pub fn parse(word: &str, rest: &str) -> Option<Result<Command, String>> {
    let rest = rest.trim();
    Some(match word {
        "instances" => words::<0>(rest, "instances").map(|_| Command::Instances),
        "ban" => sentence(rest, "ban <who> <duration | forever> [reason]", false)
            .map(|(who, seconds, reason)| Command::Ban { who, seconds, reason }),
        "mute" => sentence(rest, "mute <who> <duration | forever> [reason]", true)
            .map(|(who, seconds, reason)| Command::Mute { who, seconds, reason }),
        "unban" => {
            words::<1>(rest, "unban <ban number | player:<id> | ip:<address> | range:<range>>").and_then(|[w]| {
                if w.bytes().all(|b| b.is_ascii_digit()) && !w.starts_with('0') && w.len() <= 6 {
                    return w.parse().map(|n| Command::Unban(Unban::Number(n))).map_err(|_| "not a number".into());
                }
                Target::parse(&w).map(|t| Command::Unban(Unban::Target(t)))
            })
        }
        "bans" => match rest {
            "" => Ok(Command::Bans { reload: false }),
            "reload" => Ok(Command::Bans { reload: true }),
            _ => Err("usage: bans [reload]".into()),
        },
        "unmute" => words::<1>(rest, "unmute <rider | player:<id>>").and_then(|[w]| match who(&w)? {
            Who::Target(Target::Address(_)) => Err("a mute is of a player (its rider or player:<id>)".into()),
            w => Ok(Command::Unmute(w)),
        }),
        "mutes" => words::<0>(rest, "mutes").map(|_| Command::Mutes),
        "acl" => acl(rest).map(Command::Acl),
        "aclrequest" => request(rest).map(Command::AclRequest),
        _ => return None,
    })
}

fn group(text: &str) -> Result<String, String> {
    group_name(text).map(str::to_string)
}

fn checked_right(text: &str) -> Result<String, String> {
    right(text).map(str::to_string)
}

fn acl(rest: &str) -> Result<AclCommand, String> {
    let (sub, after) = next(rest).map_err(|_| {
        "usage: acl list | show | check | create | delete | add | remove | allow | deny | revoke | reload".to_string()
    })?;
    match sub.as_str() {
        "list" => words::<0>(after, "acl list").map(|_| AclCommand::List),
        "reload" => words::<0>(after, "acl reload").map(|_| AclCommand::Reload),
        "show" => words::<1>(after, "acl show <group>").and_then(|[g]| Ok(AclCommand::Show(group(&g)?))),
        "create" => words::<1>(after, "acl create <group>").and_then(|[g]| Ok(AclCommand::Create(group(&g)?))),
        "delete" => words::<1>(after, "acl delete <group>").and_then(|[g]| Ok(AclCommand::Delete(group(&g)?))),
        "check" => words::<2>(after, "acl check <object> <right>")
            .and_then(|[o, r]| Ok(AclCommand::Check { object: Object::parse(&o)?, right: checked_right(&r)? })),
        "add" | "remove" => words::<2>(after, "acl add|remove <group> <object>").and_then(|[g, o]| {
            let (group, object) = (group(&g)?, Object::parse(&o)?);
            Ok(if sub == "add" { AclCommand::Add { group, object } } else { AclCommand::Remove { group, object } })
        }),
        "allow" | "deny" | "revoke" => words::<2>(after, "acl allow|deny|revoke <group> <right>").and_then(|[g, r]| {
            let (group, right) = (group(&g)?, checked_right(&r)?);
            Ok(match sub.as_str() {
                "allow" => AclCommand::Allow { group, right },
                "deny" => AclCommand::Deny { group, right },
                _ => AclCommand::Revoke { group, right },
            })
        }),
        _ => Err(format!("{} is not an acl command; {HELP}", ssx_core::text::quoted(&sub))),
    }
}

fn request(rest: &str) -> Result<RequestCommand, String> {
    let (sub, after) = next(rest).map_err(|_| "usage: aclrequest list [resource] | allow | deny".to_string())?;
    match sub.as_str() {
        "list" if after.is_empty() => Ok(RequestCommand::List(None)),
        "list" => words::<1>(after, "aclrequest list [resource]")
            .and_then(|[r]| Ok(RequestCommand::List(Some(resource(&r)?.to_string())))),
        "allow" | "deny" => {
            words::<2>(after, "aclrequest allow|deny <resource> <right | all>").and_then(|[r, what]| {
                let resource = resource(&r)?.to_string();
                let right = if what == "all" { None } else { Some(checked_right(&what)?) };
                Ok(if sub == "allow" {
                    RequestCommand::Allow { resource, right }
                } else {
                    RequestCommand::Deny { resource, right }
                })
            })
        }
        _ => Err(format!("{} is not an aclrequest command", ssx_core::text::quoted(&sub))),
    }
}

#[cfg(test)]
mod tests;
