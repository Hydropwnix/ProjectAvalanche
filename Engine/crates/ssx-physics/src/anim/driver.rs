use glam::{Quat, Vec3};
use ssx_assets::anim_eval::DriverEvent;
use ssx_assets::anim_graph::DriverTag;
use ssx_assets::tags::TagEvent;

const MIN_REMAINING: f32 = 1.525_878_9e-5;

#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentDriver {
    pub tag: DriverTag,
    pub remaining: f32,
    pub source: Option<(Quat, Vec3)>,
}

impl AttachmentDriver {
    pub fn new(tag: DriverTag) -> Self {
        let remaining = tag.blend_in;
        Self { tag, remaining, source: None }
    }

    pub fn on_events(slot: &mut Option<AttachmentDriver>, events: &[DriverEvent]) {
        let mut kept = false;
        for e in events {
            match e.event {
                TagEvent::Enter => {
                    *slot = Some(Self::new(e.tag.clone()));
                    kept = true;
                }
                TagEvent::Apply(_) => kept |= slot.as_ref().is_some_and(|d| d.tag == e.tag),
                TagEvent::Leave(_) => {}
            }
        }
        if !kept {
            *slot = None;
        }
    }

    pub fn update(&mut self, root: (Quat, Vec3), target: (Quat, Vec3), dt: f32) -> Option<(Quat, Vec3)> {
        let (qs, ts) = self.source?;
        let rem = self.remaining;
        let (span, left) = if rem - dt >= 0.0 { (rem, rem - dt) } else { (dt, 0.0) };
        self.remaining = left;
        let f = dt / span.max(MIN_REMAINING);
        let (q, p) = root;
        let offset_q = Quat::from_array(self.tag.offset_q).normalize();
        let (qd, pd) = (target.0 * offset_q, target.1 + target.0 * Vec3::from_array(self.tag.offset_t));
        let x = (q.conjugate() * qd * qs.conjugate()).normalize();
        let id = Quat::IDENTITY;
        let c = if x.w > 0.0 { id + (x - id) * f } else { id - (x + id) * f };
        let c = c.normalize();
        let source_full = p + q * (x * ts);
        Some(((q * c).normalize(), p + (pd - source_full) * f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(blend_in: f32) -> DriverTag {
        DriverTag {
            window: ssx_assets::tags::TagWindow::new(0.0, 42.0, false),
            target: "ID_TargetAnimatable".into(),
            source_template: "Ground".into(),
            source_joint: Some("Ground".into()),
            target_template: "Heli_Floor".into(),
            target_joints: vec![("helicopter_rig".into(), "Floor".into())],
            offset_t: [0.0; 3],
            offset_q: [0.0, 0.0, 0.0, 1.0],
            blend_in,
        }
    }

    #[test]
    fn no_blend_in_places_the_source_on_the_target() {
        let mut d = AttachmentDriver::new(tag(0.0));
        d.source = Some((Quat::from_rotation_x(0.3), Vec3::new(0.1, -0.9, 0.2)));
        let root = (Quat::from_rotation_y(1.0), Vec3::new(5.0, 2.0, -3.0));
        let target = (Quat::from_rotation_z(0.4) * Quat::from_rotation_y(-0.7), Vec3::new(-1.0, 7.0, 4.0));
        let (q, p) = d.update(root, target, 2.0).unwrap();
        let (qs, ts) = d.source.unwrap();
        assert!((q * qs).angle_between(target.0) < 1e-4);
        assert!((p + q * ts).distance(target.1) < 1e-4);
    }

    #[test]
    fn the_blend_in_closes_the_gap_linearly() {
        let mut d = AttachmentDriver::new(tag(20.0));
        d.source = Some((Quat::IDENTITY, Vec3::new(0.0, -1.0, 0.0)));
        let target = (Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0));
        let mut root = (Quat::IDENTITY, Vec3::new(0.0, 1.0, 0.0));
        for k in 1..=10 {
            root = d.update(root, target, 2.0).unwrap();
            let src = root.1 + Vec3::new(0.0, -1.0, 0.0);
            assert!((src.x - k as f32).abs() < 1e-4, "update {k}: {src}");
        }
        assert_eq!(d.remaining, 0.0);
        root = d.update(root, target, 2.0).unwrap();
        assert!((root.1 - Vec3::new(10.0, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn the_tag_callbacks_drive_the_slot() {
        let t = tag(12.0);
        let ev = |event| DriverEvent { tag: t.clone(), event };
        let mut slot = None;
        AttachmentDriver::on_events(&mut slot, &[ev(TagEvent::Enter), ev(TagEvent::Apply(0.0))]);
        assert_eq!(slot.as_ref().map(|d| d.remaining), Some(12.0));
        slot.as_mut().unwrap().remaining = 4.0;
        AttachmentDriver::on_events(&mut slot, &[ev(TagEvent::Apply(0.5))]);
        assert_eq!(slot.as_ref().map(|d| d.remaining), Some(4.0));
        AttachmentDriver::on_events(&mut slot, &[ev(TagEvent::Leave(1.0))]);
        assert!(slot.is_none());
        AttachmentDriver::on_events(&mut slot, &[ev(TagEvent::Enter), ev(TagEvent::Apply(0.0))]);
        AttachmentDriver::on_events(&mut slot, &[]);
        assert!(slot.is_none());
    }
}
