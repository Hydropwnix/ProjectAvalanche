use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

use anyhow::{Context, Result};
use bevy::camera::visibility::RenderLayers;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::render::{ExtractSchedule, RenderApp};
use bevy::window::{PresentMode, WindowResolution};
use bevy::winit::WinitSettings;
use serde::{Deserialize, Serialize};
use ssx_assets::career::Profile;
use ssx_assets::career::profile::DropState;
use ssx_assets::career::save::SaveSchema;
use ssx_ui::game::GameUi;
use ssx_ui::game::fe::host::{Host, Phase as HostPhase, Step};
use ssx_ui::game::loc::Language;

use crate::audio::music::frontend as fe_audio;
use crate::shell::game_level;
use crate::shell::level::{self, LevelData, ShadingArg};
use crate::ui;

mod check;
mod frame;
mod pads;
mod save;
mod stage;
mod window;
mod world;

pub use check::{UiPipelinesReady, WorldCheck, report_pipelines, screenshot, ui_cover};
use frame::{shell_input, shell_step};
pub use pads::{PadSlots, PadState, StickDpad, pad_presses, pads_frame};
pub use save::SaveSlot;
pub use window::{WindowHidden, hidden_frame};
use world::shell_world;

pub const SETTINGS_VALUES: [u16; 12] = [0x1F0, 0x9D, 0x9E, 0x28, 0x23, 0x26, 0x21, 0x2A, 0x24, 0x27, 0x22, 0x29];

pub fn settings_values(values: &ssx_ui::game::values::UiValues) -> Vec<(u16, i32)> {
    SETTINGS_VALUES.iter().filter(|&&id| values.written(id)).map(|&id| (id, values.int(id))).collect()
}

pub const SAVE_FILE: &str = "ssxvprofile";

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WindowSpec {
    pub width: u32,
    pub height: u32,
    pub physical: bool,
    pub vsync: bool,
    pub position: Option<(i32, i32)>,
}

impl WindowSpec {
    pub fn window(&self, title: &str) -> Window {
        let resolution = if self.physical {
            WindowResolution::new(self.width, self.height).with_scale_factor_override(1.0)
        } else {
            UVec2::new(self.width, self.height).into()
        };
        Window {
            title: title.into(),
            resolution,
            present_mode: if self.vsync { PresentMode::default() } else { PresentMode::AutoNoVsync },
            position: self.position.map_or(WindowPosition::Automatic, |(x, y)| WindowPosition::At(IVec2::new(x, y))),
            ..default()
        }
    }
}

pub struct GameArgs {
    pub big_root: PathBuf,
    pub save: Option<PathBuf>,
    pub lang: String,
    pub window: WindowSpec,
    pub script: String,
    pub movie_cap: Option<f64>,
    pub seconds: Option<f64>,
    pub rides: Option<u32>,
    pub ride_limits: Vec<f32>,
    pub ride_shots: Vec<f32>,
    pub level_scripts: Vec<String>,
    pub shot_dir: PathBuf,
    pub autoplay: bool,
    pub follow_prompts: bool,
    pub ride_win: Option<f32>,
    pub debug_hud: bool,
    pub fade_shots: u32,
    pub subdivide: Option<u32>,
    pub shading: Option<ShadingArg>,
    pub no_objects: bool,
    pub wireframe: bool,
    pub launch: Option<(String, f64)>,
}

fn report_gpu(
    info: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    device: Option<Res<bevy::render::renderer::RenderDevice>>,
) {
    use bevy::render::render_resource::WgpuFeatures as F;
    let (Some(info), Some(device)) = (info, device) else {
        warn!("shell: GPU: no adapter or device in the world at start-up");
        return;
    };
    let f = device.features();
    let yes = |on: bool| if on { "yes" } else { "NO" };
    info!(
        "shell: GPU {:?} — backend {:?}, {:?}, driver {:?} {:?}; BC textures {}, wireframe (line polygons) {}, \
         clamp-to-border sampling {}, timestamp queries {}; largest 2-D texture {}, {} bind groups",
        info.name,
        info.backend,
        info.device_type,
        info.driver,
        info.driver_info,
        yes(f.contains(F::TEXTURE_COMPRESSION_BC)),
        yes(f.contains(F::POLYGON_MODE_LINE)),
        yes(f.contains(F::ADDRESS_MODE_CLAMP_TO_BORDER)),
        yes(f.contains(F::TIMESTAMP_QUERY)),
        device.limits().max_texture_dimension_2d,
        device.limits().max_bind_groups,
    );
}

