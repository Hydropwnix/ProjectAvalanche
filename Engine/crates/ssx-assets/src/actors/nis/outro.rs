use super::*;

pub const OUTRO_BINDING: &str = "nis_binding_outro";

pub const OUTRO_REACTION_WIN: i64 = 0;
pub const OUTRO_REACTION_NEUT: i64 = 1;
pub const OUTRO_REACTION_LOSE: i64 = 2;

pub fn load_outro(big_root: &Path, character: &str, reaction: i64) -> Result<IntroNis> {
    let binding = OUTRO_BINDING;
    let bind = Binding::load(big_root, binding, character)?;
    let db = &bind.db;
    let walk = bind.walk(HashMap::from([("INT_TrackEndReaction".to_string(), reaction)]))?;
    let b = db.by_name(binding).context("binding")?;
    let sf = db.get(b.guid("ControllerAsset").context("binding without controller")?).context("state flow")?;
    let start = db.get(sf.guid("StartNode").context("no start node")?).context("start node")?;
    let t = sf
        .guids("StateFlowTransitionAssets")
        .into_iter()
        .filter_map(|g| db.get(g))
        .find(|t| same(t.guid("From"), start.guid))
        .with_context(|| format!("{binding}: no transition from {}", start.name()))?;
    anyhow::ensure!(
        walk.name(&t, "BranchWindowTypeAsset").as_deref() == Some("NIS_Out"),
        "{}: not a NIS_Out branch window",
        t.name()
    );
    let blend = t.guid("OverrideBlend").and_then(|g| db.get(g)).and_then(|b| b.f32("BlendTime")).unwrap_or(0.0);
    let cut = db.get(t.guid("To").context("transition without target")?).context("camera-cut node")?;
    let (ready, jump) = (walk.scene(&start)?, walk.scene(&cut)?);
    let branch_out = ready.actor(ROLE_CHARACTER1).and_then(|a| a.tags.branch_out);
    let controller = sf.name().to_string();
    let Binding { bundles, rigs: [rider, heli, camera], .. } = bind;
    Ok(IntroNis {
        binding: binding.to_string(),
        controller,
        ready,
        jump,
        transition_blend: blend,
        branch_out,
        bundles,
        rider,
        heli,
        camera,
        cabin: Vec::new(),
        requires_wingsuit: false,
        character_of_interest: false,
        ready_loop: true,
        jump_start: 0.0,
    })
}

pub const HELI_PICKUP_OUTRO_BINDING: &str = "nis_binding_outro_survive_til_die";

pub fn load_heli_pickup_outro(big_root: &Path, character: &str) -> Result<IntroNis> {
    let binding = HELI_PICKUP_OUTRO_BINDING;
    let bind = Binding::load(big_root, binding, character)?;
    let db = &bind.db;
    let walk = bind.walk(HashMap::new())?;
    let b = db.by_name(binding).context("binding")?;
    let sf = db.get(b.guid("ControllerAsset").context("binding without controller")?).context("state flow")?;
    let start = db.get(sf.guid("StartNode").context("no start node")?).context("start node")?;
    anyhow::ensure!(sf.guids("StateFlowTransitionAssets").is_empty(), "{binding}: the flow has transitions");
    let ready = walk.scene(&start)?;
    let controller = sf.name().to_string();
    let Binding { bundles, rigs: [rider, heli, camera], .. } = bind;
    Ok(IntroNis {
        binding: binding.to_string(),
        controller,
        jump: ready.clone(),
        ready,
        transition_blend: 0.0,
        branch_out: None,
        bundles,
        rider,
        heli,
        camera,
        cabin: Vec::new(),
        requires_wingsuit: false,
        character_of_interest: false,
        ready_loop: true,
        jump_start: 0.0,
    })
}

pub const HELI_GRAB_OUTRO_BINDING: &str = "nis_end_heli_grab";

