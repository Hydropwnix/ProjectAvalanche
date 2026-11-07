use std::sync::Arc;

use ssx_content::ContentId;
use ssx_content::anim::{Clip, Path as TrackPath};
use ssx_formats::ant_anim::{Animation, ChannelCounts, Dof, DofType, Pose};
use ssx_formats::crsf::SceneGraph;

#[derive(Clone, Debug, PartialEq)]
pub enum ClipData {
    Disc(Box<Animation>),
    Mod(Arc<ModClip>),
}

impl ClipData {
    pub fn sample(&self, frame: f32) -> ssx_formats::Result<Pose> {
        match self {
            ClipData::Disc(a) => a.sample(frame),
            ClipData::Mod(m) => Ok(m.sample(frame)),
        }
    }

    pub fn counts(&self) -> ChannelCounts {
        match self {
            ClipData::Disc(a) => a.counts(),
            ClipData::Mod(m) => m.counts(),
        }
    }

    pub fn end_frame(&self) -> u16 {
        match self {
            ClipData::Disc(a) => a.info().end_frame,
            ClipData::Mod(m) => m.end_frame,
        }
    }

    pub fn additive(&self) -> bool {
        match self {
            ClipData::Disc(a) => a.info().additive,
            ClipData::Mod(m) => m.clip.additive,
        }
    }

    pub fn package(&self) -> Option<&ModClip> {
        match self {
            ClipData::Disc(_) => None,
            ClipData::Mod(m) => Some(m),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Replaced<'a> {
    pub id: &'a ContentId,
    pub end_frame: u16,
    pub additive: bool,
    pub frame_rate: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModClip {
    pub id: ContentId,
    pub source: String,
    pub clip: Arc<Clip>,
    quats: Vec<usize>,
    vec3s: Vec<usize>,
    floats: Vec<usize>,
    pub channel_to_dof: Vec<u16>,
    frame_rate: f32,
    end_frame: u16,
}

impl ModClip {
    pub fn counts(&self) -> ChannelCounts {
        ChannelCounts { quats: self.quats.len(), vec3s: self.vec3s.len(), floats: self.floats.len() }
    }

    pub fn sample(&self, frame: f32) -> Pose {
        let seconds = frame / self.frame_rate;
        let quats = self.quats.iter().map(|&j| self.clip.joint_at(j, seconds)).collect();
        let vec3s = self
            .vec3s
            .iter()
            .map(|&j| {
                let v = self.clip.joint_at(j, seconds);
                [v[0], v[1], v[2]]
            })
            .collect();
        let floats = self.floats.iter().map(|&f| self.clip.float_at(f, seconds)).collect();
        Pose { quats, vec3s, floats }
    }
}

pub fn float_dof_name(d: &Dof) -> String {
    format!("{}.{}", d.layout, d.name)
}

pub fn bind(
    clip: Arc<Clip>,
    source: &str,
    replaced: Replaced<'_>,
    dofs: &[Dof],
    skeleton: &SceneGraph,
) -> Result<ModClip, String> {
    if clip.additive != replaced.additive {
        let is = |a: bool| if a { "additive" } else { "not additive" };
        return Err(format!(
            "additive = {}: the clip it replaces ({}) is {} (the game binds a clip by it)",
            clip.additive,
            replaced.id,
            is(replaced.additive)
        ));
    }
    if !(replaced.frame_rate.is_finite() && replaced.frame_rate > 0.0) {
        return Err(format!("{}: its controller's frame rate {} cannot time a clip", replaced.id, replaced.frame_rate));
    }
    let mut quats: Vec<(u16, usize)> = Vec::new();
    let mut vec3s: Vec<(u16, usize)> = Vec::new();
    for (i, t) in clip.joints.iter().enumerate() {
        let (suffix, ty) = match t.path {
            TrackPath::Rotation => ("q", DofType::Quaternion),
            TrackPath::Translation => ("t", DofType::Translation),
            TrackPath::Scale => ("s", DofType::Scale),
        };
        let what = format!("joint {:?} {}", t.joint, t.path.name());
        let name = format!("{}.{suffix}", t.joint);
        let Some(d) = dofs.iter().position(|d| d.name == name && d.ty == ty) else {
            let has: Vec<&str> =
                dofs.iter().filter_map(|d| d.joint().filter(|j| j.0 == t.joint).map(|_| &*d.name)).collect();
            return Err(match has.is_empty() {
                true => format!("{what}: no DOF of the riders' rig is a joint of that name"),
                false => format!("{what}: the rig drives {} of this joint, not its {}", has.join(", "), t.path.name()),
            });
        };
        let Some(node) = skeleton.find(&t.joint) else {
            return Err(format!("{what}: the riders' skeleton has no node of that name"));
        };
        let parent = skeleton.nodes[node].parent.and_then(|p| skeleton.nodes.get(p)).map(|p| p.name.as_str());
        if t.parent.as_deref() != parent {
            return Err(format!(
                "{what}: its parent node in the glTF is {}, the skeleton's is {} (keys are local to the parent)",
                t.parent.as_deref().map_or("none".into(), |p| format!("{p:?}")),
                parent.map_or("none".into(), |p| format!("{p:?}")),
            ));
        }
        let d = u16::try_from(d).map_err(|_| format!("{what}: DOF index {d} past the table's range"))?;
        match t.path {
            TrackPath::Rotation => quats.push((d, i)),
            _ => vec3s.push((d, i)),
        }
    }
    let mut floats: Vec<(u16, usize)> = Vec::new();
    for (i, f) in clip.floats.iter().enumerate() {
        let Some(d) = dofs.iter().position(|d| d.ty == DofType::Float && float_dof_name(d) == f.dof) else {
            return Err(format!(
                "[[anim.float]] {}: dof {} is no float DOF of the riders' rig (`<layout>.<name>`, e.g. \
                 \"TrajectoryVelocityRig.AITrajectory.height\", \"FingerPoser.l_index_root\")",
                i + 1,
                ssx_core::text::quoted(&f.dof)
            ));
        };
        let d =
            u16::try_from(d).map_err(|_| format!("[[anim.float]] {}: DOF index {d} past the table's range", i + 1))?;
        floats.push((d, i));
    }
    for group in [&mut quats, &mut vec3s, &mut floats] {
        group.sort_unstable();
    }
    let channel_to_dof = quats.iter().chain(&vec3s).chain(&floats).map(|&(d, _)| d).collect();
    let tracks = |g: Vec<(u16, usize)>| g.into_iter().map(|(_, i)| i).collect::<Vec<_>>();
    Ok(ModClip {
        id: replaced.id.clone(),
        source: source.to_string(),
        clip,
        quats: tracks(quats),
        vec3s: tracks(vec3s),
        floats: tracks(floats),
        channel_to_dof,
        frame_rate: replaced.frame_rate,
        end_frame: replaced.end_frame,
    })
}

#[cfg(test)]
mod tests;
