use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use ssx_formats::ant::Stream;
use ssx_formats::ant_anim::{Animation, ChannelToDof, VbrBlock, dof_list, nlerp};
use ssx_formats::crsf::{Mat4, SceneGraph};

use crate::character::{Srt, mat_mul, model_matrix};

pub const ANIM_DIR: &str = "cache0/data/common/anim";
pub const RIG_BUNDLE: &str = "anim_all.cba";
pub const FPS: f32 = 60.0;
pub const RIDE_CLIPS: [&str; 7] = [
    "carv_regular_lean",
    "carv_tucked_lean",
    "carv_regular_jump",
    "jump_ride_fwd",
    "air_noise_upright_loop",
    "air_noise_upright_tucked_loop",
    "ride_get_going_fwd",
];

#[derive(Clone, Debug)]
pub struct RigClip {
    pub name: String,
    pub codec: &'static str,
    pub end_frame: u16,
    pub additive: bool,
    pub driven: Vec<usize>,
    anim: Animation,
    bindings: Vec<Binding>,
    floats: Vec<(usize, usize)>,
    pub unbound: usize,
    cache: BlockCache,
}

#[derive(Default)]
struct BlockCache(Mutex<Blocks>);

#[derive(Default)]
struct Blocks {
    vbr: Vec<(usize, Arc<VbrBlock>)>,
    dct: Vec<(usize, Arc<DctBlock>)>,
}

type DctBlock = Vec<Vec<[f32; 4]>>;

const CACHED_BLOCKS: usize = 4;

impl Clone for BlockCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for BlockCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BlockCache")
    }
}

fn cached<T>(
    list: &mut Vec<(usize, Arc<T>)>,
    block: usize,
    decode: impl FnOnce(usize) -> ssx_formats::Result<T>,
) -> ssx_formats::Result<Arc<T>> {
    if let Some(i) = list.iter().position(|e| e.0 == block) {
        let e = list.remove(i);
        list.insert(0, e);
    } else {
        list.insert(0, (block, Arc::new(decode(block)?)));
        list.truncate(CACHED_BLOCKS);
    }
    Ok(list[0].1.clone())
}

#[derive(Clone, Debug, PartialEq)]
pub struct RigPose {
    pub nodes: Vec<Srt>,
    pub floats: Vec<f32>,
}

impl RigPose {
    pub fn blend(&mut self, other: &RigPose, t: f32) {
        blend_into(&mut self.nodes, &other.nodes, t);
        for (a, b) in self.floats.iter_mut().zip(&other.floats) {
            *a += (b - *a) * t;
        }
    }
}

impl RigClip {
    pub fn duration(&self) -> f32 {
        f32::from(self.end_frame) / FPS
    }

    pub fn float_slots(&self) -> Vec<usize> {
        self.floats.iter().map(|f| f.0).collect()
    }

    fn sample(&self, frame: f32) -> ssx_formats::Result<ssx_formats::ant_anim::Pose> {
        let mut c = self.cache.0.lock().unwrap_or_else(|e| e.into_inner());
        match &self.anim {
            Animation::Vbr(a) => {
                let (k0, k1, t) = a.keys_at(frame);
                let ((b0, r0), (b1, r1)) = (a.key_location(k0), a.key_location(k1));
                let x = cached(&mut c.vbr, b0, |b| a.decode_block(b))?;
                let y = if b1 == b0 { x.clone() } else { cached(&mut c.vbr, b1, |b| a.decode_block(b))? };
                drop(c);
                a.assemble(&x.rows[r0], Some(&y.rows[r1]), t)
            }
            Animation::Dct(a) => {
                let (k0, k1, t) = a.keys_at(frame);
                let x = cached(&mut c.dct, k0 / 8, |b| a.decode_block(b))?;
                let y = cached(&mut c.dct, k1 / 8, |b| a.decode_block(b))?;
                drop(c);
                Ok(a.pose_from(&x[k0 % 8], &y[k1 % 8], t))
            }
            other => {
                drop(c);
                other.sample(frame)
            }
        }
    }

