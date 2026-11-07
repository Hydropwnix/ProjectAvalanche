use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use ssx_content::{BASE_NAMESPACE, ContentId, Kind};
use ssx_formats::ant::{Guid, Object, Stream, Value};
use ssx_formats::ant_anim::{Animation, ChannelToDof, Dof, dof_list};

use super::super::segment;

pub const CLIP_CLASSES: [&str; 5] =
    ["VbrAnimationAsset", "DctAnimationAsset", "RawAnimationAsset", "FrameAnimationAsset", "CurveAnimationAsset"];

pub const RIDER_RIG: &str = "genchar_rig";
pub const RIDER_TARGET: &str = "AnimRig";

#[derive(Clone, Debug, PartialEq)]
pub struct NamedClip {
    pub path: String,
    pub asset: usize,
    pub guid: Guid,
    pub class: String,
    pub name: String,
    pub controller: String,
}

pub fn named_clips(stem: &str, s: &Stream, left_out: &mut Vec<String>) -> Vec<NamedClip> {
    let Some(bundle) = segment(stem) else {
        left_out.push(format!("bundle {stem:?} outside the id alphabet"));
        return Vec::new();
    };
    let mut controller: HashMap<Guid, String> = HashMap::new();
    for a in s.assets_of_class("ClipControllerAsset") {
        if let Some(g) = a.object.get(&s.schema, "Anim").and_then(Value::as_guid) {
            controller.entry(g).or_insert_with(|| a.object.name(&s.schema).unwrap_or("").to_string());
        }
    }
    let mut found = Vec::new();
    for (i, a) in s.assets.iter().enumerate() {
        let class = a.object.class_chain(&s.schema).first().copied().unwrap_or("");
        if !CLIP_CLASSES.contains(&class) {
            continue;
        }
        let name = a.object.name(&s.schema).unwrap_or("");
        let guid = a.object.guid(&s.schema).unwrap_or_default();
        let ctrl = controller.get(&guid).cloned().unwrap_or_default();
        found.push((i, guid, class.to_string(), name.to_string(), ctrl));
    }
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for f in &found {
        *uses.entry(f.3.strip_suffix(" Anim").unwrap_or(&f.3)).or_default() += 1;
    }
    let mut out = Vec::new();
    for (asset, guid, class, name, controller) in &found {
        let clip = name.strip_suffix(" Anim").unwrap_or(name);
        let shown = if uses[clip] > 1 { controller.as_str() } else { clip };
        match segment(shown).filter(|_| !shown.is_empty()) {
            Some(id) => out.push(NamedClip {
                path: format!("{bundle}/{id}"),
                asset: *asset,
                guid: *guid,
                class: class.clone(),
                name: name.clone(),
                controller: controller.clone(),
            }),
            None => {
                left_out.push(format!("{bundle}: clip {name:?} (controller {controller:?}) outside the id alphabet"))
            }
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscClip {
    pub id: ContentId,
    pub bundle: String,
    pub guid: Guid,
    pub controller: String,
    pub target: String,
    pub rig: String,
    pub num_ticks: f32,
    pub fps_scale: f32,
    pub end_frame: u16,
    pub additive: bool,
    pub drives: Vec<String>,
    pub canonical: Option<ContentId>,
}

impl DiscClip {
    pub fn not_offered(&self) -> Option<String> {
        if let Some(c) = &self.canonical {
            return Some(format!("{} is {c} in another bundle: replace it by that id (it replaces both)", self.id));
        }
        if self.rig != RIDER_RIG || self.target != RIDER_TARGET {
            return Some(format!(
                "{} plays on {} {}: a package replaces clips of the riders' body ({RIDER_RIG} {RIDER_TARGET}) only — \
                 the port plays no face clips yet, and the camera's and helicopter's rigs have no glTF skeleton",
                self.id,
                if self.rig.is_empty() { "no rig" } else { &self.rig },
                self.target
            ));
        }
        if self.controller.is_empty() {
            return Some(format!("{}: no clip controller plays it", self.id));
        }
        None
    }

    pub fn frame_rate(&self) -> f32 {
        crate::rig_anim::FPS * self.fps_scale
    }
}

#[derive(Debug, Default)]
pub struct DiscAnims {
    pub clips: BTreeMap<ContentId, DiscClip>,
    by_guid: HashMap<Guid, ContentId>,
    pub dofs: Vec<Dof>,
    pub notes: Vec<String>,
}

impl DiscAnims {
    pub fn index(big_root: &Path) -> Result<Self> {
        let dir = big_root.join(crate::rig_anim::ANIM_DIR);
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".cba"))
            .collect();
        names.sort();
        let mut streams = Vec::with_capacity(names.len());
        for n in &names {
            let p = dir.join(n);
            let data = std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
            let s = Stream::parse(&data).with_context(|| format!("parsing {}", p.display()))?;
            streams.push((n.trim_end_matches(".cba").to_string(), s));
        }
        Self::of_streams(&streams)
    }

    fn of_streams(streams: &[(String, Stream)]) -> Result<Self> {
        let all = &streams.iter().find(|s| s.0 == "anim_all").context("anim_all.cba is not among the bundles")?.1;
        let dofs = dof_list(all, RIDER_RIG, RIDER_TARGET).context("genchar_rig AnimRig DOF list")?;
        let owners = rig_owners(all);
        let all_index = all.guid_index();
        let mut out = DiscAnims { dofs, ..Default::default() };
        let mut lists: HashMap<(String, String), Vec<Dof>> = HashMap::new();
        for (stem, s) in streams {
            let index = s.guid_index();
            let lookup = |g: Guid| -> Option<(&ssx_formats::ant::Schema, &Object)> {
                match index.get(&g) {
                    Some(&i) => Some((&s.schema, &s.assets[i].object)),
                    None => all_index.get(&g).map(|&i| (&all.schema, &all.assets[i].object)),
                }
            };
            let ctrls: HashMap<Guid, &Object> = s
                .assets_of_class("ClipControllerAsset")
                .filter_map(|a| Some((a.object.get(&s.schema, "Anim").and_then(Value::as_guid)?, &a.object)))
                .collect();
            for n in named_clips(stem, s, &mut out.notes) {
                let id = ContentId::new(BASE_NAMESPACE, Kind::Anim, &n.path).map_err(anyhow::Error::msg)?;
                let anim = Animation::from_object(&s.schema, &s.assets[n.asset].object)
                    .with_context(|| format!("decoding {id}"))?
                    .with_context(|| format!("{id}: not an animation"))?;
                let ctrl = ctrls.get(&n.guid);
                let f = |field: &str| ctrl.and_then(|c| c.get(&s.schema, field)).and_then(Value::as_f32);
                let target_g = ctrl.and_then(|c| c.get(&s.schema, "Target")).and_then(Value::as_guid);
                let target = target_g
                    .and_then(|g| all_index.get(&g))
                    .and_then(|&i| all.assets[i].object.name(&all.schema))
                    .unwrap_or("")
                    .to_string();
                let rig = target_g.and_then(|g| owners.get(&g)).cloned().unwrap_or_default();
                let list = lists
                    .entry((rig.clone(), target.clone()))
                    .or_insert_with(|| dof_list(all, &rig, &target).unwrap_or_default());
                let table = lookup(anim.info().channel_to_dof).map(|(sc, o)| ChannelToDof::parse(sc, o)).transpose()?;
                let drives = table
                    .map(|t| {
                        t.indices
                            .iter()
                            .map(|&d| list.get(usize::from(d)).map_or(format!("?{d}"), |x| x.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let clip = DiscClip {
                    id: id.clone(),
                    bundle: stem.clone(),
                    guid: n.guid,
                    controller: n.controller.clone(),
                    target,
                    rig,
                    num_ticks: f("NumTicks").unwrap_or(0.0),
                    fps_scale: f("FPSScale").unwrap_or(1.0),
                    end_frame: anim.info().end_frame,
                    additive: anim.info().additive,
                    drives,
                    canonical: out.by_guid.get(&n.guid).cloned(),
                };
                out.by_guid.entry(n.guid).or_insert_with(|| id.clone());
                out.clips.insert(id, clip);
            }
        }
        Ok(out)
    }

    pub fn by_guid(&self, guid: Guid) -> Option<&DiscClip> {
        self.clips.get(self.by_guid.get(&guid)?)
    }

    pub fn aliases(&self, guid: Guid) -> Vec<&ContentId> {
        let mut v: Vec<&DiscClip> = self.clips.values().filter(|c| c.guid == guid).collect();
        v.sort_by_key(|c| c.canonical.is_some());
        v.into_iter().map(|c| &c.id).collect()
    }

    pub fn counts(&self) -> (usize, usize, usize) {
        let guids: BTreeSet<Guid> = self.clips.values().map(|c| c.guid).collect();
        let offered = self.clips.values().filter(|c| c.not_offered().is_none()).count();
        (self.clips.len(), guids.len(), offered)
    }
}

fn rig_owners(all: &Stream) -> HashMap<Guid, String> {
    let sc = &all.schema;
    let idx = all.guid_index();
    let guids = |o: &Object, f: &str| -> Vec<Guid> {
        o.get(sc, f)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_guid).collect())
            .unwrap_or_default()
    };
    let mut owner = HashMap::new();
    for r in all.assets_of_class("RigAsset") {
        let rig = r.object.name(sc).unwrap_or("").to_string();
        let mut queue = guids(&r.object, "DofSetLists");
        let mut seen = BTreeSet::new();
        while let Some(g) = queue.pop() {
            if !seen.insert(g) {
                continue;
            }
            owner.entry(g).or_insert_with(|| rig.clone());
            if let Some(&i) = idx.get(&g) {
                queue.extend(guids(&all.assets[i].object, "Children"));
            }
        }
    }
    owner
}
