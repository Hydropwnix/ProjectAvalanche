use serde::Deserialize;
use toml::Spanned;

use super::{
    Acl, Group, MAX_GROUPS, MAX_OBJECTS, MAX_REQUESTS, MAX_RIGHTS, Object, Request, RequestState, group_name, resource,
    right,
};
use crate::rules::{FileError, by, toml_string};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    group: Option<Spanned<Vec<Spanned<RawGroup>>>>,
    request: Option<Spanned<Vec<Spanned<RawRequest>>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGroup {
    name: Spanned<String>,
    objects: Spanned<Vec<Spanned<String>>>,
    allow: Spanned<Vec<Spanned<String>>>,
    deny: Spanned<Vec<Spanned<String>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequest {
    resource: Spanned<String>,
    right: Spanned<String>,
    state: Spanned<String>,
    asked: Spanned<i64>,
    by: Spanned<String>,
    at: Spanned<i64>,
}

fn rights(text: &str, list: Spanned<Vec<Spanned<String>>>, seen: &mut Vec<String>) -> Result<Vec<String>, FileError> {
    let mut out = Vec::new();
    for r in list.into_inner() {
        right(r.get_ref()).map_err(|m| FileError::at(text, r.span().start, m))?;
        if seen.contains(r.get_ref()) {
            return Err(FileError::at(text, r.span().start, "a right twice in one group (allowed or denied)"));
        }
        seen.push(r.get_ref().clone());
        out.push(r.into_inner());
    }
    Ok(out)
}

pub(super) fn parse(text: &str) -> Result<Acl, FileError> {
    let raw: RawFile = toml::from_str(text)
        .map_err(|e| FileError::at(text, e.span().map_or(0, |s| s.start), e.message().to_string()))?;
    let mut acl = Acl::default();
    if let Some(groups) = raw.group {
        if groups.get_ref().len() > MAX_GROUPS {
            return Err(FileError::at(text, groups.span().start, format!("more than {MAX_GROUPS} groups")));
        }
        for g in groups.into_inner() {
            let g = g.into_inner();
            let name = group_name(g.name.get_ref()).map_err(|m| FileError::at(text, g.name.span().start, m))?;
            if acl.group(name).is_some() {
                return Err(FileError::at(text, g.name.span().start, "a group's name twice (case does not count)"));
            }
            let name = name.to_string();
            if g.objects.get_ref().len() > MAX_OBJECTS {
                return Err(FileError::at(text, g.objects.span().start, format!("more than {MAX_OBJECTS} objects")));
            }
            let mut objects: Vec<Object> = Vec::new();
            for o in g.objects.into_inner() {
                let object = Object::parse(o.get_ref()).map_err(|m| FileError::at(text, o.span().start, m))?;
                if objects.contains(&object) {
                    return Err(FileError::at(text, o.span().start, "an object twice in one group"));
                }
                objects.push(object);
            }
            if g.allow.get_ref().len() + g.deny.get_ref().len() > MAX_RIGHTS {
                return Err(FileError::at(text, g.allow.span().start, format!("more than {MAX_RIGHTS} rights")));
            }
            let mut seen = Vec::new();
            let allow = rights(text, g.allow, &mut seen)?;
            let deny = rights(text, g.deny, &mut seen)?;
            acl.groups.push(Group { name, objects, allow, deny });
        }
    }
    if let Some(requests) = raw.request {
        if requests.get_ref().len() > MAX_REQUESTS {
            return Err(FileError::at(text, requests.span().start, format!("more than {MAX_REQUESTS} requests")));
        }
        for q in requests.into_inner() {
            let span = q.span();
            let q = q.into_inner();
            let at = |s: &Spanned<String>, m: String| FileError::at(text, s.span().start, m);
            resource(q.resource.get_ref()).map_err(|m| at(&q.resource, m))?;
            right(q.right.get_ref()).map_err(|m| at(&q.right, m))?;
            let state = match q.state.get_ref().as_str() {
                "pending" => RequestState::Pending,
                "allowed" => RequestState::Allowed,
                "denied" => RequestState::Denied,
                _ => return Err(at(&q.state, "state: pending, allowed or denied".into())),
            };
            let asked = u64::try_from(*q.asked.get_ref())
                .map_err(|_| FileError::at(text, q.asked.span().start, "asked: a Unix time, 0 or more"))?;
            let when = u64::try_from(*q.at.get_ref())
                .map_err(|_| FileError::at(text, q.at.span().start, "at: a Unix time, 0 or more"))?;
            match state {
                RequestState::Pending if !q.by.get_ref().is_empty() || when != 0 => {
                    return Err(at(&q.by, "a pending request has no decision: by = \"\", at = 0".into()));
                }
                RequestState::Pending => {}
                _ => {
                    by(q.by.get_ref()).map_err(|m| at(&q.by, m))?;
                    if when == 0 {
                        return Err(FileError::at(text, q.at.span().start, "at: when it was decided"));
                    }
                }
            }
            if acl.requests.iter().any(|o| o.resource == *q.resource.get_ref() && o.right == *q.right.get_ref()) {
                return Err(FileError::at(text, span.start, "a resource's request for a right twice"));
            }
            acl.requests.push(Request {
                resource: q.resource.into_inner(),
                right: q.right.into_inner(),
                state,
                asked,
                by: q.by.into_inner(),
                at: when,
            });
        }
    }
    Ok(acl)
}

const HEADER: &str = "\
# Project Avalanche: the server's ACL (written by the server: the console's and RCON's `acl` and `aclrequest`). Edit
# it while the server is stopped, or run `acl reload` after an edit. A group holds objects (operator.<key>,
# player.<id>, player.*, ip.<address, prefix or wildcard>, resource.<name>, resource.*) and the rights it allows and
# denies; a right is checked from its most specific level to its root, and among the entries of one level deny beats
# allow. The remote console answers only operator keys a group holds. Requests are resources' permission requests:
# pending until an operator allows or denies them (`by` and `at` record who and when; Unix seconds, UTC).
";

fn list(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.map(|i| toml_string(&i)).collect();
    if items.is_empty() { "[]".into() } else { format!("[\n    {},\n]", items.join(",\n    ")) }
}

pub(super) fn write(acl: &Acl) -> String {
    let mut out = String::from(HEADER);
    for g in &acl.groups {
        out.push_str("\n[[group]]\n");
        out.push_str(&format!("name = {}\n", toml_string(&g.name)));
        out.push_str(&format!("objects = {}\n", list(g.objects.iter().map(|o| o.to_string()))));
        out.push_str(&format!("allow = {}\n", list(g.allow.iter().cloned())));
        out.push_str(&format!("deny = {}\n", list(g.deny.iter().cloned())));
    }
    for q in &acl.requests {
        out.push_str("\n[[request]]\n");
        out.push_str(&format!("resource = {}\n", toml_string(&q.resource)));
        out.push_str(&format!("right = {}\n", toml_string(&q.right)));
        out.push_str(&format!("state = \"{}\"\n", q.state.name()));
        out.push_str(&format!("asked = {}  # {}\n", q.asked, ssx_net_proto::operator::utc(q.asked)));
        out.push_str(&format!("by = {}\n", toml_string(&q.by)));
        if q.at == 0 {
            out.push_str("at = 0\n");
        } else {
            out.push_str(&format!("at = {}  # {}\n", q.at, ssx_net_proto::operator::utc(q.at)));
        }
    }
    out
}