    pub fn sample_into(&self, frame: f32, pose: &mut [Srt]) {
        if let Ok(p) = self.sample(frame) {
            self.apply(&p, pose);
        }
    }

    fn apply(&self, p: &ssx_formats::ant_anim::Pose, pose: &mut [Srt]) {
        for b in &self.bindings {
            let srt = &mut pose[b.node];
            match b.kind {
                'q' => srt.rotation = p.quats[b.slot],
                't' => srt.translation = p.vec3s[b.slot],
                _ => srt.scale = p.vec3s[b.slot],
            }
        }
    }

    pub fn sample_pose_into(&self, frame: f32, pose: &mut RigPose) {
        let Ok(p) = self.sample(frame) else { return };
        self.apply(&p, &mut pose.nodes);
        for &(slot, k) in &self.floats {
            if let Some(v) = pose.floats.get_mut(slot) {
                *v = p.floats[k];
            }
        }
    }

    pub fn sample_anim_into(&self, frame: f32, pose: &mut crate::anim_pose::AnimPose) {
        use crate::anim_pose::{VALID_Q, VALID_S, VALID_T};
        let Ok(p) = self.sample(frame) else { return };
        self.apply(&p, &mut pose.pose.nodes);
        for b in &self.bindings {
            pose.valid_nodes[b.node] |= match b.kind {
                'q' => VALID_Q,
                't' => VALID_T,
                _ => VALID_S,
            };
        }
        for &(slot, k) in &self.floats {
            if let Some(v) = pose.pose.floats.get_mut(slot) {
                *v = p.floats[k];
                pose.valid_floats[slot] = true;
            }
        }
    }

    pub fn frame_at(&self, seconds: f32, looping: bool) -> f32 {
        let f = seconds * FPS;
        let end = f32::from(self.end_frame);
        if looping && end > 0.0 { f.rem_euclid(end) } else { f.clamp(0.0, end) }
    }
}

pub fn lerp_srt(a: &Srt, b: &Srt, t: f32) -> Srt {
    let lerp3 = |x: [f32; 3], y: [f32; 3]| std::array::from_fn(|k| x[k] + (y[k] - x[k]) * t);
    Srt {
        scale: lerp3(a.scale, b.scale),
        rotation: nlerp(a.rotation, b.rotation, t),
        translation: lerp3(a.translation, b.translation),
    }
}

pub fn blend_into(acc: &mut [Srt], other: &[Srt], t: f32) {
    for (a, b) in acc.iter_mut().zip(other) {
        *a = lerp_srt(a, b, t);
    }
}

fn codec_name(a: &Animation) -> &'static str {
    match a {
        Animation::Vbr(_) => "VBR",
        Animation::Dct(_) => "DCT",
        Animation::Curve(_) => "curve",
        Animation::Raw(_) => "raw",
        Animation::Frame(_) => "frame",
    }
}

pub fn load_clips(big_root: &Path, rig: &SceneGraph, bind: &[Srt], names: &[&str]) -> Result<Vec<RigClip>> {
    if bind.len() != rig.nodes.len() {
        bail!("rest pose has {} nodes, the rig {}", bind.len(), rig.nodes.len());
    }
    let path = big_root.join(ANIM_DIR).join(RIG_BUNDLE);
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let stream = Stream::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let dofs = dof_list(&stream, "genchar_rig", "AnimRig").context("genchar_rig AnimRig DOF list")?;
    let mut tables = HashMap::new();
    for a in stream.assets_of_class(ChannelToDof::CLASS) {
        if let Some(g) = a.object.guid(&stream.schema) {
            tables.insert(g, ChannelToDof::parse(&stream.schema, &a.object)?);
        }
    }
    let mut out = Vec::with_capacity(names.len());
    for &name in names {
        let anim = stream
            .assets
            .iter()
            .filter(|a| a.object.name(&stream.schema).is_some_and(|n| n.strip_suffix(" Anim").unwrap_or(n) == name))
            .find_map(|a| Animation::from_object(&stream.schema, &a.object).transpose())
            .with_context(|| format!("no animation {name} in {RIG_BUNDLE}"))?
            .with_context(|| format!("decoding {name}"))?;
        let table = tables
            .get(&anim.info().channel_to_dof)
            .with_context(|| format!("{name}: its ChannelToDof table is not in {RIG_BUNDLE}"))?;
        out.push(bind_clip(name, &anim, table, &dofs, rig)?);
    }
    Ok(out)
}

