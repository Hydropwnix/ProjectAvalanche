use super::*;

pub const GHOST_LAYERS: &str = "layers_ghost_rewind";

pub const GHOST_WIPEOUT: &str = "sf_ghost_wipeout";

const SRT_IDENTITY: Srt = Srt { scale: [1.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], translation: [0.0; 3] };

#[derive(Clone, Debug)]
pub struct GhostRig {
    pub rig: SceneGraph,
    pub bind: Vec<Srt>,
    pub trajectory: usize,
    pub roots: Vec<usize>,
    pub rotations: Vec<Option<usize>>,
    pub translations: Vec<Option<usize>>,
    pub board: Option<usize>,
}

impl GhostRig {
    pub fn new(big_root: &Path, model: &GhostModel) -> Result<Self> {
        Self::from_rig(big_root, &model.rig, &model.bind)
    }

    pub fn from_rig(big_root: &Path, rig: &SceneGraph, bind: &[Srt]) -> Result<Self> {
        let (rig, bind, _) = crate::anim_rigops::extend_character_rig(big_root, rig, bind)?;
        let trajectory = rig.find("AITrajectory").context("the ghost rig has no AITrajectory")?;
        let mut roots = Vec::new();
        let mut p = rig.nodes[trajectory].parent;
        while let Some(n) = p {
            roots.push(n);
            p = rig.nodes[n].parent;
        }
        let rotations = ROTATION_CHANNELS.iter().map(|n| rig.find(n)).collect();
        let translations = TRANSLATION_CHANNELS.iter().map(|n| rig.find(n)).collect();
        let board = rig.find("Board");
        Ok(Self { rig, bind, trajectory, roots, rotations, translations, board })
    }
}

pub const GHOST_IDLE: &str = "ghost_idle_loop_generic";

pub const BOARD_PARTICLE_DEFAULT: ([f32; 3], [f32; 4]) = ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]);