fn start_failure_hook() {
    static FIRST: std::sync::Once = std::sync::Once::new();
    FIRST.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |p| {
            before(p);
            let text = p.payload_as_str().unwrap_or("(no message)");
            let hint = if text.contains("Unable to find a GPU") || text.contains("adapter") {
                " — no usable graphics adapter: update the graphics driver; WGPU_BACKEND=dx12 or =vulkan chooses the \
                 backend"
            } else if text.contains("feature") || text.contains("Failed to create") || text.contains("device") {
                " — the graphics device refused what the viewer asked for: update the graphics driver, or try the \
                 other backend with WGPU_BACKEND=vulkan or =dx12"
            } else {
                ""
            };
            eprintln!("Project Avalanche stopped: {text}{hint}");
        }));
    });
}

pub fn default_save_path() -> PathBuf {
    ssx_assets::setup::app_dir().join("Saves").join(SAVE_FILE)
}

pub fn summary(p: &Profile) -> String {
    let count = |s: DropState| p.drop_states.values().filter(|&&d| d == s).count();
    format!(
        "credits {}, earned {}, drops unlocked {} / completed {}, chapter drops {}, scores {}, new-game state {}, \
         collected geotags {}, movies {}",
        p.credits(),
        p.earned(),
        count(DropState::Unlocked),
        count(DropState::Completed),
        p.chapter_drops.len(),
        p.explore_scores.len(),
        p.new_game_state,
        p.collected_geotags.len(),
        p.movies.len()
    )
}

pub fn boot_ui(big_root: &Path, lang: Language) -> Result<(GameUi, ui::UiScene)> {
    use ssx_ui::avm1::{Vm, natives};
    use ssx_ui::display::Player;
    use ssx_ui::fs::Disc;
    use ssx_ui::library::Library;
    let disc = Disc::new(big_root)?;
    let game = GameUi::new(&disc, lang)?;
    let mut lib = Library::new(Box::new(Disc::new(big_root)?));
    lib.load_font_library(lang.font_region())?;
    let mut player = Player::new();
    let mut vm = Vm::new();
    let boot = lib.load_movie("bootstrapper.gfx")?;
    vm.load_level(&mut lib, &mut player, boot, 0);
    // TODO(guess): the PC's display counts as widescreen (0x82FB57A0's `fIsWideScreen`: the render manager's display set-up reads `XGetVideoMode` and tells the movies 1280 × 720 on a widescreen display, 960 × 720 otherwise); settle: none (a PC has no such dashboard setting; the window shows the 16:9 stage scaled and cropped).
    natives::set_game_globals(&mut vm.ctx(&mut lib, &mut player), 1280, 720, true);
    for _ in 0..60 {
        if vm.trace_log.iter().any(|l| l.contains("OnAllSwfsLoaded")) {
            break;
        }
        vm.step_frame(&mut lib, &mut player);
    }
    let mut scene = ui::UiScene::new(lib, player);
    scene.vm = Some(vm);
    scene.playing = false;
    Ok((game, scene))
}

const MOVIE_LAYER: usize = 7;

const FE_BACKGROUND: Color = Color::BLACK;

#[derive(Resource, Deref, DerefMut)]
pub struct GameUiRes(pub GameUi);

#[derive(Component)]
pub struct FrontEndCamera;

