use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use ssx_content::BASE_NAMESPACE;
use ssx_content::check::{Checked, Severity};

use super::check::push;
use crate::anim_clip::{Replaced, bind};
use crate::content::GameContent;

const TICKS: f32 = 60.0;

pub(super) fn check(c: &mut Checked, big: &Path) {
    let clips: Vec<_> = c.anims.iter().map(|(id, (f, clip))| (id.clone(), f.clone(), clip.clone())).collect();
    if clips.is_empty() {
        return;
    }
    let content = GameContent::of(big);
    let Some(disc) = content.disc_anims() else {
        push(c, Severity::Error, "", (0, 0), "the disc's animation clips cannot be read: clips are not checked".into());
        return;
    };
    let skeleton = match content.riders.skeletons.get(big, None) {
        Ok(s) => s,
        Err(e) => {
            push(
                c,
                Severity::Error,
                "",
                (0, 0),
                format!("the riders' skeleton cannot be read ({e:#}): clips are not checked"),
            );
            return;
        }
    };
    for (id, file, clip) in clips {
        if id.namespace() != BASE_NAMESPACE {
            if c.tricks.values().any(|(_, t)| t.clips.contains(&id)) {
                continue;
            }
            let message = format!(
                "{id}: a clip of the package's own is played only by an added trick of the package (`trick/…`, its \
                 [trick.clips]), and none names it — a package replaces the disc's clips by their ids (`ssx:anim/…`, \
                 `ssx-assets content ids --kind anim`)"
            );
            push(c, Severity::Warning, &file, (0, 0), message);
            continue;
        }
        let Some(dc) = disc.clips.get(&id) else { continue };
        if let Some(why) = dc.not_offered() {
            push(c, Severity::Error, &file, (0, 0), why);
            continue;
        }
        let replaced =
            Replaced { id: &dc.id, end_frame: dc.end_frame, additive: dc.additive, frame_rate: dc.frame_rate() };
        let m = match bind(Arc::new(clip.clone()), "", replaced, &disc.dofs, &skeleton.rig) {
            Ok(m) => m,
            Err(why) => {
                push(c, Severity::Error, &file, (0, 0), format!("{id}: {why}"));
                continue;
            }
        };
        let ours: BTreeSet<&str> =
            m.channel_to_dof.iter().filter_map(|&d| disc.dofs.get(usize::from(d))).map(|d| d.name.as_str()).collect();
        let left: Vec<&str> = dc.drives.iter().map(String::as_str).filter(|n| !ours.contains(n)).collect();
        if !left.is_empty() {
            let (severity, why) = match dc.additive {
                false => (Severity::Warning, "the pose of the layers below shows through on them"),
                true => (Severity::Note, "an additive clip adds nothing to them"),
            };
            let shown: Vec<&str> = left.iter().take(12).copied().collect();
            let more = if left.len() > shown.len() {
                format!(" and {} more", left.len() - shown.len())
            } else {
                String::new()
            };
            let message = format!(
                "{id}: drives {} of the {} DOFs the disc clip drives; not {}{more}: {why}",
                dc.drives.len() - left.len(),
                dc.drives.len(),
                shown.join(", ")
            );
            push(c, severity, &file, (0, 0), message);
        }
        let length = dc.num_ticks / TICKS;
        let tick = 1.0 / TICKS;
        if clip.duration + tick < length {
            let message = format!(
                "{id}: {:.3} s long; its controller plays {:.3} s ({} ticks): it holds its last pose for {:.3} s",
                clip.duration,
                length,
                dc.num_ticks,
                length - clip.duration
            );
            push(c, Severity::Note, &file, (0, 0), message);
        } else if clip.duration > length + tick {
            let message = format!(
                "{id}: {:.3} s long; its controller plays {:.3} s ({} ticks, the graph's timing): the last {:.3} s are \
                 never shown",
                clip.duration,
                length,
                dc.num_ticks,
                clip.duration - length
            );
            push(c, Severity::Warning, &file, (0, 0), message);
        }
        let aliases: Vec<String> = disc.aliases(dc.guid).iter().skip(1).map(|a| a.to_string()).collect();
        let message = format!(
            "{id}: bound to the riders' rig ({} channels){}; simulation content: the sim content id of a session \
             changes with it",
            m.channel_to_dof.len(),
            if aliases.is_empty() { String::new() } else { format!(", also in place of {}", aliases.join(", ")) }
        );
        push(c, Severity::Note, &file, (0, 0), message);
    }
}
