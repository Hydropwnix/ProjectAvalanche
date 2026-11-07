use super::{
    Acl, Group, MAX_GROUPS, MAX_OBJECTS, MAX_REQUESTS, MAX_RIGHTS, Object, Request, RequestState, group_name, resource,
    right,
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AclError {
    #[error("{0}")]
    Rule(String),
    #[error("no group {}", ssx_core::text::quoted(.0))]
    NoGroup(String),
    #[error("the group {} exists", ssx_core::text::quoted(.0))]
    GroupExists(String),
    #[error("{0}")]
    Full(&'static str),
    #[error("{0}")]
    Unchanged(String),
}

fn clock(now: u64) -> Result<(), AclError> {
    if now == 0 || now > i64::MAX as u64 {
        return Err(AclError::Rule("the server's clock is outside the file's times".into()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allow,
    Deny,
}

impl Acl {
    fn group_mut(&mut self, name: &str) -> Result<&mut Group, AclError> {
        self.groups.iter_mut().find(|g| g.name.eq_ignore_ascii_case(name)).ok_or_else(|| AclError::NoGroup(name.into()))
    }

    pub fn group(&self, name: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.name.eq_ignore_ascii_case(name))
    }

    pub fn create_group(&mut self, name: &str) -> Result<(), AclError> {
        group_name(name).map_err(AclError::Rule)?;
        if self.group(name).is_some() {
            return Err(AclError::GroupExists(name.into()));
        }
        if self.groups.len() >= MAX_GROUPS {
            return Err(AclError::Full("the ACL holds its most groups"));
        }
        self.groups.push(Group { name: name.into(), objects: Vec::new(), allow: Vec::new(), deny: Vec::new() });
        Ok(())
    }

    pub fn delete_group(&mut self, name: &str) -> Result<Group, AclError> {
        let k = self.groups.iter().position(|g| g.name.eq_ignore_ascii_case(name));
        k.map(|k| self.groups.remove(k)).ok_or_else(|| AclError::NoGroup(name.into()))
    }

    pub fn add_object(&mut self, group: &str, object: Object) -> Result<(), AclError> {
        let g = self.group_mut(group)?;
        if g.objects.contains(&object) {
            return Err(AclError::Unchanged(format!("{} holds {object} already", g.name)));
        }
        if g.objects.len() >= MAX_OBJECTS {
            return Err(AclError::Full("the group holds its most objects"));
        }
        g.objects.push(object);
        Ok(())
    }

    pub fn remove_object(&mut self, group: &str, object: &Object) -> Result<(), AclError> {
        let g = self.group_mut(group)?;
        let k = g.objects.iter().position(|o| o == object);
        let k = k.ok_or_else(|| AclError::Unchanged(format!("{} does not hold {object}", g.name)))?;
        g.objects.remove(k);
        Ok(())
    }

    pub fn set_right(&mut self, group: &str, right_text: &str, access: Access) -> Result<(), AclError> {
        right(right_text).map_err(AclError::Rule)?;
        let g = self.group_mut(group)?;
        let (to, from) = match access {
            Access::Allow => (&mut g.allow, &mut g.deny),
            Access::Deny => (&mut g.deny, &mut g.allow),
        };
        from.retain(|r| r != right_text);
        if to.iter().any(|r| r == right_text) {
            return Err(AclError::Unchanged(format!("{} has {right_text} so already", g.name)));
        }
        if g.allow.len() + g.deny.len() >= MAX_RIGHTS {
            return Err(AclError::Full("the group holds its most rights"));
        }
        let to = match access {
            Access::Allow => &mut g.allow,
            Access::Deny => &mut g.deny,
        };
        to.push(right_text.into());
        Ok(())
    }

    pub fn revoke(&mut self, group: &str, right_text: &str) -> Result<(), AclError> {
        let g = self.group_mut(group)?;
        let before = g.allow.len() + g.deny.len();
        g.allow.retain(|r| r != right_text);
        g.deny.retain(|r| r != right_text);
        if g.allow.len() + g.deny.len() == before {
            return Err(AclError::Unchanged(format!(
                "{} has no entry for {}",
                g.name,
                ssx_core::text::quoted(right_text)
            )));
        }
        Ok(())
    }

    pub fn request(&mut self, resource_name: &str, right_text: &str, now: u64) -> Result<RequestState, AclError> {
        resource(resource_name).map_err(AclError::Rule)?;
        right(right_text).map_err(AclError::Rule)?;
        clock(now)?;
        if let Some(q) = self.requests.iter().find(|q| q.resource == resource_name && q.right == right_text) {
            return Ok(q.state);
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err(AclError::Full("the ACL holds its most requests"));
        }
        self.requests.push(Request {
            resource: resource_name.into(),
            right: right_text.into(),
            state: RequestState::Pending,
            asked: now,
            by: String::new(),
            at: 0,
        });
        Ok(RequestState::Pending)
    }

    pub fn decide(
        &mut self,
        resource_name: &str,
        right_text: Option<&str>,
        allow: bool,
        by: &str,
        now: u64,
    ) -> Result<usize, AclError> {
        crate::rules::by(by).map_err(AclError::Rule)?;
        clock(now)?;
        let state = if allow { RequestState::Allowed } else { RequestState::Denied };
        let mut n = 0;
        for q in self.requests.iter_mut().filter(|q| q.resource == resource_name) {
            let hit = match right_text {
                Some(r) => q.right == r,
                None => q.state == RequestState::Pending,
            };
            if hit && q.state != state {
                q.state = state;
                q.by = by.into();
                q.at = now;
                n += 1;
            }
        }
        if n == 0 {
            let what = right_text
                .map_or("pending request".to_string(), |r| format!("request for {}", ssx_core::text::quoted(r)));
            return Err(AclError::Unchanged(format!(
                "{} has no {what} to change",
                ssx_core::text::quoted(resource_name)
            )));
        }
        Ok(n)
    }
}