pub fn load_heli_grab_outro(big_root: &Path, character: &str, reaction: i64) -> Result<IntroNis> {
    let binding = HELI_GRAB_OUTRO_BINDING;
    let bind = Binding::load_with(big_root, binding, character, &["nis_binding_outro.cba"])?;
    let db = &bind.db;
    let walk = bind.walk(HashMap::from([("INT_TrackEndReaction".to_string(), reaction)]))?;
    let b = db.by_name(binding).context("binding")?;
    let sf = db.get(b.guid("ControllerAsset").context("binding without controller")?).context("state flow")?;
    let start = db.get(sf.guid("StartNode").context("no start node")?).context("start node")?;
    let exit = |from: &Asset, window: &str| -> Result<(Asset, f32)> {
        let t = sf
            .guids("StateFlowTransitionAssets")
            .into_iter()
            .filter_map(|g| db.get(g))
            .find(|t| {
                same(t.guid("From"), from.guid) && walk.name(t, "BranchWindowTypeAsset").as_deref() == Some(window)
            })
            .with_context(|| format!("{binding}: no {window} transition from {}", from.name()))?;
        let blend = t.guid("OverrideBlend").and_then(|g| db.get(g)).and_then(|b| b.f32("BlendTime")).unwrap_or(0.0);
        Ok((db.get(t.guid("To").context("transition without target")?).context("target node")?, blend))
    };
    let window_of = |sc: &NisScene, early: bool| {
        sc.actors.iter().filter_map(|a| if early { a.tags.early_out } else { a.tags.branch_out }).reduce(f32::min)
    };
    let scene1 = walk.scene(&start)?;
    let (ready, jump, blend, branch_out, cabin) = if reaction == OUTRO_REACTION_WIN {
        let (cut, blend) = exit(&start, "NIS_Out")?;
        let out = window_of(&scene1, false).with_context(|| format!("{binding}: scene 1 has no NIS_Out window"))?;
        (scene1, walk.scene(&cut)?, blend, Some(out), Vec::new())
    } else {
        let (lose, early_blend) = exit(&start, "NIS_EarlyOut")?;
        let early =
            window_of(&scene1, true).with_context(|| format!("{binding}: scene 1 has no NIS_EarlyOut window"))?;
        let lose_scene = walk.scene(&lose)?;
        let (cut, blend) = exit(&lose, "NIS_Out")?;
        let out = window_of(&lose_scene, false).with_context(|| format!("{binding}: the lose scene has no NIS_Out"))?;
        let first = CabinScene {
            scene: scene1,
            start: 0.0,
            end: early,
            window: "NIS_EarlyOut".into(),
            blend: early_blend,
            tuning_offset: None,
            skip: None,
        };
        (lose_scene, walk.scene(&cut)?, blend, Some(out), vec![first])
    };
    let controller = sf.name().to_string();
    let Binding { bundles, rigs: [rider, heli, camera], .. } = bind;
    Ok(IntroNis {
        binding: binding.to_string(),
        controller,
        ready,
        jump,
        transition_blend: blend,
        branch_out,
        bundles,
        rider,
        heli,
        camera,
        cabin,
        requires_wingsuit: false,
        character_of_interest: false,
        ready_loop: true,
        jump_start: 0.0,
    })
}

pub fn binding_item_start(big_root: &Path, binding: &str, role: i64) -> Result<Option<([f32; 3], [f32; 4])>> {
    let dir = big_root.join(ANIM_DIR);
    let db = AssetDb::load(&dir, &[&format!("{binding}.cba"), RIG_BUNDLE])?;
    let b = db.by_name(binding).with_context(|| format!("no NisBindingAsset {binding}"))?;
    Ok(b.guids("Bindings")
        .into_iter()
        .filter_map(|g| db.get(g))
        .find(|item| enum_value(&db, item, "RoleId") == Some(role))
        .map(|item| (vec3_of(&item, "StartPosition"), quat_of(&item, "StartOrientation"))))
}

pub fn requires_wingsuit(big_root: &Path, binding: &str) -> Result<bool> {
    let dir = big_root.join(ANIM_DIR);
    let db = AssetDb::load(&dir, &[&format!("{binding}.cba")])?;
    let b = db.by_name(binding).with_context(|| format!("no NisBindingAsset {binding}"))?;
    Ok(b.bool("RequiresWingsuit").unwrap_or(false))
}

pub fn character_of_interest_name(big_root: &Path, binding: &str) -> Result<Option<String>> {
    let dir = big_root.join(ANIM_DIR);
    let db = AssetDb::load(&dir, &[&format!("{binding}.cba")])?;
    let b = db.by_name(binding).with_context(|| format!("no NisBindingAsset {binding}"))?;
    Ok(b.value("CharacterOfInterestName").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string))
}
