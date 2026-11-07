use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ssx_content::check::{Checked, Severity};
use ssx_content::package::PackageSource;
use ssx_content::registry::Source;
use ssx_content::trick::{BASES, ROLES, TrickToml};
use ssx_content::{ContentId, Kind};

use super::check::push;
use crate::anim_clip::{Replaced, bind};
use crate::content::GameContent;
use crate::content::tricks::base_clip;

const TICKS: f32 = 60.0;

pub(super) fn check(c: &mut Checked, big: &Path, others: &[PathBuf]) {
    if c.tricks.is_empty() {
        return;
    }
    let tricks: Vec<_> = c.tricks.iter().map(|(id, (f, t))| (id.clone(), f.clone(), t.clone())).collect();
    taken_elsewhere(c, &tricks, others);
    let content = GameContent::of(big);
    let Some(disc) = content.disc_anims() else {
        push(c, Severity::Error, "", (0, 0), "the disc's animation clips cannot be read: tricks are not bound".into());
        return;
    };
    let skeleton = match content.riders.skeletons.get(big, None) {
        Ok(s) => s,
        Err(e) => {
            let message = format!("the riders' skeleton cannot be read ({e:#}): tricks are not bound");
            push(c, Severity::Error, "", (0, 0), message);
            return;
        }
    };
    for (id, file, t) in tricks {
        let (mut left, mut drives, mut lengths, mut bound) = (BTreeSet::new(), 0usize, Vec::new(), true);
        for (r, clip_id) in t.clips.iter().enumerate() {
            let role = ROLES[r].0;
            let Some((_, clip)) = c.anims.get(clip_id).cloned() else {
                bound = false;
                continue;
            };
            let Some(dc) = base_clip(&disc, &t, r) else {
                let message = format!("clips.{role}: the disc has no clip {} to stand in", t.base_clip(r));
                push(c, Severity::Error, &file, (0, 0), message);
                bound = false;
                continue;
            };
            let replaced =
                Replaced { id: &dc.id, end_frame: dc.end_frame, additive: dc.additive, frame_rate: dc.frame_rate() };
            let duration = clip.duration;
            let m = match bind(Arc::new(clip), "", replaced, &disc.dofs, &skeleton.rig) {
                Ok(m) => m,
                Err(why) => {
                    push(c, Severity::Error, &file, (0, 0), format!("clips.{role}: {clip_id}: {why}"));
                    bound = false;
                    continue;
                }
            };
            let ours: BTreeSet<&str> = m
                .channel_to_dof
                .iter()
                .filter_map(|&d| disc.dofs.get(usize::from(d)))
                .map(|d| d.name.as_str())
                .collect();
            drives = drives.max(dc.drives.len());
            left.extend(dc.drives.iter().filter(|n| !ours.contains(n.as_str())).cloned());
            let length = dc.num_ticks / TICKS;
            if (duration - length).abs() > 1.0 / TICKS {
                lengths.push(format!("{role} {duration:.3} s / {length:.3} s"));
            }
        }
        if !left.is_empty() {
            let shown: Vec<&str> = left.iter().take(12).map(String::as_str).collect();
            let more =
                if left.len() > shown.len() { format!(" and {} more", left.len() - shown.len()) } else { "".into() };
            let message = format!(
                "{id}: its clips leave {} of the up to {drives} DOFs the {}'s clips drive: {}{more}; the pose of the \
                 layers below shows through on them",
                left.len(),
                BASES[t.base].0,
                shown.join(", ")
            );
            push(c, Severity::Warning, &file, (0, 0), message);
        }
        if !lengths.is_empty() {
            let message = format!(
                "{id}: clip lengths against their controllers' (the graph's timing; a shorter clip holds its last \
                 pose, a longer one is cut): {}",
                lengths.join(", ")
            );
            push(c, Severity::Note, &file, (0, 0), message);
        }
        if bound {
            let message = format!(
                "{id}: its 12 clips bound to the riders' rig in the {}'s places; simulation content: the sim content id \
                 of a session changes with it",
                BASES[t.base].0
            );
            push(c, Severity::Note, &file, (0, 0), message);
        }
    }
}

fn taken_elsewhere(c: &mut Checked, tricks: &[(ContentId, String, TrickToml)], others: &[PathBuf]) {
    let name = c.manifest.as_ref().map(|m| m.resource.name.clone()).unwrap_or_default();
    let mut theirs: BTreeMap<ssx_content::trick::Input, Vec<String>> = BTreeMap::new();
    for dir in others {
        let Ok(p) = PackageSource::open(dir) else { continue };
        if p.manifest.resource.name == name {
            continue;
        }
        for id in p.manifest.add.keys().filter(|id| id.kind() == Kind::Trick) {
            if let Ok(a) = p.load(id)
                && let Some(t) = a.into_value::<TrickToml>()
            {
                theirs.entry(t.input).or_default().push(format!("{id} ({})", p.manifest.resource.name));
            }
        }
    }
    for (id, file, t) in tricks {
        if let Some(other) = theirs.get(&t.input) {
            let message = format!(
                "{id}: input {} is also {}'s: with both on, the stronger package's trick plays (the higher priority, \
                 then the later on the Mods page) and the other is passed over",
                t.input.name(),
                other.join(", ")
            );
            push(c, Severity::Warning, file, (0, 0), message);
        }
    }
}

pub(super) fn other_dirs(app_dir: &Path, with: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(app_dir.join(crate::content::mods::MODS_DIR))
        .map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect())
        .unwrap_or_default();
    out.extend(with.iter().cloned());
    out
}