pub type LoadedClips = (Vec<RigClip>, HashMap<ssx_formats::ant::Guid, usize>, Vec<String>);

pub fn load_clips_db(
    db: &crate::anim_db::AssetDb,
    rig: &SceneGraph,
    clips: &[(ssx_formats::ant::Guid, String)],
) -> Result<LoadedClips> {
    let (_, all) = db.streams.iter().find(|(n, _)| n == RIG_BUNDLE).context("anim_all.cba not loaded")?;
    let dofs = dof_list(all, "genchar_rig", "AnimRig").context("genchar_rig AnimRig DOF list")?;
    let mut tables = HashMap::new();
    for a in db.all_of_class(ChannelToDof::CLASS) {
        tables.insert(a.guid, ChannelToDof::parse(a.schema, a.obj)?);
    }
    let (mut out, mut index, mut problems) = (Vec::new(), HashMap::new(), Vec::new());
    for (g, name) in clips {
        let Some(a) = db.get(*g) else {
            problems.push(format!("{name}: animation not loaded"));
            continue;
        };
        let bound = Animation::from_object(a.schema, a.obj)
            .map_err(anyhow::Error::from)
            .and_then(|x| x.with_context(|| format!("{name}: not an animation")))
            .and_then(|anim| {
                let table =
                    tables.get(&anim.info().channel_to_dof).with_context(|| format!("{name}: no channel table"))?;
                bind_clip(name, &anim, table, &dofs, rig)
            });
        match bound {
            Ok(c) => {
                index.insert(*g, out.len());
                out.push(c);
            }
            Err(e) => problems.push(format!("{e:#}")),
        }
    }
    Ok((out, index, problems))
}

#[derive(Clone, Copy, Debug)]
struct Binding {
    node: usize,
    kind: char,
    slot: usize,
}

fn bind_clip(
    name: &str,
    anim: &Animation,
    table: &ChannelToDof,
    dofs: &[ssx_formats::ant_anim::Dof],
    rig: &SceneGraph,
) -> Result<RigClip> {
    let counts = anim.counts();
    if table.indices.len() != counts.total() {
        bail!("{name}: {} channels, its table has {}", counts.total(), table.indices.len());
    }
    let mut bindings = Vec::new();
    let mut unbound = 0;
    let float_list = float_dof_names(dofs);
    let mut floats: Vec<(usize, usize)> = Vec::new();
    for (c, &d) in table.indices.iter().enumerate() {
        if c >= counts.quats + counts.vec3s {
            let name = dofs.get(usize::from(d)).filter(|dof| dof.ty == ssx_formats::ant_anim::DofType::Float);
            match name.and_then(|dof| float_list.iter().position(|n| *n == format!("{}.{}", dof.layout, dof.name))) {
                Some(slot) => floats.push((slot, c - counts.quats - counts.vec3s)),
                None => unbound += 1,
            }
            continue;
        }
        let joint = dofs.get(usize::from(d)).and_then(|dof| dof.joint());
        let node = joint.and_then(|(j, _)| rig.find(j));
        let binding = match (joint, node) {
            (Some((_, 'q')), Some(node)) if c < counts.quats => Some(Binding { node, kind: 'q', slot: c }),
            (Some((_, k @ ('t' | 's'))), Some(node)) if (counts.quats..counts.quats + counts.vec3s).contains(&c) => {
                Some(Binding { node, kind: k, slot: c - counts.quats })
            }
            _ => None,
        };
        match binding {
            Some(b) => bindings.push(b),
            None => unbound += 1,
        }
    }
    let mut driven: Vec<usize> = bindings.iter().map(|b| b.node).collect();
    driven.sort_unstable();
    driven.dedup();
    let info = anim.info();
    for f in [0.0, f32::from(info.end_frame)] {
        anim.sample(f).with_context(|| format!("{name}: frame {f}"))?;
    }
    Ok(RigClip {
        name: name.to_string(),
        codec: codec_name(anim),
        end_frame: info.end_frame,
        additive: info.additive,
        driven,
        anim: anim.clone(),
        bindings,
        floats,
        unbound,
        cache: BlockCache::default(),
    })
}

