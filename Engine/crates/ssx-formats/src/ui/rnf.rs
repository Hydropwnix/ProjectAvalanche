use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Drop,
    Unlock,
    OpenRange,
    UnlockGear,
    UnlockChar,
    UnlockChapter,
    UnlockMode,
    RemoveDrop,
}

impl NodeKind {
    pub fn from_word(word: &str) -> Option<Self> {
        Some(match word {
            "drop" => NodeKind::Drop,
            "unlock" => NodeKind::Unlock,
            "open_range" => NodeKind::OpenRange,
            "unlock_gear" => NodeKind::UnlockGear,
            "unlock_char" => NodeKind::UnlockChar,
            "unlock_chapter" => NodeKind::UnlockChapter,
            "unlock_mode" => NodeKind::UnlockMode,
            "remove_drop" => NodeKind::RemoveDrop,
            _ => return None,
        })
    }

    pub fn id(self) -> u32 {
        self as u32
    }

    pub fn word(self) -> &'static str {
        match self {
            NodeKind::Drop => "drop",
            NodeKind::Unlock => "unlock",
            NodeKind::OpenRange => "open_range",
            NodeKind::UnlockGear => "unlock_gear",
            NodeKind::UnlockChar => "unlock_char",
            NodeKind::UnlockChapter => "unlock_chapter",
            NodeKind::UnlockMode => "unlock_mode",
            NodeKind::RemoveDrop => "remove_drop",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: i32,
    pub kind: NodeKind,
    pub attribs: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CampaignGraph {
    pub nodes: Vec<Node>,
    pub transitions: Vec<(i32, i32)>,
}

pub const MAX_NODES: usize = 256;

fn lines(data: &[u8]) -> Result<Vec<&str>> {
    let text = std::str::from_utf8(data).map_err(|e| Error::invalid(format!("rnf: not UTF-8: {e}")))?;
    Ok(text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect())
}

fn int_after(line: &str, prefix: &str, at: usize) -> Result<i32> {
    line.strip_prefix(prefix)
        .and_then(|v| v.trim().parse().ok())
        .ok_or_else(|| Error::invalid(format!("rnf line {}: expected `{prefix} <int>`, found {line:?}", at + 1)))
}

impl CampaignGraph {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let lines = lines(data)?;
        let mut g = CampaignGraph::default();
        let mut i = 0;
        let next = |i: &mut usize| -> Result<&str> {
            let l = lines.get(*i).copied().ok_or_else(|| Error::invalid("rnf: unexpected end of file"))?;
            *i += 1;
            Ok(l)
        };
        while i < lines.len() {
            let line = lines[i];
            i += 1;
            match line.trim_end() {
                "*startstate" => {
                    let at = i;
                    let id = int_after(next(&mut i)?, "id:", at)?;
                    let type_line = next(&mut i)?;
                    let word = type_line
                        .strip_prefix("type:")
                        .map(str::trim)
                        .ok_or_else(|| Error::invalid(format!("rnf line {}: expected `type:`", at + 2)))?;
                    let kind = NodeKind::from_word(word)
                        .ok_or_else(|| Error::invalid(format!("rnf line {}: unknown node type {word:?}", at + 2)))?;
                    let mut attribs = Vec::new();
                    loop {
                        let l = next(&mut i)?;
                        if l.trim_end() == "*endstate" {
                            break;
                        }
                        if let Some(v) = l.strip_prefix("attrib:") {
                            let v = v.split_whitespace().next().unwrap_or("");
                            attribs.push(v.to_string());
                        } else if !l.trim().is_empty() {
                            return Err(Error::invalid(format!("rnf line {i}: unexpected {l:?} in a state")));
                        }
                    }
                    if g.nodes.iter().any(|n| n.id == id) {
                        return Err(Error::invalid(format!("rnf: duplicate state id {id}")));
                    }
                    g.nodes.push(Node { id, kind, attribs });
                }
                "*starttransition" => {
                    let at = i;
                    let source = int_after(next(&mut i)?, "source:", at)?;
                    let target = int_after(next(&mut i)?, "target:", at + 1)?;
                    if next(&mut i)?.trim_end() != "*endtransition" {
                        return Err(Error::invalid(format!("rnf line {}: expected `*endtransition`", at + 3)));
                    }
                    g.transitions.push((source, target));
                }
                _ => {}
            }
        }
        if g.nodes.len() > MAX_NODES {
            return Err(Error::invalid(format!("rnf: {} states, the game holds {MAX_NODES}", g.nodes.len())));
        }
        for &(s, t) in &g.transitions {
            if g.node(s).is_none() || g.node(t).is_none() {
                return Err(Error::invalid(format!("rnf: transition {s} -> {t} names an unknown state")));
            }
        }
        Ok(g)
    }

    pub fn node(&self, id: i32) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn children(&self, id: i32) -> impl Iterator<Item = i32> + '_ {
        self.transitions.iter().filter(move |t| t.0 == id).map(|t| t.1)
    }

