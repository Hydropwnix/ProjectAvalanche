use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::{ExtractSchedule, MainWorld, RenderApp};

const MAX_WAIT_FRAMES: u32 = 1200;
const UP_FRAMES: u32 = 30;
const SETTLE_FRAMES: u32 = 3;

pub struct AutoScreenshotPlugin {
    pub path: PathBuf,
    pub frames: u32,
}

impl Plugin for AutoScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(AutoScreenshot {
            path: self.path.clone(),
            frames: self.frames,
            ready_frames: 0,
            elapsed_frames: 0,
            up_since: None,
            requested: false,
        })
        .init_resource::<PipelinesReady>()
        .init_resource::<PipelinesWaiting>()
        .add_systems(Update, request_screenshot);
        app.sub_app_mut(RenderApp).add_systems(ExtractSchedule, report_pipelines);
    }
}

#[derive(Resource)]
struct AutoScreenshot {
    path: PathBuf,
    frames: u32,
    ready_frames: u32,
    elapsed_frames: u32,
    up_since: Option<u32>,
    requested: bool,
}

#[derive(Resource, Default)]
pub struct PipelinesReady(pub bool);

#[derive(Resource, Default)]
pub struct PipelinesWaiting(pub Vec<String>);

fn report_pipelines(mut main_world: ResMut<MainWorld>, pipelines: Res<PipelineCache>) {
    let waiting: Vec<usize> = pipelines.waiting_pipelines().collect();
    if let Some(mut ready) = main_world.get_resource_mut::<PipelinesReady>() {
        ready.0 = waiting.is_empty();
    }
    let Some(mut out) = main_world.get_resource_mut::<PipelinesWaiting>() else { return };
    if waiting.is_empty() && out.0.is_empty() {
        return;
    }
    let all: Vec<_> = pipelines.pipelines().collect();
    out.0 = waiting.iter().filter_map(|&id| all.get(id).map(|p| describe_pipeline(id, p))).collect();
}

fn describe_pipeline(id: usize, p: &bevy::render::render_resource::CachedPipeline) -> String {
    let state = match &p.state {
        CachedPipelineState::Queued => "queued".to_string(),
        CachedPipelineState::Creating(_) => "creating".to_string(),
        CachedPipelineState::Ok(_) => "ok".to_string(),
        CachedPipelineState::Err(e) => format!("error: {e}"),
    };
    match &p.descriptor {
        PipelineDescriptor::RenderPipelineDescriptor(d) => {
            let fragment = d.fragment.as_ref().map(|f| (f.shader.path().map(|p| p.to_string()), f.shader.id()));
            format!(
                "#{id} render {:?}: vertex {:?} ({:?}), fragment {fragment:?}, {} vertex defs — {state}",
                d.label.as_deref().unwrap_or("-"),
                d.vertex.shader.path().map(|p| p.to_string()),
                d.vertex.shader.id(),
                d.vertex.shader_defs.len(),
            )
        }
        PipelineDescriptor::ComputePipelineDescriptor(d) => format!(
            "#{id} compute {:?}: {:?} ({:?}) — {state}",
            d.label.as_deref().unwrap_or("-"),
            d.shader.path().map(|p| p.to_string()),
            d.shader.id(),
        ),
    }
}

fn request_screenshot(
    mut commands: Commands,
    mut shot: ResMut<AutoScreenshot>,
    pipelines_ready: Res<PipelinesReady>,
    waiting: Res<PipelinesWaiting>,
    meshes: Query<(), With<Mesh3d>>,
    diagnostics: Res<DiagnosticsStore>,
) {
    if shot.requested {
        return;
    }
    shot.elapsed_frames += 1;
    let before = shot.ready_frames;
    shot.ready_frames = if pipelines_ready.0 && !meshes.is_empty() { shot.ready_frames + 1 } else { 0 };
    let up_frames = UP_FRAMES.min(shot.frames);
    if shot.up_since.is_none() && shot.ready_frames >= up_frames {
        shot.up_since = Some(shot.elapsed_frames - up_frames);
    }
    if before >= UP_FRAMES && shot.ready_frames == 0 {
        info!(
            "screenshot: a pipeline compiles at frame {} after {before} fully drawn frames: {:?}",
            shot.elapsed_frames, waiting.0
        );
    }

    let counted = shot.up_since.map_or(0, |up| shot.elapsed_frames - up);
    let timed_out = shot.elapsed_frames >= shot.frames.saturating_add(MAX_WAIT_FRAMES);
    if (counted < shot.frames || shot.ready_frames < SETTLE_FRAMES.min(shot.frames)) && !timed_out {
        return;
    }
    if timed_out {
        warn!(
            "Render pipelines still compiling {MAX_WAIT_FRAMES} frames after the {} asked for; capturing anyway ({} \
             fully drawn frames in a row; waiting: {:?})",
            shot.frames, shot.ready_frames, waiting.0
        );
    }
    info!("Capturing screenshot at frame {}", shot.elapsed_frames);
    if let Some(ms) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME).and_then(|d| d.average()) {
        info!("Frame time before capture: {ms:.2} ms average ({:.0} fps)", 1000.0 / ms);
    }
    shot.requested = true;

    let path = shot.path.clone();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
            let status = save_png(captured.image.clone(), &path);
            exit.write(match status {
                Ok(()) => {
                    info!("Screenshot saved to {}", path.display());
                    AppExit::Success
                }
                Err(e) => {
                    error!("Saving the screenshot failed: {e:#}");
                    AppExit::error()
                }
            });
        },
    );
}

fn save_png(image: Image, path: &Path) -> Result<()> {
    let rgb = image.try_into_dynamic().context("converting the captured frame")?.to_rgb8();
    rgb.save(path).with_context(|| format!("writing {}", path.display()))
}