fn place_board_at_default(rig: &GhostRig, nodes: &mut [Srt], board: usize, root: [f32; 3]) {
    use crate::rigamate::{quat_mul, rotate};
    let mut chain = Vec::new();
    let mut n = rig.rig.nodes[board].parent;
    while let Some(i) = n {
        chain.push(i);
        n = rig.rig.nodes[i].parent;
    }
    let (mut s, mut q, mut t) = ([1.0f32; 3], [0.0, 0.0, 0.0, 1.0], root);
    for &i in chain.iter().rev() {
        let l = &nodes[i];
        let st = [s[0] * l.translation[0], s[1] * l.translation[1], s[2] * l.translation[2], 0.0];
        let r = rotate(q, st);
        t = [t[0] + r[0], t[1] + r[1], t[2] + r[2]];
        q = quat_mul(q, l.rotation);
        s = [s[0] * l.scale[0], s[1] * l.scale[1], s[2] * l.scale[2]];
    }
    let gs = [s[0] * nodes[board].scale[0], s[1] * nodes[board].scale[1], s[2] * nodes[board].scale[2]];
    let conj = [-q[0], -q[1], -q[2], q[3]];
    let (p0, q0) = BOARD_PARTICLE_DEFAULT;
    let d = [(p0[0] - t[0]) / gs[0], (p0[1] - t[1]) / gs[1], (p0[2] - t[2]) / gs[2], 0.0];
    let lt = rotate(conj, d);
    nodes[board] = Srt {
        scale: [1.0 / gs[0], 1.0 / gs[1], 1.0 / gs[2]],
        rotation: quat_mul(conj, q0),
        translation: [lt[0], lt[1], lt[2]],
    };
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhostSchemas {
    pub ghost_animatable: String,
    pub ghost_object: String,
    pub bot_animatable: String,
}

impl GhostSchemas {
    pub fn load(big_root: &Path) -> Result<Self> {
        let (schemas, _) = crate::avalanche_entities::batches(big_root, &[]).context("the schemas batch")?;
        let name = |schema: &str, kind: &str, setter: &str| -> Result<String> {
            schemas
                .schemas
                .get(schema)
                .with_context(|| format!("no schema {schema}"))?
                .factories
                .iter()
                .filter(|f| f.kind == kind)
                .flat_map(|f| &f.calls)
                .find(|(m, _)| m == setter)
                .and_then(|(_, a)| a.first().and_then(|a| a.str()).map(str::to_string))
                .with_context(|| format!("{schema}: no {kind} {setter}"))
        };
        Ok(GhostSchemas {
            ghost_animatable: name("GhostSnowboarderRef", "Animatable", "SetAnimatableName")?,
            ghost_object: name("GhostSnowboarderRef", "GameObjectRenderEntityRef", "SetGameObjectName")?,
            bot_animatable: name("BotSnowboarderRef", "Animatable", "SetAnimatableName")?,
        })
    }

    pub fn ghost_config(&self) -> String {
        format!("{}_config", self.ghost_animatable)
    }

    pub fn bot_actor(&self, character: &str) -> String {
        self.bot_animatable.replace("%g", character)
    }
}

pub fn config_rigamates(all: &ssx_formats::ant::Stream, config: &str) -> Result<[Option<String>; 2]> {
    config_rigamates_in(&[all], config)
}

pub fn config_rigamates_in(streams: &[&ssx_formats::ant::Stream], config: &str) -> Result<[Option<String>; 2]> {
    let indices: Vec<_> = streams.iter().map(|s| s.guid_index()).collect();
    let find = |g: &ssx_formats::ant::Guid| {
        streams.iter().zip(&indices).find_map(|(s, ix)| ix.get(g).map(|&i| (*s, &s.assets[i].object)))
    };
    let name_of = |g: &ssx_formats::ant::Guid| find(g).and_then(|(s, o)| o.name(&s.schema));
    let mut out: [Option<String>; 2] = [None, None];
    let mut next = Some(config.to_string());
    while let Some(name) = next.take() {
        let (all, a) = streams
            .iter()
            .find_map(|s| {
                s.assets_of_class("AnimatableConfigAsset")
                    .find(|a| a.object.name(&s.schema) == Some(name.as_str()))
                    .map(|a| (*s, a))
            })
            .with_context(|| format!("no AnimatableConfigAsset {name}"))?;
        let guids = |f: &str| -> Vec<ssx_formats::ant::Guid> {
            a.object
                .get(&all.schema, f)
                .and_then(AntValue::as_array)
                .map(|v| v.iter().filter_map(AntValue::as_guid).collect())
                .unwrap_or_default()
        };
        for (t, d) in guids("OverrideGameStateTarget").iter().zip(guids("OverrideGameStateData")) {
            let slot = match name_of(t) {
                Some("PTR_Rigamate_bodytype") => 0,
                Some("PTR_Rigamate_character") => 1,
                _ => continue,
            };
            let program = find(&d)
                .and_then(|(s, o)| o.get(&s.schema, "RigamateAsset"))
                .and_then(AntValue::as_guid)
                .and_then(|g| name_of(&g))
                .with_context(|| format!("{name}: the Rigamate value of slot {slot} names no RigamateAsset"))?;
            out[slot].get_or_insert_with(|| program.to_string());
        }
        next = a
            .object
            .get(&all.schema, "Parent")
            .and_then(AntValue::as_guid)
            .filter(|g| !g.is_null())
            .and_then(|g| name_of(&g))
            .map(str::to_string);
    }
    Ok(out)
}

fn ghost_rigamate(big_root: &Path, rig: &GhostRig, bot: Option<&str>) -> Result<(RigamateRig, Vec<String>)> {
    let stream = |name: &str| -> Result<ssx_formats::ant::Stream> {
        let path = big_root.join(crate::rig_anim::ANIM_DIR).join(name);
        ssx_formats::ant::Stream::parse(&std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?)
            .with_context(|| format!("parsing {}", path.display()))
    };
    let all = crate::content::data::tuning::anim_all(big_root)?;
    let names = GhostSchemas::load(big_root)?;
    let own = bot.map(|c| stream(&format!("{}.cba", names.bot_actor(c)))).transpose()?;
    let config = bot.map_or(names.ghost_config(), |c| format!("{}_config", names.bot_actor(c)));
    let streams: Vec<&ssx_formats::ant::Stream> = own.iter().chain(std::iter::once(&all)).collect();
    let [bodytype, character] = config_rigamates_in(&streams, &config)?;
    let (mut rm, problems) = RigamateRig::load(big_root, &rig.rig, &rig.bind, &names.ghost_animatable)?;
    let mut problems: Vec<String> = problems
        .into_iter()
        .filter(|p| !p.contains("ghost_generic.cba") && !p.contains("Snowboarder_config"))
        .collect();
    let loaded: Vec<&str> = rm.instances.iter().map(|i| i.program.name.as_str()).collect();
    if loaded != [bodytype.as_deref().with_context(|| format!("{config} sets no PTR_Rigamate_bodytype"))?] {
        bail!("{config}: body type {bodytype:?}, loaded {loaded:?}");
    }
    match character {
        Some(name) => {
            let program = streams
                .iter()
                .find_map(|s| crate::rigamate::load_program(s, &name).ok())
                .with_context(|| format!("no RigamateAsset {name}"))?;
            for (s, d) in &program.dofs {
                let n = format!("{s}.{d}");
                if !rm.float_names.contains(&n) {
                    rm.float_names.push(n);
                }
            }
            let names = rm.float_names.clone();
            let instance = crate::rigamate::RigamateInstance::new(program, &rm.skeleton, |s, d| {
                names.iter().position(|n| *n == format!("{s}.{d}"))
            })?;
            rm.instances.push(instance);
        }
        None => problems.push(format!("{config} sets no PTR_Rigamate_character")),
    }
    Ok((rm, problems))
}

pub fn bot_actor(big_root: &Path, character: &str) -> Result<String> {
    Ok(GhostSchemas::load(big_root)?.bot_actor(character))
}

pub struct GhostAnimation {
    animator: Animator,
    idle: Option<Animator>,
    wipeout: Option<Animator>,
    wipeout_weight: f32,
    legs: Option<LegRigOps>,
    rigamate: Option<(RigamateRig, Vec<Option<usize>>)>,
    nis: Option<(String, Animator)>,
}

impl GhostAnimation {
    pub fn new(big_root: &Path, db: &AssetDb, rig: &GhostRig, floats: &[String]) -> Result<(Self, Vec<String>)> {
        Self::with_actor(big_root, db, rig, floats, None)
    }

    pub fn bot(
        big_root: &Path,
        db: &AssetDb,
        rig: &GhostRig,
        floats: &[String],
        character: &str,
    ) -> Result<(Self, Vec<String>)> {
        Self::with_actor(big_root, db, rig, floats, Some(character))
    }

    fn with_actor(
        big_root: &Path,
        db: &AssetDb,
        rig: &GhostRig,
        floats: &[String],
        bot: Option<&str>,
    ) -> Result<(Self, Vec<String>)> {
        let animator = Animator::new(db, GHOST_LAYERS, &rig.rig, rig.bind.clone(), floats)?;
        let mut problems = Vec::new();
        let idle = match Animator::new(db, GHOST_IDLE, &rig.rig, rig.bind.clone(), floats) {
            Ok(a) => Some(a),
            Err(e) => {
                problems.push(format!("ghost idle not loaded: {e:#}"));
                None
            }
        };
        let wipeout = match Animator::new(db, GHOST_WIPEOUT, &rig.rig, rig.bind.clone(), floats) {
            Ok(a) => Some(a),
            Err(e) => {
                problems.push(format!("ghost wipeout layer not loaded: {e:#}"));
                None
            }
        };
        let rigamate = match ghost_rigamate(big_root, rig, bot) {
            Ok((r, p)) => {
                problems.extend(p);
                let slots = r.float_names.iter().map(|n| floats.iter().position(|f| f == n)).collect();
                Some((r, slots))
            }
            Err(e) => {
                problems.push(format!("rigamate not loaded: {e:#}"));
                None
            }
        };
        let legs = LegRigOps::new(&rig.rig, floats);
        if legs.is_none() {
            problems.push("leg rig ops: the rig lacks a leg or effector joint".into());
        }
        Ok((Self { animator, idle, wipeout, wipeout_weight: 0.0, legs, rigamate, nis: None }, problems))
    }

    pub fn pose(&mut self, rig: &GhostRig, p: &GhostPose, dt_ticks: f32) -> RigPose {
        self.animator.set("deltaTicks", dt_ticks);
        self.animator.update(dt_ticks);
        let mut pose = self.animator.pose();
        for (n, q) in rig.rotations.iter().zip(&p.rotations) {
            if let Some(s) = n.and_then(|n| pose.nodes.get_mut(n)) {
                s.rotation = *q;
            }
        }
        for (n, t) in rig.translations.iter().zip(&p.translations) {
            if let Some(s) = n.and_then(|n| pose.nodes.get_mut(n)) {
                s.translation = *t;
            }
        }
        for &r in &rig.roots {
            pose.nodes[r] = SRT_IDENTITY;
        }
        pose.nodes[rig.trajectory] = Srt { rotation: p.trajectory_rotation, ..SRT_IDENTITY };
        if let Some(wa) = self.wipeout.as_mut() {
            wa.set("deltaTicks", dt_ticks);
            wa.update(dt_ticks);
            let w = self.wipeout_weight.clamp(0.0, 1.0);
            if w > 0.0 {
                let layer = wa.pose();
                for (i, (n, l)) in pose.nodes.iter_mut().zip(&layer.nodes).enumerate() {
                    if i == rig.trajectory || rig.roots.contains(&i) {
                        continue;
                    }
                    n.rotation = nlerp(n.rotation, l.rotation, w);
                    n.translation = lerp3(n.translation, l.translation, w);
                    n.scale = lerp3(n.scale, l.scale, w);
                }
            }
        }
        // TODO(guess): replays of builds ≤ 434 653 play back with `BLN_ClassicRig` (0x82C78A6C), sending the pinning kernel 0x82D08800 down its classic path (0x82D08BB8…0x82D08C28), which `LegRigOps` lacks; settle: port the classic path in `anim_rigops` (the rig-op runtime's +0x00, 0x82CE5448 / 0x82D082E0).
        if let Some(legs) = self.legs.as_mut().filter(|_| !p.board_particle()) {
            legs.apply(&rig.rig, &mut pose.nodes, &pose.floats, None);
        }
        if p.board_particle()
            && let Some(board) = rig.board
        {
            place_board_at_default(rig, &mut pose.nodes, board, p.trajectory_position);
        }
        if let Some((rm, slots)) = &mut self.rigamate {
            let floats: Vec<f32> = slots.iter().map(|s| s.map_or(0.0, |i| pose.floats[i])).collect();
            rm.apply(&mut pose.nodes, &floats);
        }
        pose
    }
}

impl GhostAnimation {
    pub fn set_signal(&mut self, name: &str, v: f32) {
        self.animator.set(name, v);
        if let Some(w) = self.wipeout.as_mut() {
            w.set(name, v);
        }
    }

    pub fn set_wipeout_weight(&mut self, w: f32) {
        self.wipeout_weight = w;
    }

    pub fn playing(&self) -> Vec<(String, f32)> {
        let mut out = self.animator.playing();
        if let Some(w) = self.wipeout.as_ref().filter(|_| self.wipeout_weight > 0.0) {
            out.extend(w.playing().into_iter().map(|(n, x)| (n, x * self.wipeout_weight)));
        }
        out
    }

    pub fn idle_pose(
        &mut self,
        rig: &GhostRig,
        dt_ticks: f32,
        offset: [f32; 3],
        orientation: [f32; 4],
    ) -> Option<(RigPose, [f32; 3])> {
        let idle = self.idle.as_mut()?;
        idle.set("deltaTicks", dt_ticks);
        idle.update(dt_ticks);
        let pose = idle.pose();
        Some(self.placed(rig, pose, offset, orientation))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn nis_pose(
        &mut self,
        db: &AssetDb,
        rig: &GhostRig,
        floats: &[String],
        root: &str,
        time: f32,
        at: [f32; 3],
        rotation: [f32; 4],
    ) -> Option<(RigPose, [f32; 3])> {
        if self.nis.as_ref().is_none_or(|(n, _)| n != root) {
            self.nis = Animator::new(db, root, &rig.rig, rig.bind.clone(), floats).ok().map(|a| (root.to_string(), a));
        }
        let (_, a) = self.nis.as_mut()?;
        a.set_time(time);
        a.set("deltaTicks", 0.0);
        a.update(0.0);
        let mut pose = a.pose();
        pose.nodes[rig.trajectory] = Srt { rotation, ..SRT_IDENTITY };
        let (pose, _) = self.placed(rig, pose, [0.0; 3], [0.0, 0.0, 0.0, 1.0]);
        Some((pose, at))
    }

    fn placed(
        &mut self,
        rig: &GhostRig,
        mut pose: RigPose,
        offset: [f32; 3],
        orientation: [f32; 4],
    ) -> (RigPose, [f32; 3]) {
        let clip = pose.nodes[rig.trajectory];
        let [x, y, z, _] =
            crate::rigamate::rotate(orientation, [clip.translation[0], clip.translation[1], clip.translation[2], 0.0]);
        let at = [offset[0] + x, offset[1] + y, offset[2] + z];
        for &r in &rig.roots {
            pose.nodes[r] = SRT_IDENTITY;
        }
        pose.nodes[rig.trajectory] =
            Srt { rotation: crate::rigamate::quat_mul(orientation, clip.rotation), ..SRT_IDENTITY };
        if let Some(legs) = self.legs.as_mut() {
            legs.apply(&rig.rig, &mut pose.nodes, &pose.floats, None);
        }
        if let Some((rm, slots)) = &mut self.rigamate {
            let floats: Vec<f32> = slots.iter().map(|s| s.map_or(0.0, |i| pose.floats[i])).collect();
            rm.apply(&mut pose.nodes, &floats);
        }
        (pose, at)
    }

    pub fn reset_idle(&mut self) {
        if let Some(a) = self.idle.as_mut() {
            a.reset();
        }
    }
}