    pub fn parents(&self, id: i32) -> impl Iterator<Item = i32> + '_ {
        self.transitions.iter().filter(move |t| t.1 == id).map(|t| t.0)
    }

    pub fn drop_node(&self, ui_drop: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.kind == NodeKind::Drop && n.attribs.first().is_some_and(|a| a == ui_drop))
    }

    pub fn is_acyclic(&self) -> bool {
        let mut indeg: std::collections::HashMap<i32, usize> = self.nodes.iter().map(|n| (n.id, 0)).collect();
        for &(_, t) in &self.transitions {
            *indeg.entry(t).or_default() += 1;
        }
        let mut ready: Vec<i32> = indeg.iter().filter(|(_, d)| **d == 0).map(|(id, _)| *id).collect();
        let mut seen = 0;
        while let Some(id) = ready.pop() {
            seen += 1;
            for c in self.children(id).collect::<Vec<_>>() {
                if let Some(d) = indeg.get_mut(&c) {
                    *d -= 1;
                    if *d == 0 {
                        ready.push(c);
                    }
                }
            }
        }
        seen == self.nodes.len()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DropPositions {
    pub entries: Vec<(String, f32, f32)>,
}

impl DropPositions {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut entries = Vec::new();
        for (n, line) in lines(data)?.into_iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let bad = || Error::invalid(format!("droppos line {}: expected `name x,y`, found {line:?}", n + 1));
            let (name, rest) = line.trim().split_once(char::is_whitespace).ok_or_else(bad)?;
            let (x, y) = rest.split_once(',').ok_or_else(bad)?;
            let x: f32 = x.trim().parse().map_err(|_| bad())?;
            let y: f32 = y.trim().parse().map_err(|_| bad())?;
            entries.push((name.to_string(), x, y));
        }
        Ok(DropPositions { entries })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPH: &str = "*startstate\r\nid: 1\r\ntype: drop\r\nattrib: a\r\n*endstate\r\n\r\n\
                         *startstate\r\nid: 2\r\ntype: unlock_mode\r\nattrib: explore\r\nattrib: campaign\r\n*endstate\r\n\
                         *starttransition\r\nsource: 1\r\ntarget: 2\r\n*endtransition\r\n";

    #[test]
    fn states_and_transitions() {
        let g = CampaignGraph::parse(GRAPH.as_bytes()).unwrap();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes[1].kind, NodeKind::UnlockMode);
        assert_eq!(g.nodes[1].attribs, ["explore", "campaign"]);
        assert_eq!(g.children(1).collect::<Vec<_>>(), [2]);
        assert_eq!(g.parents(2).collect::<Vec<_>>(), [1]);
        assert_eq!(g.drop_node("a").map(|n| n.id), Some(1));
        assert!(g.is_acyclic());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(CampaignGraph::parse(b"*startstate\nid: 1\ntype: bogus\n*endstate\n").is_err());
        assert!(CampaignGraph::parse(b"*startstate\nid: 1\ntype: drop\n").is_err());
        assert!(CampaignGraph::parse(b"*starttransition\nsource: 1\ntarget: 2\n*endtransition\n").is_err());
        let cyc = "*startstate\nid: 1\ntype: drop\n*endstate\n*startstate\nid: 2\ntype: drop\n*endstate\n\
                   *starttransition\nsource: 1\ntarget: 2\n*endtransition\n*starttransition\nsource: 2\ntarget: 1\n*endtransition\n";
        assert!(!CampaignGraph::parse(cyc.as_bytes()).unwrap().is_acyclic());
    }

    #[test]
    fn drop_positions() {
        let p = DropPositions::parse(b"a 1.5,2\r\nb 3.25, 4.5\r\n").unwrap();
        assert_eq!(p.entries, [("a".to_string(), 1.5, 2.0), ("b".to_string(), 3.25, 4.5)]);
        assert!(DropPositions::parse(b"a 1.5\n").is_err());
    }
}
