use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_content::ContentId;
use ssx_content::anim::{Clip, FloatTrack, Interpolation, JointTrack, Keys, MAX_KEYS, Path as TrackPath};
use ssx_content::model::write_skinned::WriteJoint;
use ssx_formats::ant::Stream;
use ssx_formats::ant_anim::{Animation, ChannelToDof, DofType};

use super::DiscClip;
use crate::anim_clip::float_dof_name;
use crate::content::GameContent;

pub fn disc_animation(big: &Path, dc: &DiscClip) -> Result<(Animation, ChannelToDof)> {
    let dir = big.join(crate::rig_anim::ANIM_DIR);
    let read = |name: &str| -> Result<Stream> {
        let p = dir.join(format!("{name}.cba"));
        Stream::parse(&std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?)
            .with_context(|| format!("parsing {}", p.display()))
    };
    let find = |s: &Stream, g| s.guid_index().get(&g).map(|&i| s.assets[i].object.clone());
    let s = read(&dc.bundle)?;
    let obj = find(&s, dc.guid).with_context(|| format!("{}: its asset is not in {}.cba", dc.id, dc.bundle))?;
    let anim = Animation::from_object(&s.schema, &obj)?.with_context(|| format!("{}: not an animation", dc.id))?;
    let g = anim.info().channel_to_dof;
    let table = match find(&s, g) {
        Some(t) => ChannelToDof::parse(&s.schema, &t)?,
        None => {
            let all = read("anim_all")?;
            let t = find(&all, g).with_context(|| format!("{}: no channel table", dc.id))?;
            ChannelToDof::parse(&all.schema, &t)?
        }
    };
    Ok((anim, table))
}

pub fn clip_of(content: &GameContent, dc: &DiscClip) -> Result<Clip> {
    let disc = content.disc_anims().context("the disc's clips cannot be read")?;
    let skeleton = content.riders.skeletons.get(content.big_root(), None)?;
    let (anim, table) = disc_animation(content.big_root(), dc)?;
    let counts = anim.counts();
    if table.indices.len() != counts.total() {
        bail!("{}: {} channels, its table has {}", dc.id, counts.total(), table.indices.len());
    }
    let frames: Vec<u32> = match &anim {
        Animation::Curve(_) => (0..=u32::from(anim.info().end_frame)).collect(),
        _ => anim.key_frames(),
    };
    if frames.len() > MAX_KEYS {
        bail!("{}: {} keys; a clip has at most {MAX_KEYS} a track", dc.id, frames.len());
    }
    let rate = dc.frame_rate();
    let times: Vec<f32> = frames.iter().map(|&f| f as f32 / rate).collect();
    let sampled = frames.iter().map(|&f| anim.sample(f as f32)).collect::<ssx_formats::Result<Vec<_>>>()?;
    let stored = match &anim {
        Animation::Curve(_) => sampled.clone(),
        _ => (0..frames.len()).map(|k| anim.stored_key(k)).collect::<ssx_formats::Result<Vec<_>>>()?,
    };
    let keys_of = |pick: &dyn Fn(&ssx_formats::ant_anim::Pose) -> Vec<f32>| -> Keys {
        let first = pick(&sampled[0]);
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        if sampled.iter().all(|p| bits(&pick(p)) == bits(&first)) {
            return Keys { interpolation: Interpolation::Linear, times: vec![times[0]], values: first };
        }
        Keys {
            interpolation: Interpolation::Linear,
            times: times.clone(),
            values: stored.iter().flat_map(pick).collect(),
        }
    };
    let mut joints = Vec::new();
    let mut floats = Vec::new();
    for (c, &d) in table.indices.iter().enumerate() {
        let dof = disc.dofs.get(usize::from(d)).with_context(|| format!("{}: channel {c} → DOF {d}", dc.id))?;
        if c < counts.quats + counts.vec3s {
            let Some((joint, k)) = dof.joint() else {
                bail!(
                    "{}: channel {c} drives {} {}, no joint: a glTF clip cannot carry it",
                    dc.id,
                    dof.layout,
                    dof.name
                )
            };
            let (path, keys) = if c < counts.quats {
                (TrackPath::Rotation, keys_of(&|p| p.quats[c].to_vec()))
            } else {
                let v = c - counts.quats;
                let path = if k == 's' { TrackPath::Scale } else { TrackPath::Translation };
                (path, keys_of(&|p| p.vec3s[v].to_vec()))
            };
            let node =
                skeleton.rig.find(joint).with_context(|| format!("{}: no joint {joint} in the skeleton", dc.id))?;
            let parent = skeleton.rig.nodes[node].parent.map(|p| skeleton.rig.nodes[p].name.clone());
            joints.push(JointTrack { joint: joint.to_string(), path, keys, parent });
        } else {
            if dof.ty != DofType::Float {
                bail!("{}: channel {c} drives {} {}, no float DOF", dc.id, dof.layout, dof.name);
            }
            let f = c - counts.quats - counts.vec3s;
            floats.push(FloatTrack { dof: float_dof_name(dof), keys: keys_of(&|p| vec![p.floats[f]]) });
        }
    }
    let duration = joints
        .iter()
        .map(|t| &t.keys)
        .chain(floats.iter().map(|t| &t.keys))
        .filter_map(|k| k.times.last().copied())
        .fold(0.0, f32::max);
    Ok(Clip { joints, floats, additive: anim.info().additive, duration })
}

pub fn skeleton_joints(content: &GameContent) -> Result<Vec<WriteJoint>> {
    let sk = content.riders.skeletons.get(content.big_root(), None)?;
    Ok(sk
        .rig
        .nodes
        .iter()
        .zip(&sk.bind)
        .map(|(n, b)| WriteJoint {
            name: n.name.clone(),
            parent: n.parent,
            translation: b.translation,
            rotation: b.rotation,
            scale: b.scale,
        })
        .collect())
}

pub fn export_anim(big: &Path, id: &ContentId, out: &Path, app_dir: &Path) -> Result<Vec<PathBuf>> {
    let content = GameContent::of(big);
    let disc = content.disc_anims().context("the disc's clips cannot be read")?;
    let dc = disc.clips.get(id).with_context(|| format!("{id}: no clip of the disc (`content ids --kind anim`)"))?;
    if let Some(why) = dc.not_offered() {
        bail!("{why}");
    }
    let clip = clip_of(&content, dc)?;
    let name = id.path().rsplit('/').next().unwrap_or(id.path());
    let gltf = format!("anims/{name}.glb");
    let mut files = Vec::new();
    if !clip.joints.is_empty() {
        let glb =
            ssx_content::anim::write::glb(&skeleton_joints(&content)?, name, &clip).map_err(anyhow::Error::msg)?;
        files.push((gltf.clone(), glb));
    }
    let header = format!(
        "# {id} from your own disc, for your own use (Docs/architecture/content.md §23.7): its controller {} plays it \
         over {} ticks; in resource.toml: [override] \"{id}\" = \"anims/{name}.toml\"\n",
        dc.controller, dc.num_ticks
    );
    let toml =
        ssx_content::anim::write::anim_toml(&clip, (!clip.joints.is_empty()).then_some(gltf.as_str()), name, &header);
    files.push((format!("anims/{name}.toml"), toml.into_bytes()));
    super::super::export_records::write_files(id, out, app_dir, files)
}
