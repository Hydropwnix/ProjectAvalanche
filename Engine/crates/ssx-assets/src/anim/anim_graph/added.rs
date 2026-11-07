use std::collections::{BTreeSet, HashMap};

use ssx_content::ContentId;
use ssx_content::trick::TrickToml;
use ssx_formats::ant::Guid;

use super::{ChooserEntry, Ctrl, CtrlId, Graph, Selector};
use crate::anim_db::AssetDb;
use crate::content::tricks::TrickRoutes;

const TRICK_SIGNALS: [&str; 2] = ["GSENUM_TrickType", "GSENUM_TrickTypeLocked"];

fn child_slots(c: &mut Ctrl) -> Vec<&mut Option<CtrlId>> {
    match c {
        Ctrl::StateFlow(sf) => sf.nodes.iter_mut().map(|n| &mut n.subject).collect(),
        Ctrl::Modular(items) => items.iter_mut().map(|x| &mut x.0).collect(),
        Ctrl::Physics { subject, .. }
        | Ctrl::TrajMask { subject }
        | Ctrl::Looping { subject, .. }
        | Ctrl::SignalPhase { subject, .. }
        | Ctrl::PlaybackRate { subject, .. }
        | Ctrl::AutoMirror { subject, .. }
        | Ctrl::Mirror { subject, .. } => vec![subject],
        Ctrl::Chooser { entries, .. } => entries.iter_mut().map(|e| &mut e.controller).collect(),
        Ctrl::Sequence { anims, .. } => anims.iter_mut().map(|a| &mut a.controller).collect(),
        Ctrl::ReferencePoseBlend { subject, reference, .. } | Ctrl::OffsetPose { subject, reference, .. } => {
            vec![subject, reference]
        }
        Ctrl::BlendArray { items, master, .. } => {
            std::iter::once(master).chain(items.iter_mut().map(|i| &mut i.controller)).collect()
        }
        Ctrl::Layers(l) => l.iter_mut().map(|x| &mut x.controller).collect(),
        Ctrl::Proxy { default, resolved, .. } => vec![default, resolved],
        Ctrl::ContextDb { entries, .. } => entries.iter_mut().map(|e| &mut e.0).collect(),
        Ctrl::BlendSpace { items, .. } => items.iter_mut().map(|i| &mut i.1).collect(),
        Ctrl::Clip { .. } | Ctrl::Other(_) => Vec::new(),
    }
}

pub fn children(c: &Ctrl) -> Vec<CtrlId> {
    let mut c = c.clone();
    child_slots(&mut c).into_iter().filter_map(|s| *s).collect()
}

pub fn reach(g: &Graph, root: CtrlId) -> Vec<CtrlId> {
    let (mut order, mut stack, mut seen) = (Vec::new(), vec![root], BTreeSet::new());
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        order.push(id);
        stack.extend(children(&g.ctrls[id].1));
    }
    order
}

fn trick_choosers(g: &Graph) -> Vec<CtrlId> {
    let Some(air) = g.ctrl_id("SF_AirTricks") else { return Vec::new() };
    reach(g, air)
        .into_iter()
        .filter(|&id| match &g.ctrls[id].1 {
            Ctrl::Chooser { policy, policy_signal: Some(s), .. } => {
                policy == "EnumChooserPolicyAsset" && TRICK_SIGNALS.contains(&g.signals[*s].name.as_str())
            }
            _ => false,
        })
        .collect()
}

pub fn add_tricks(g: &mut Graph, tricks: &TrickRoutes, db: &AssetDb) -> Vec<String> {
    let choosers = trick_choosers(g);
    if choosers.is_empty() {
        return Vec::new();
    }
    let mut problems = Vec::new();
    for t in tricks.tricks() {
        let guids: Vec<Guid> = t.clips.iter().map(|c| c.0).collect();
        let added = match guids.iter().find(|g| db.get(**g).is_some()) {
            Some(g) => Err(format!("its clip GUID {:02x?} is a disc asset's", g.0)),
            None => add_trick(g, &choosers, &t.id, &t.trick, &guids),
        };
        if let Err(why) = added {
            problems.push(format!("{}: left out of the riding graph: {why}", t.id));
        }
    }
    problems
}

fn add_trick(g: &mut Graph, choosers: &[CtrlId], id: &ContentId, t: &TrickToml, guids: &[Guid]) -> Result<(), String> {
    let value = i64::from(t.value());
    let base = i64::from(t.base_value());
    if guids.len() != ssx_content::trick::ROLES.len() {
        return Err(format!("{} clip GUIDs for 12 places", guids.len()));
    }
    let roles: HashMap<String, (usize, Guid)> =
        guids.iter().enumerate().map(|(r, guid)| (t.base_clip(r), (r, *guid))).collect();
    let (start, mut map) = (g.ctrls.len(), HashMap::<CtrlId, CtrlId>::new());
    let (mut copies, mut entries) = (Vec::new(), Vec::new());
    for &ch in choosers {
        let Ctrl::Chooser { entries: list, .. } = &g.ctrls[ch].1 else { continue };
        if list.iter().any(|e| e.selector == Selector::Enum(value)) {
            return Err(format!("{} has an entry of {value} already", g.ctrls[ch].0));
        }
        let Some(root) = list.iter().find(|e| e.selector == Selector::Enum(base)).and_then(|e| e.controller) else {
            return Err(format!("{} has no entry of the base's value {base}", g.ctrls[ch].0));
        };
        for id in reach(g, root) {
            if let std::collections::hash_map::Entry::Vacant(e) = map.entry(id) {
                e.insert(start + copies.len());
                copies.push(id);
            }
        }
        entries.push((ch, map[&root]));
    }
    let mut made = Vec::with_capacity(copies.len());
    let mut bound = BTreeSet::new();
    for &original in &copies {
        let (name, mut c) = g.ctrls[original].clone();
        for s in child_slots(&mut c) {
            *s = s.map(|k| map[&k]);
        }
        if let Ctrl::Clip { anim, anim_guid, target, .. } = &mut c {
            let Some(&(r, guid)) = roles.get(anim.as_str()) else {
                return Err(format!("the base plays the clip {anim} in no place of a grab's 12"));
            };
            if target != "AnimRig" {
                return Err(format!("the base's clip {anim} plays on {target}"));
            }
            *anim = format!("{id} {}", ssx_content::trick::ROLES[r].0);
            *anim_guid = Some(guid);
            bound.insert(r);
        }
        made.push((name, c));
    }
    if bound.len() != roles.len() {
        return Err(format!("the base plays {} of its 12 clip places", bound.len()));
    }
    g.ctrls.extend(made);
    for (ch, root) in entries {
        if let Ctrl::Chooser { entries: list, .. } = &mut g.ctrls[ch].1 {
            list.insert(
                list.len().saturating_sub(1),
                ChooserEntry { controller: Some(root), selector: Selector::Enum(value) },
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
