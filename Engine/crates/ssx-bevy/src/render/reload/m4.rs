use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use bevy::log::{info, warn};
use bevy::prelude::{Commands, Local, Res, ResMut, World};
use ssx_content::Kind;

use super::{ContentReload, ReloadState, Replaced};
use crate::render::lighting::{DropSelect, GameLighting};

pub(super) struct Presentation {
    lighting: GameLighting,
    sky: anyhow::Result<crate::render::sky::SkyLoad>,
    post: anyhow::Result<crate::render::post::PostLoad>,
    seconds: f64,
}

fn touches(reload: &ContentReload, l: &GameLighting) -> bool {
    let dome = matches!(l.level.sky, ssx_assets::content::level::LevelSky::Dome { .. });
    match &reload.ids {
        None => true,
        Some(ids) => ids.iter().any(|id| match id.kind() {
            Kind::Sky | Kind::Lighting | Kind::Postfx | Kind::Particles => true,
            Kind::Model | Kind::Material => dome,
            _ => l.level.textures.contains_key(id),
        }),
    }
}

pub(super) fn presentation(
    reload: Res<ContentReload>,
    state: Res<ReloadState>,
    mut seen: Local<Option<u64>>,
    mut pending: Local<Option<Mutex<Receiver<anyhow::Result<Presentation>>>>>,
    lighting: Option<ResMut<GameLighting>>,
    mut replaced: ResMut<Replaced>,
    mut commands: Commands,
) {
    let Some(mut lighting) = lighting else { return };
    if let Some(rx) = pending.as_ref() {
        let got = rx.lock().ok().and_then(|r| r.try_recv().ok());
        match got {
            Some(Ok(p)) => {
                *pending = None;
                info!("mod-dev: the level's lighting, sky and post-FX loaded again in {:.2} s", p.seconds);
                lighting.replace_presets(p.lighting);
                let (sky, post) = (p.sky, p.post);
                replaced.0 += 1;
                commands.queue(move |world: &mut World| apply(world, sky, post));
            }
            Some(Err(e)) => {
                *pending = None;
                warn!("mod-dev: the level's lighting not loaded again: {e:#}; the last good state stays");
            }
            None => return,
        }
    }
    let fresh = reload.generation > lighting.level.generation && *seen != Some(reload.generation);
    *seen = Some(reload.generation);
    if !fresh || !touches(&reload, &lighting) {
        return;
    }
    let (root, track, drop) = (state.big_root.clone(), lighting.track.clone(), lighting.drop_index);
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        let loaded = crate::render::lighting::load(&root, &track, &DropSelect::Index(drop)).map(|lighting| {
            let sky = crate::render::sky::load(&lighting, &root);
            let post = crate::render::post::load(&lighting, &root, false);
            Presentation { lighting, sky, post, seconds: start.elapsed().as_secs_f64() }
        });
        let _ = tx.send(loaded);
    });
    *pending = Some(Mutex::new(rx));
}

fn apply(
    world: &mut World,
    sky: anyhow::Result<crate::render::sky::SkyLoad>,
    post: anyhow::Result<crate::render::post::PostLoad>,
) {
    match sky.and_then(|s| crate::render::sky::insert(world, s)) {
        Ok(()) => info!("mod-dev: the sky loaded again"),
        Err(e) => warn!("mod-dev: the sky not loaded again: {e:#}"),
    }
    match post.and_then(|p| crate::render::post::insert(world, p)) {
        Ok(()) => info!("mod-dev: the post-FX preset loaded again"),
        Err(e) => warn!("mod-dev: the post-FX preset not loaded again: {e:#}"),
    }
    crate::render::lighting::game::refresh_light_assets(world);
    crate::particles::reload_world(world);
}