pub fn float_dof_names(dofs: &[ssx_formats::ant_anim::Dof]) -> Vec<String> {
    dofs.iter()
        .filter(|d| d.ty == ssx_formats::ant_anim::DofType::Float)
        .map(|d| format!("{}.{}", d.layout, d.name))
        .collect()
}

pub fn float_dofs(big_root: &Path) -> Result<Vec<String>> {
    let path = big_root.join(ANIM_DIR).join(RIG_BUNDLE);
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let stream = Stream::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    Ok(float_dof_names(&dof_list(&stream, "genchar_rig", "AnimRig")?))
}

pub fn inverse_rigid(m: &Mat4) -> Mat4 {
    let r: [[f32; 3]; 3] = std::array::from_fn(|i| std::array::from_fn(|j| m[j][i]));
    let t: [f32; 3] = std::array::from_fn(|j| -(0..3).map(|k| m[3][k] * r[k][j]).sum::<f32>());
    [
        [r[0][0], r[0][1], r[0][2], 0.0],
        [r[1][0], r[1][1], r[1][2], 0.0],
        [r[2][0], r[2][1], r[2][2], 0.0],
        [t[0], t[1], t[2], 1.0],
    ]
}

pub fn ground_roll_table(clip: &RigClip, rig: &SceneGraph, bind: &[Srt], board: usize, ground: usize) -> Vec<f32> {
    let mut pose = bind.to_vec();
    (0..=usize::from(clip.end_frame))
        .map(|f| {
            clip.sample_into(f as f32, &mut pose);
            let rel = mat_mul(&model_matrix(rig, &pose, ground), &board_space(rig, bind, &pose, board));
            rel[1][2].atan2(rel[1][1]).to_degrees()
        })
        .collect()
}

pub fn board_space(rig: &SceneGraph, bind: &[Srt], pose: &[Srt], board: usize) -> Mat4 {
    mat_mul(&inverse_rigid(&model_matrix(rig, pose, board)), &model_matrix(rig, bind, board))
}

pub fn frame_for_roll(table: &[f32], roll: f32) -> f32 {
    for (i, w) in table.windows(2).enumerate() {
        let (a, b) = (w[0], w[1]);
        if (a - roll) * (b - roll) <= 0.0 && a != b {
            return i as f32 + (roll - a) / (b - a);
        }
    }
    let nearest = |x: &f32| (x - roll).abs();
    match (table.first(), table.last()) {
        (Some(first), Some(last)) if nearest(last) < nearest(first) => (table.len() - 1) as f32,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_lookup() {
        let table = [90.0, 45.0, 0.0, -45.0, -90.0];
        assert_eq!(frame_for_roll(&table, 0.0), 2.0);
        assert!((frame_for_roll(&table, 22.5) - 1.5).abs() < 1e-6);
        assert_eq!(frame_for_roll(&table, 120.0), 0.0);
        assert_eq!(frame_for_roll(&table, -120.0), 4.0);
    }
}