fn spawn_front_end_camera(world: &mut World) {
    world.spawn((
        Camera2d,
        Camera { clear_color: ClearColorConfig::Custom(FE_BACKGROUND), ..default() },
        Msaa::Off,
        ui::UiCamera,
        FrontEndCamera,
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Menus,
    Loading,
    Level,
}

pub struct LoadedLevel {
    pub data: LevelData,
    pub music: Option<crate::audio::music::MusicLevel>,
    pub sfx: Option<crate::audio::sfx::SfxLevel>,
    pub game: game_level::LevelLoad,
    pub speech_block: Option<ssx_assets::sfx::speech::launch::LaunchBlock>,
}

enum WorldOp {
    Spawn(Box<LoadedLevel>),
    Teardown,
}

#[derive(Resource)]
struct Shell {
    host: Host,
    big_root: PathBuf,
    stage: Stage,
    save: SaveSlot,
    stat_names: std::sync::Arc<std::collections::BTreeMap<u32, String>>,
    loading: Option<(std::sync::Mutex<Receiver<Result<LoadedLevel>>>, Instant)>,
    op: std::sync::Mutex<Option<WorldOp>>,
    op_audio: bool,
    stats_in: Option<u32>,
    op_stats: bool,
    launch: Option<ssx_ui::game::fe::Launch>,
    rides: u32,
    rides_done: u32,
    segments: std::collections::VecDeque<Vec<(f64, Step)>>,
    segment_started: bool,
    max_rides: Option<u32>,
    ride_limits: Vec<f32>,
    ride_shots: Vec<f32>,
    level_scripts: Vec<String>,
    seconds: Option<f64>,
    movie_cap: f64,
    shot_dir: PathBuf,
    autoplay: bool,
    follow_prompts: bool,
    ride_win: Option<f32>,
    debug_hud: bool,
    subdivide: Option<u32>,
    shading: Option<ShadingArg>,
    no_objects: bool,
    aspect: f32,
    movie: Option<ui::movie::MoviePlayback>,
    movie_ended: Option<u32>,
    lang: String,
    last_stick: Vec2,
    stick_dpad: StickDpad,
    movie_sprite: Entity,
    movie_camera: Entity,
    ready_frames: u32,
    started: bool,
    hold: u32,
    pending_shots: Vec<String>,
    log_seen: (usize, usize),
    notes: Vec<String>,
    done_at: Option<f64>,
    test_launch: Option<(String, f64)>,
    launch_start_at: f64,
    popup_reported: bool,
    frame_sleep: Option<u64>,
    input_log: bool,
    fade_shots: u32,
    fade_frames: u32,
    fade_last: f32,
}

const LAUNCH_BOOT_WAIT: f64 = 20.0;

pub fn run(a: GameArgs) -> Result<AppExit> {
    use ssx_ui::game::fe::host::parse_script;
    let lang = Language::parse(&a.lang).with_context(|| format!("--lang {}", a.lang))?;
    crate::render::fx_states::init(&a.big_root);
    let (mut game, scene) = boot_ui(&a.big_root, lang)?;
    let mut fe = ssx_ui::game::fe::FrontEnd::new(&a.big_root, Profile::new())?;
    let schema = SaveSchema::load(&a.big_root, &fe.career)?;
    let stat_names = std::sync::Arc::new(ssx_assets::career::save::modelled_stats(&schema));
    let (save, profile, note) = SaveSlot::load(a.save.clone().unwrap_or_else(default_save_path), schema)?;
    fe.saveload.load_error = save.damaged.then_some(ssx_ui::game::fe::saveload::LoadError::Corrupt);
    game.fe = Some(fe);
    let stored_audio = save.profile_into(&mut game, profile);
    let mut segments: std::collections::VecDeque<_> =
        a.script.split('|').map(parse_script).collect::<Result<_>>().context("--script")?;
    let mut host = Host::new(&a.big_root, segments.pop_front().unwrap_or_default());
    host.phase = HostPhase::Riding;
    host.external_movies = true;
    host.external_pause = true;
    host.movie_cap = a.movie_cap.unwrap_or(f64::INFINITY);
    let mut globe_note: Option<Result<String, String>> = None;
    let globe_assets = game.fe.as_ref().and_then(|fe| fe.globe_scene.as_ref()).and_then(|s| {
        ui::globe::GlobeAssets::load(&a.big_root, &s.data)
            .map(std::sync::Arc::new)
            .inspect(|g| globe_note = Some(Ok(g.texture_line())))
            .inspect_err(|e| globe_note = Some(Err(format!("{e:#}"))))
            .ok()
    });
    let fe_audio_data = std::sync::Mutex::new(fe_audio::load(Some(&a.big_root)));
    start_failure_hook();
    let mut app = App::new();
    app.add_plugins((
        DefaultPlugins
            .set(WindowPlugin { primary_window: Some(a.window.window("Project Avalanche")), ..default() })
            .set(level::render_plugin())
            .disable::<bevy::audio::AudioPlugin>(),
        FrameTimeDiagnosticsPlugin::default(),
        ui::UiPlugin,
        ui::globe::GlobePlugin,
    ));
    app.add_systems(Startup, report_gpu);
    app.add_systems(Startup, move || match &globe_note {
        Some(Ok(line)) => info!("globe assets: {line}"),
        Some(Err(e)) => error!("globe assets: {e}"),
        None => {}
    });
    level::register(&mut app, level::Registration { wireframe: a.wireframe, viewer_hud: a.debug_hud });
    crate::audio::music::register(&mut app);
    crate::audio::sfx::register(&mut app);
    fe_audio::register(&mut app);
    game_level::register(&mut app);
    let test_run = !a.script.is_empty()
        || !a.level_scripts.is_empty()
        || a.autoplay
        || a.follow_prompts
        || a.launch.is_some()
        || a.seconds.is_some()
        || a.rides.is_some();
    let real_focus = std::env::var("SSX_SHELL_REAL_FOCUS").is_ok_and(|v| v == "1");
    app.insert_resource(PadSlots::new(test_run, !test_run || real_focus));
    app.insert_resource(scene)
        .insert_resource(ui::globe::GlobeView { assets: globe_assets, ..default() })
        .insert_resource(WinitSettings::game())
        .insert_resource(WindowHidden::from_env())
        .init_resource::<UiPipelinesReady>()
        .init_resource::<WorldCheck>();
    let (w, h) = (a.window.width as f32, a.window.height as f32);
    let movie_layer = RenderLayers::layer(MOVIE_LAYER);
    let movie_sprite = app
        .world_mut()
        .spawn((Sprite { custom_size: Some(Vec2::new(w, h)), ..default() }, Visibility::Hidden, movie_layer.clone()))
        .id();
    let movie_camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Camera { order: 1, clear_color: ClearColorConfig::None, is_active: false, ..default() },
            Msaa::Off,
            movie_layer,
            ui::fade::FadeCamera,
        ))
        .id();
    app.insert_resource(GameUiRes(game))
        .insert_resource(Shell {
            host,
            big_root: a.big_root.clone(),
            stage: Stage::Menus,
            save,
            stat_names,
            loading: None,
            op: std::sync::Mutex::new(None),
            op_audio: false,
            stats_in: None,
            op_stats: false,
            launch: None,
            rides: 0,
            rides_done: 0,
            segments,
            segment_started: false,
            max_rides: a.rides,
            ride_limits: a.ride_limits.clone(),
            ride_shots: a.ride_shots.clone(),
            level_scripts: a.level_scripts.clone(),
            seconds: a.seconds,
            movie_cap: a.movie_cap.unwrap_or(f64::INFINITY),
            shot_dir: a.shot_dir.clone(),
            autoplay: a.autoplay,
            follow_prompts: a.follow_prompts,
            ride_win: a.ride_win,
            fade_shots: a.fade_shots,
            fade_frames: 0,
            fade_last: 0.0,
            debug_hud: a.debug_hud,
            subdivide: a.subdivide,
            shading: a.shading,
            no_objects: a.no_objects,
            aspect: w / h,
            movie: None,
            movie_ended: None,
            lang: a.lang.clone(),
            last_stick: Vec2::ZERO,
            stick_dpad: StickDpad::default(),
            movie_sprite,
            movie_camera,
            ready_frames: 0,
            started: false,
            hold: 0,
            pending_shots: Vec::new(),
            log_seen: (0, 0),
            notes: vec![note],
            done_at: None,
            test_launch: a.launch.clone(),
            launch_start_at: 0.0,
            popup_reported: false,
            frame_sleep: std::env::var("SSX_SHELL_FRAME_SLEEP_MS").ok().and_then(|v| v.parse().ok()),
            input_log: std::env::var_os("SSX_SHELL_INPUT_LOG").is_some(),
        })
        .add_systems(Startup, move |world: &mut World| {
            spawn_front_end_camera(world);
            let data = fe_audio_data.lock().ok().and_then(|mut d| d.take());
            fe_audio::start(world, data);
            let values = settings_values(&world.resource::<GameUiRes>().values);
            fe_audio::apply_audio_settings(world, &audio_values(&values));
            if let Some(a) = stored_audio {
                fe_audio::apply_stored_settings(world, &a);
            }
            let stats = level::stats(world);
            info!("shell: the front end's world: {stats}");
        })
        .add_systems(
            PreUpdate,
            (hidden_frame, pads_frame, shell_world, game_level::pause_frame).chain().after(bevy::input::InputSystems),
        )
        .add_systems(
            Update,
            (game_level::level_frame, shell_input, shell_step).chain().after(crate::ride::RideStepSet),
        );
    app.sub_app_mut(RenderApp).add_systems(ExtractSchedule, report_pipelines);
    Ok(app.run())
}

fn audio_values(list: &[(u16, i32)]) -> Vec<(u32, i32)> {
    list.iter().filter(|(id, _)| (0x21..=0x2A).contains(id)).map(|&(id, v)| (u32::from(id), v)).collect()
}
