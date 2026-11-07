use super::{Acl, RESOURCE_ALLOWED, RESOURCE_DENIED, RequestState, Subject};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Console,
    Group(String),
    Request,
    Default,
    Nothing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub allowed: bool,
    pub source: Source,
    pub level: Option<String>,
}

fn levels(right: &str) -> impl Iterator<Item = &str> {
    let mut next = Some(right);
    std::iter::from_fn(move || {
        let current = next?;
        next = current.rfind('.').map(|k| &current[..k]);
        Some(current)
    })
}

impl Acl {
    pub fn allows(&self, subject: Subject<'_>, right: &str) -> bool {
        self.check(subject, right).allowed
    }

    pub fn check(&self, subject: Subject<'_>, right: &str) -> Verdict {
        if matches!(subject, Subject::Console) {
            return Verdict { allowed: true, source: Source::Console, level: None };
        }
        if super::right(right).is_err() {
            return Verdict { allowed: false, source: Source::Nothing, level: None };
        }
        let groups: Vec<&super::Group> =
            self.groups.iter().filter(|g| g.objects.iter().any(|o| o.holds(&subject))).collect();
        let resource = match subject {
            Subject::Resource(name) => Some(name),
            _ => None,
        };
        for level in levels(right) {
            let decided = |allowed: bool, source: Source| Verdict { allowed, source, level: Some(level.to_string()) };
            if let Some(g) = groups.iter().find(|g| g.deny.iter().any(|r| r == level)) {
                return decided(false, Source::Group(g.name.clone()));
            }
            let request = |state: RequestState| {
                resource.is_some_and(|name| {
                    self.requests.iter().any(|q| q.resource == name && q.right == level && q.state == state)
                })
            };
            if request(RequestState::Denied) {
                return decided(false, Source::Request);
            }
            if let Some(g) = groups.iter().find(|g| g.allow.iter().any(|r| r == level)) {
                return decided(true, Source::Group(g.name.clone()));
            }
            if request(RequestState::Allowed) {
                return decided(true, Source::Request);
            }
            if resource.is_some() {
                if RESOURCE_DENIED.contains(&level) {
                    return decided(false, Source::Default);
                }
                if RESOURCE_ALLOWED.contains(&level) {
                    return decided(true, Source::Default);
                }
            }
        }
        Verdict { allowed: false, source: Source::Nothing, level: None }
    }
}

impl Acl {
    pub fn check_all(&self, subject: Subject<'_>, rights: &[String]) -> Vec<Verdict> {
        use std::collections::{HashMap, HashSet};
        if matches!(subject, Subject::Console) {
            return rights.iter().map(|_| Verdict { allowed: true, source: Source::Console, level: None }).collect();
        }
        let wanted: HashSet<&str> = rights.iter().flat_map(|r| levels(r)).collect();
        let (mut deny, mut allow) = (HashMap::<&str, &str>::new(), HashMap::<&str, &str>::new());
        for g in self.groups.iter().filter(|g| g.objects.iter().any(|o| o.holds(&subject))) {
            for r in g.deny.iter().filter(|r| wanted.contains(r.as_str())) {
                deny.entry(r.as_str()).or_insert(g.name.as_str());
            }
            for r in g.allow.iter().filter(|r| wanted.contains(r.as_str())) {
                allow.entry(r.as_str()).or_insert(g.name.as_str());
            }
        }
        let resource = match subject {
            Subject::Resource(name) => Some(name),
            _ => None,
        };
        let (mut denied, mut allowed) = (HashSet::<&str>::new(), HashSet::<&str>::new());
        for q in self.requests.iter().filter(|q| resource == Some(q.resource.as_str())) {
            match q.state {
                RequestState::Denied => denied.insert(q.right.as_str()),
                RequestState::Allowed => allowed.insert(q.right.as_str()),
                RequestState::Pending => false,
            };
        }
        let one = |right: &str| {
            if super::right(right).is_err() {
                return Verdict { allowed: false, source: Source::Nothing, level: None };
            }
            for level in levels(right) {
                let decided =
                    |allowed: bool, source: Source| Verdict { allowed, source, level: Some(level.to_string()) };
                if let Some(g) = deny.get(level) {
                    return decided(false, Source::Group((*g).to_string()));
                }
                if denied.contains(level) {
                    return decided(false, Source::Request);
                }
                if let Some(g) = allow.get(level) {
                    return decided(true, Source::Group((*g).to_string()));
                }
                if allowed.contains(level) {
                    return decided(true, Source::Request);
                }
                if resource.is_some() {
                    if RESOURCE_DENIED.contains(&level) {
                        return decided(false, Source::Default);
                    }
                    if RESOURCE_ALLOWED.contains(&level) {
                        return decided(true, Source::Default);
                    }
                }
            }
            Verdict { allowed: false, source: Source::Nothing, level: None }
        };
        rights.iter().map(|r| one(r)).collect()
    }
}

#[cfg(test)]
#[test]
fn levels_walk_to_the_root() {
    assert_eq!(levels("function.db.exec").collect::<Vec<_>>(), ["function.db.exec", "function.db", "function"]);
    assert_eq!(levels("command").collect::<Vec<_>>(), ["command"]);
}
