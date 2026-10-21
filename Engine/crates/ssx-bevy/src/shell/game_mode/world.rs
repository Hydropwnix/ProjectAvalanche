use std::time::Instant;

use bevy::prelude::*;

use super::{
    FrontEndCamera, GameUiRes, LoadedLevel, Shell, WorldOp, audio_values, settings_values, spawn_front_end_camera,
};
use crate::audio::music::frontend::{self as fe_audio, FrontEndAudio};
use crate::shell::{game_level, level};

pub(super) fn shell_world(world: &mut World) {
    let (op, audio, stats) = {
        let mut s = world.resource_mut::<Shell>();
        let op = s.op.get_mut().ok().and_then(Option::take);
        (op, std::mem::take(&mut s.op_audio), std::mem::take(&mut s.op_stats))
    };
    if stats {
        let rides = world.resource::<Shell>().rides_done;
        let stats = level::stats(world);
        info!("shell: the world 120 frames after level {rides}'s teardown: {stats}");
    }
    if audio {
        let values = settings_values(&world.resource::<GameUiRes>().values);
        fe_audio::apply_audio_settings(world, &audio_values(&values));
    }
    match op {
        Some(WorldOp::Spawn(loaded)) => spawn_level(world, *loaded),
        Some(WorldOp::Teardown) => teardown_level(world),
        None => {}
    }
}

pub(super) fn speech_block(
    big_root: &std::path::Path,
    track: &str,
    character: &str,
    words: ssx_assets::sfx::speech::launch::CampaignWords,
    required: [bool; 7],
    descriptions: [Option<u32>; ssx_ui::game::fe::setup::GEAR_TYPES],
) -> ssx_assets::sfx::speech::launch::LaunchBlock {
    use ssx_assets::sfx::speech::launch;
    let infos = launch::gear_audio_infos(big_root)
        .inspect_err(|e| warn!("shell: speech block: no gear audio infos ({e:#})"))
        .unwrap_or_default();
    let gear = descriptions.map(|d| d.and_then(|d| infos.get(&d).copied()).unwrap_or(0));
    let rider = ssx_assets::sfx::rider_audio_info(big_root, character).map_or(0, |r| r.key);
    let peak = ssx_assets::sfx::track_audio_info(big_root, track).ok().map(|t| (t.dd_rider, t.dd_gear));
    let block = launch::LaunchBlock::new(words, required, gear, rider, peak);
    info!(
        "shell: speech block (0x823F7788): chapter {} drop {}/{} dd {} hold {} gear {:?}",
        words.chapter, words.drop, words.drops, words.dd, block.dd_hold, block.gear
    );
    block
}

fn spawn_level(world: &mut World, loaded: LoadedLevel) {
    let t0 = Instant::now();
    let LoadedLevel { data, music, sfx, game, speech_block } = loaded;
    let drop_key = data.lighting.as_ref().map(|l| l.drop_key);
    let load_secs = data.load_secs;
    let fe_cams: Vec<Entity> = world.query_filtered::<Entity, With<FrontEndCamera>>().iter(world).collect();
    for e in fe_cams {
        world.despawn(e);
    }
    level::begin(world);
    if let Some(mut a) = world.get_resource_mut::<FrontEndAudio>() {
        a.unplug();
    }
    if let Err(e) = data.insert(world) {
        error!("shell: the level's data did not go in: {e:#}");
    }
    crate::audio::music::start_level(world, music, drop_key);
    crate::audio::sfx::start_level(world, sfx);
    let goals = world.get_resource::<GameUiRes>().map(|ui| [ui.values.int(0x87), ui.values.int(0x6E)]);
    let rng = world.get_resource::<FrontEndAudio>().and_then(FrontEndAudio::aems_rng);
    if let (Some(goals), Some(g)) = (goals, world.get_resource::<crate::audio::music::GameAudio>())
        && let Some(mut m) = g.lock()
        && let Some(sfx) = m.sfx.as_mut()
    {
        sfx.ui_goals = goals;
        sfx.launch_block = speech_block;
        if let Some(rng) = rng {
            sfx.aems.set_rng_state(rng);
        }
    }
    let insert_secs = t0.elapsed().as_secs_f32();
    let startup_secs = level::run_startup(world);
    game_level::start(world, game);
    let stats = level::stats(world);
    info!(
        "shell: the level is in the world: loaded in {load_secs:.2} s (worker thread), inserted in {insert_secs:.3} s, \
         spawned in {startup_secs:.3} s (main thread); {stats}"
    );
}

fn teardown_level(world: &mut World) {
    let t0 = Instant::now();
    game_level::end(world);
    crate::audio::sfx::end_level(world);
    crate::audio::music::end_level(world);
    let (entities, resources) = level::teardown(world);
    if let Some(mut a) = world.get_resource_mut::<FrontEndAudio>() {
        a.plug();
    }
    spawn_front_end_camera(world);
    let stats = level::stats(world);
    info!(
        "shell: the level is torn down in {:.3} s ({entities} entities despawned; resources added since it began that \
         remain: {resources}); {stats}",
        t0.elapsed().as_secs_f32()
    );
}
