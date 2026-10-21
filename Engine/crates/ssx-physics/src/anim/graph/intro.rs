use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct StepStart {
    pub velocity: Vec3,
    pub wipeout: bool,
}

pub struct LevelStartGraph {
    pub(super) animator: Animator,
    pub(super) blend: ssx_assets::anim_graph::Blend,
    pub(super) running: bool,
    pub(super) elapsed: Option<f32>,
    pub(super) intro: Option<IntroGraph>,
}

pub struct IntroGraph {
    pub(super) ready: Animator,
    pub(super) jump: Animator,
    pub(super) loop_end: Option<f32>,
    pub(super) blend: ssx_assets::anim_graph::Blend,
    pub(super) in_jump: bool,
    pub(super) fade: Option<f32>,
    pub(super) ready_ticks: f32,
}

impl IntroGraph {
    pub fn load(
        big_root: &Path,
        nis: &ssx_assets::nis::IntroNis,
        rig: &SceneGraph,
        bind: Vec<Srt>,
        floats: &[String],
    ) -> Result<Self> {
        let dir = big_root.join(ssx_assets::rig_anim::ANIM_DIR);
        let db = ssx_assets::anim_db::AssetDb::load(&dir, &nis.bundles.iter().map(String::as_str).collect::<Vec<_>>())?;
        let ready = Animator::new(&db, &nis.ready.sequence, rig, bind.clone(), floats)?;
        let jump = Animator::new(&db, &nis.jump.sequence, rig, bind, floats)?;
        let blend = ssx_assets::anim_graph::Blend {
            class: "AccelBlendAsset".into(),
            blend_time: nis.transition_blend,
            launch_angle: 0.0,
            curve: -1,
            sync_phase: false,
            weight: None,
            samples: Vec::new(),
            dof_curves: None,
        };
        Ok(Self { ready, jump, loop_end: nis.ready.loop_end, blend, in_jump: false, fade: None, ready_ticks: 0.0 })
    }

    pub(super) fn current(&self) -> &Animator {
        if self.in_jump { &self.jump } else { &self.ready }
    }

    pub(super) fn reset(&mut self, random: &ssx_assets::anim_eval::RandomState) {
        self.ready.reset_with_random(random.clone());
        self.jump.reset_with_random(random.clone());
        (self.in_jump, self.fade, self.ready_ticks) = (false, None, 0.0);
    }

    pub(super) fn update(&mut self, scene: crate::nis::Scene, dt: f32) {
        match scene {
            crate::nis::Scene::Ready { .. } => {
                let prev = self.ready_ticks;
                self.ready_ticks += dt;
                match self.loop_end.filter(|&e| e > 0.0) {
                    Some(end) if self.ready_ticks.rem_euclid(end) < prev.rem_euclid(end) => {
                        let random = self.ready.gs.random.clone();
                        self.ready.reset_with_random(random);
                        self.ready.update(self.ready_ticks.rem_euclid(end));
                    }
                    _ => self.ready.update(dt),
                }
            }
            crate::nis::Scene::Jump { .. } => {
                if !self.in_jump {
                    (self.in_jump, self.fade) = (true, Some(0.0));
                    let random = self.ready.gs.random.clone();
                    self.jump.reset_with_random(random);
                }
                if let Some(e) = self.fade {
                    if self.blend.blend_time - e <= dt {
                        self.fade = None;
                    } else {
                        self.ready.update(dt);
                        self.fade = Some(e + dt);
                    }
                }
                self.jump.update(dt);
            }
        }
    }

    pub(super) fn anim_pose(&self) -> ssx_assets::anim_pose::AnimPose {
        match self.fade.filter(|_| self.in_jump) {
            Some(e) => {
                let mut old = self.ready.anim_pose();
                old.blend(&self.jump.anim_pose(), ssx_assets::anim_eval::blend_new_weight(&self.blend, e));
                old
            }
            None => self.current().anim_pose(),
        }
    }
}

impl LevelStartGraph {
    pub(super) fn current(&self) -> &Animator {
        match &self.intro {
            Some(i) => i.current(),
            None => &self.animator,
        }
    }

    pub(super) fn anim_pose(&self) -> ssx_assets::anim_pose::AnimPose {
        match &self.intro {
            Some(i) => i.anim_pose(),
            None => self.animator.anim_pose(),
        }
    }
}

impl LevelStartGraph {
    pub fn load(
        db: &ssx_assets::anim_db::AssetDb,
        rig: &SceneGraph,
        bind: Vec<Srt>,
        floats: &[String],
    ) -> Result<Self> {
        let animator = Animator::new(db, "SF_default_level_start", rig, bind, floats)?;
        let a = db.by_name("accel_medium").context("no accel_medium blend")?;
        anyhow::ensure!(a.class() == "AccelBlendAsset", "accel_medium is a {}", a.class());
        let blend = ssx_assets::anim_graph::Blend {
            class: a.class().to_string(),
            blend_time: a.f32("BlendTime").context("accel_medium has no BlendTime")?,
            launch_angle: 0.0,
            curve: -1,
            sync_phase: false,
            weight: None,
            samples: Vec::new(),
            dof_curves: None,
        };
        Ok(Self { animator, blend, running: false, elapsed: None, intro: None })
    }
}
