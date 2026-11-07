use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::value::{ElementId, NetValue};

pub const ROOT: ElementId = ElementId(1);
pub const LOCAL_BASE: u64 = 1 << 52;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Creator {
    Host,
    Session,
    Package(String),
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    Local,
    Subscribers,
    Broadcast,
}

impl Sync {
    pub fn name(self) -> &'static str {
        match self {
            Sync::Local => "local",
            Sync::Subscribers => "subscribers",
            Sync::Broadcast => "broadcast",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Writer {
    Package(String),
    Client(ClientId),
    Server,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Datum {
    pub value: NetValue,
    pub bytes: usize,
    pub sync: Sync,
    pub writer: Writer,
}

#[derive(Clone, Debug)]
pub struct Element {
    pub kind: String,
    pub name: String,
    pub parent: Option<ElementId>,
    pub children: Vec<ElementId>,
    pub creator: Creator,
    pub owner: Option<ClientId>,
    pub depth: usize,
    pub data: BTreeMap<String, Datum>,
    pub writable: BTreeMap<String, String>,
    pub subscribers: BTreeMap<String, BTreeMap<ClientId, String>>,
}

impl Element {
    fn new(
        kind: &str,
        name: String,
        parent: ElementId,
        creator: Creator,
        owner: Option<ClientId>,
        depth: usize,
    ) -> Self {
        Element {
            kind: kind.to_string(),
            name,
            parent: Some(parent),
            children: Vec::new(),
            creator,
            owner,
            depth,
            data: BTreeMap::new(),
            writable: BTreeMap::new(),
            subscribers: BTreeMap::new(),
        }
    }
}

pub struct Tree {
    next: u64,
    next_local: u64,
    map: HashMap<ElementId, Element>,
    by_type: BTreeMap<String, BTreeSet<ElementId>>,
    by_name: BTreeMap<String, BTreeSet<ElementId>>,
}

impl Default for Tree {
    fn default() -> Self {
        let root = Element {
            kind: "root".into(),
            name: String::new(),
            parent: None,
            children: Vec::new(),
            creator: Creator::Host,
            owner: None,
            depth: 0,
            data: BTreeMap::new(),
            writable: BTreeMap::new(),
            subscribers: BTreeMap::new(),
        };
        let mut t = Tree {
            next: ROOT.0 + 1,
            next_local: LOCAL_BASE,
            map: HashMap::new(),
            by_type: BTreeMap::new(),
            by_name: BTreeMap::new(),
        };
        t.index(ROOT, &root);
        t.map.insert(ROOT, root);
        t
    }
}

impl Tree {
    fn index(&mut self, id: ElementId, e: &Element) {
        self.by_type.entry(e.kind.clone()).or_default().insert(id);
        if !e.name.is_empty() {
            self.by_name.entry(e.name.clone()).or_default().insert(id);
        }
    }

    fn unindex(&mut self, id: ElementId, e: &Element) {
        if let Some(s) = self.by_type.get_mut(&e.kind) {
            s.remove(&id);
            if s.is_empty() {
                self.by_type.remove(&e.kind);
            }
        }
        if let Some(s) = self.by_name.get_mut(&e.name) {
            s.remove(&id);
            if s.is_empty() {
                self.by_name.remove(&e.name);
            }
        }
    }

    pub fn get(&self, id: ElementId) -> Option<&Element> {
        self.map.get(&id)
    }

    pub fn get_mut(&mut self, id: ElementId) -> Option<&mut Element> {
        self.map.get_mut(&id)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_local(id: ElementId) -> bool {
        id.0 >= LOCAL_BASE
    }

    pub fn create(
        &mut self,
        kind: &str,
        parent: ElementId,
        creator: Creator,
        owner: Option<ClientId>,
        local: bool,
        max_depth: usize,
    ) -> Result<ElementId, String> {
        let depth = self.map.get(&parent).ok_or("the parent element does not exist")?.depth + 1;
        if depth > max_depth {
            return Err(format!("the element would sit {depth} levels below the root (at most {max_depth})"));
        }
        let id = if local {
            let id = ElementId(self.next_local);
            self.next_local += 1;
            id
        } else {
            let id = ElementId(self.next);
            self.next += 1;
            id
        };
        self.insert(id, Element::new(kind, String::new(), parent, creator, owner, depth));
        Ok(id)
    }

    fn insert(&mut self, id: ElementId, e: Element) {
        let parent = e.parent;
        self.index(id, &e);
        self.map.insert(id, e);
        if let Some(p) = parent.and_then(|p| self.map.get_mut(&p)) {
            p.children.push(id);
        }
    }

    pub fn insert_mirror(
        &mut self,
        id: ElementId,
        kind: &str,
        name: &str,
        parent: ElementId,
        max_depth: usize,
    ) -> Result<(), String> {
        if Self::is_local(id) || id == ROOT || self.map.contains_key(&id) {
            return Err(format!("element {id} cannot be mirrored (local, the root, or known)"));
        }
        if Self::is_local(parent) {
            return Err(format!("element {id} cannot be mirrored below this client's own element {parent}"));
        }
        let depth = self.map.get(&parent).ok_or("the parent element is not known")?.depth + 1;
        if depth > max_depth {
            return Err(format!("the element would sit {depth} levels below the root (at most {max_depth})"));
        }
        self.insert(id, Element::new(kind, name.to_string(), parent, Creator::Server, None, depth));
        Ok(())
    }

    pub fn destroy(&mut self, id: ElementId) -> Vec<(ElementId, Element)> {
        if id == ROOT || !self.map.contains_key(&id) {
            return Vec::new();
        }
        let mut order = vec![id];
        let mut i = 0;
        while i < order.len() {
            if let Some(e) = self.map.get(&order[i]) {
                order.extend(e.children.iter().copied());
            }
            i += 1;
        }
        if let Some(p) = self.map.get(&id).and_then(|e| e.parent)
            && let Some(parent) = self.map.get_mut(&p)
        {
            parent.children.retain(|c| *c != id);
        }
        let mut removed = Vec::with_capacity(order.len());
        for id in order.into_iter().rev() {
            if let Some(e) = self.map.remove(&id) {
                self.unindex(id, &e);
                removed.push((id, e));
            }
        }
        removed
    }

    pub fn is_ancestor(&self, a: ElementId, b: ElementId) -> bool {
        let mut at = self.map.get(&b).and_then(|e| e.parent);
        while let Some(p) = at {
            if p == a {
                return true;
            }
            at = self.map.get(&p).and_then(|e| e.parent);
        }
        false
    }

    pub fn set_parent(&mut self, id: ElementId, parent: ElementId, max_depth: usize) -> Result<(), String> {
        if id == ROOT {
            return Err("the root cannot move".into());
        }
        if id == parent || self.is_ancestor(id, parent) {
            return Err("an element cannot move below itself".into());
        }
        let base = self.map.get(&parent).ok_or("the parent element does not exist")?.depth + 1;
        let old = self.map.get(&id).ok_or("the element does not exist")?;
        let (old_parent, old_depth) = (old.parent, old.depth);
        let mut sub = vec![id];
        let mut i = 0;
        let mut deepest = old_depth;
        while i < sub.len() {
            if let Some(e) = self.map.get(&sub[i]) {
                deepest = deepest.max(e.depth);
                sub.extend(e.children.iter().copied());
            }
            i += 1;
        }
        let new_deepest = deepest - old_depth + base;
        if new_deepest > max_depth {
            return Err(format!("the elements would sit {new_deepest} levels below the root (at most {max_depth})"));
        }
        if let Some(p) = old_parent
            && let Some(op) = self.map.get_mut(&p)
        {
            op.children.retain(|c| *c != id);
        }
        if let Some(np) = self.map.get_mut(&parent) {
            np.children.push(id);
        }
        for s in sub {
            if let Some(e) = self.map.get_mut(&s) {
                e.depth = e.depth - old_depth + base;
                if s == id {
                    e.parent = Some(parent);
                }
            }
        }
        Ok(())
    }

    pub fn set_name(&mut self, id: ElementId, name: &str) {
        let Some(e) = self.map.get(&id) else { return };
        let old = e.name.clone();
        if let Some(s) = self.by_name.get_mut(&old) {
            s.remove(&id);
            if s.is_empty() {
                self.by_name.remove(&old);
            }
        }
        if !name.is_empty() {
            self.by_name.entry(name.to_string()).or_default().insert(id);
        }
        if let Some(e) = self.map.get_mut(&id) {
            e.name = name.to_string();
        }
    }

    pub fn by_name(&self, name: &str, index: usize) -> Option<ElementId> {
        self.by_name.get(name).and_then(|s| s.iter().nth(index).copied())
    }

    pub fn by_type(&self, kind: &str) -> impl Iterator<Item = ElementId> + '_ {
        self.by_type.get(kind).into_iter().flat_map(|s| s.iter().copied())
    }

    pub fn iter(&self) -> impl Iterator<Item = (ElementId, &Element)> {
        self.map.iter().map(|(k, v)| (*k, v))
    }

    pub fn subtree(&self, id: ElementId) -> Vec<ElementId> {
        let mut out = vec![id];
        let mut i = 0;
        while i < out.len() {
            if let Some(e) = self.map.get(&out[i]) {
                out.extend(e.children.iter().copied());
            }
            i += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg(n: &str) -> Creator {
        Creator::Package(n.into())
    }

    #[test]
    fn subtrees_move_and_go_as_a_whole() {
        let mut t = Tree::default();
        let a = t.create("thing", ROOT, pkg("a"), None, false, 64).unwrap();
        let b = t.create("thing", a, pkg("a"), None, false, 64).unwrap();
        let c = t.create("thing", b, pkg("b"), None, false, 64).unwrap();
        assert!(t.is_ancestor(a, c) && !t.is_ancestor(c, a));
        assert!(t.set_parent(a, c, 64).is_err(), "a cycle");
        assert!(t.set_parent(a, a, 64).is_err());
        t.set_parent(c, ROOT, 64).unwrap();
        assert_eq!(t.get(c).unwrap().depth, 1);
        let gone: Vec<ElementId> = t.destroy(a).into_iter().map(|(id, _)| id).collect();
        assert_eq!(gone, vec![b, a], "children first");
        assert!(t.get(c).is_some() && t.get(ROOT).unwrap().children == vec![c]);
        assert!(t.destroy(ROOT).is_empty(), "the root stays");
        let l = t.create("x", ROOT, pkg("a"), None, true, 64).unwrap();
        assert!(Tree::is_local(l) && !Tree::is_local(c));
    }

    #[test]
    fn depth_is_capped_on_create_and_move() {
        let mut t = Tree::default();
        let mut at = ROOT;
        for _ in 0..4 {
            at = t.create("x", at, pkg("a"), None, false, 4).unwrap();
        }
        assert!(t.create("x", at, pkg("a"), None, false, 4).is_err());
        let other = t.create("x", ROOT, pkg("a"), None, false, 4).unwrap();
        let below = t.create("x", other, pkg("a"), None, false, 4).unwrap();
        assert!(t.set_parent(other, at, 4).is_err(), "its child would sit at 6");
        let deep = t.get(at).unwrap().parent.unwrap();
        assert!(t.set_parent(below, deep, 4).is_ok());
        assert_eq!(t.get(below).unwrap().depth, 4);
    }

    #[test]
    fn a_ten_thousand_chain_is_walked_without_recursion() {
        let mut t = Tree::default();
        let mut at = ROOT;
        let first = t.create("x", at, pkg("a"), None, false, 20_000).unwrap();
        at = first;
        for _ in 0..10_000 {
            at = t.create("x", at, pkg("a"), None, false, 20_000).unwrap();
        }
        assert!(t.is_ancestor(first, at));
        assert_eq!(t.destroy(first).len(), 10_001);
        assert_eq!(t.len(), 1);
    }
}
