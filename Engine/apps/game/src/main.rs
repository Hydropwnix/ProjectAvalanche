use std::path::PathBuf;

use anyhow::{Context, Result};
use bevy::prelude::*;
use clap::Parser;
use ssx_bevy::{RenderArgs, WINDOW_SIZE, game_mode, terrain_look};

/// The game: the front end from the boot movie to a drop and back in one window, the profile loaded and saved
/// (`Docs/architecture/game-shell.md`). The data come from --big-root (or the default search). Keys: Enter START,
/// Space A, Backspace B, X, Y, arrows the D-pad, Q / E the bumpers, W / A / S / D the globe's stick (or a gamepad);
/// riding: the ride's controls, Esc the pause menu.
#[derive(Parser, Debug)]
#[command(name = "avalanche-game", version)]
struct Cli {
    /// The asset viewer's former switch for the game (`asset-viewer --game`): accepted and ignored, so the command
    /// lines of before keep working.
    #[arg(long = "game", hide = true)]
    _game: bool,

    /// Extracted archive root holding cache1/ (vaults) and cache3/ (textures).
    /// Defaults to the one above Game-Data/Extracted/big/data0/data/xenon/gameexplorer/alps0101, searched for upwards
    /// from the current directory, then in the launcher's data folder (`ssx-assets setup` output).
    #[arg(long, value_name = "DIR")]
    big_root: Option<PathBuf>,

    #[command(flatten)]
    render: RenderArgs,

    /// The profile file (default `<app dir>/Saves/ssxvprofile`, the launcher's folder).
    #[arg(long, value_name = "FILE")]
    save: Option<PathBuf>,

    /// The text language (`eng_us`, `fre_fr`, `ger_de`, `ita_it`, `spa_es`, `jpn_jp`).
    #[arg(long, value_name = "LANG", default_value = "eng_us")]
    lang: String,

    /// The window in pixels at scale factor 1 (e.g. `1280x720`; default 1600x900 logical).
    #[arg(long, value_name = "WxH")]
    window: Option<String>,

    /// A test option: pad presses `<seconds>:<step>,…` on the front end's clock (`start`, `a`, `b`, `x`, `y`, `up`,
    /// `down`, `left`, `right`, `lb`, `rb`, `skip`, `shot=NAME`, `nav=EVENT`); `|` starts the next segment, timed
    /// from the next return from a drop.
    #[arg(long, value_name = "SCRIPT", allow_hyphen_values = true)]
    script: Option<String>,

    /// Test options: a movie's seconds at most; the front end's seconds to run; the drops to play before the session
    /// ends; per drop a ride time limit in seconds (0 = none); screenshots at these ride seconds.
    #[arg(long, value_name = "S")]
    movie_cap: Option<f64>,
    #[arg(long, value_name = "S")]
    seconds: Option<f64>,
    #[arg(long, value_name = "N")]
    rides: Option<u32>,
    #[arg(long, value_name = "S[,S...]", value_delimiter = ',')]
    ride_limit: Vec<f32>,
    #[arg(long, value_name = "S[,S...]", value_delimiter = ',')]
    ride_shots: Vec<f32>,
    /// A test option: pad presses in each level, `;` between the levels (`<seconds>:<step>,…` as --script, seconds
    /// since the level came up; `start` is the pause).
    #[arg(long, value_name = "SCRIPTS", allow_hyphen_values = true)]
    level_script: Option<String>,

    /// Where `shot=NAME` and the ride screenshots go (default: the development tree's Game-Data/Converted/screenshots,
    /// else `<app dir>/Screenshots`).
    #[arg(long, value_name = "DIR")]
    shot_dir: Option<PathBuf>,

    /// A test option: launch this `ui_drop` directly (as the globe's tutorial branch does, without the equipment
    /// screen) at this front-end second, e.g. `exp_alps_grandgolliat_drop_1_race@12`.
    #[arg(long, value_name = "UI_DROP@S")]
    launch: Option<String>,

    /// A test option: the drops are ridden by the HUD tools' scripted player.
    #[arg(long)]
    autoplay: bool,

    /// A test option: the drops are played by a player that reads the screen's prompts (a tutorial popup, the
    /// tutorial HUD's buttons, the start message) and presses the keyboard keys they name.
    #[arg(long, conflicts_with = "autoplay")]
    follow_prompts: bool,

    /// A test option: a ride without a result after S ride seconds ends won — a result that beats every rival of the
    /// event (the window's counterpart of the example `fe_run --save IN --complete DROP`, for the front end's flow
    /// after a drop the test player cannot finish).
    #[arg(long, value_name = "S")]
    ride_win: Option<f32>,

    /// The asset viewer's debug text over the ride (off: the game's HUD only).
    #[arg(long)]
    debug_hud: bool,

    /// A test option: screenshots across the screen fades — every third frame while the fade is up (its alpha above
    /// 0) and the frame it ends, at most N (`fade_<FE seconds>_<alpha>.png` in the shot folder).
    #[arg(long, value_name = "N")]
    fade_shots: Option<u32>,
}

fn main() -> AppExit {
    match game_args(Cli::parse()).and_then(game_mode::run) {
        Ok(exit) => exit,
        Err(e) => {
            eprintln!("error: {e:#}");
            AppExit::error()
        }
    }
}

fn game_args(cli: Cli) -> Result<game_mode::GameArgs> {
    let big_root = match cli.big_root {
        Some(r) => r,
        None => {
            let needs = "the game needs the extracted archive root (--big-root)";
            let track_dir = ssx_bevy::find_default_track().context(needs)?;
            terrain_look::find_big_root(&track_dir).context(needs)?
        }
    };
    let vsync = !cli.render.no_vsync;
    let window = match &cli.window {
        Some(s) => {
            let (w, h) = s.split_once('x').with_context(|| format!("--window {s}: WxH"))?;
            game_mode::WindowSpec { width: w.parse()?, height: h.parse()?, physical: true, vsync, position: None }
        }
        None => game_mode::WindowSpec {
            width: WINDOW_SIZE.x,
            height: WINDOW_SIZE.y,
            physical: false,
            vsync,
            position: None,
        },
    };
    let shot_dir = cli.shot_dir.unwrap_or_else(|| {
        let cwd = std::env::current_dir().unwrap_or_default();
        let found = cwd.ancestors().map(|d| d.join("Game-Data/Converted/screenshots")).find(|p| p.is_dir());
        found.unwrap_or_else(|| ssx_assets::setup::app_dir().join("Screenshots"))
    });
    Ok(game_mode::GameArgs {
        big_root,
        save: cli.save,
        lang: cli.lang,
        window,
        script: cli.script.unwrap_or_default(),
        movie_cap: cli.movie_cap,
        seconds: cli.seconds,
        rides: cli.rides,
        ride_limits: cli.ride_limit,
        ride_shots: cli.ride_shots,
        level_scripts: cli
            .level_script
            .as_deref()
            .map_or_else(Vec::new, |s| s.split(';').map(str::to_string).collect()),
        shot_dir,
        autoplay: cli.autoplay,
        follow_prompts: cli.follow_prompts,
        ride_win: cli.ride_win,
        debug_hud: cli.debug_hud,
        fade_shots: cli.fade_shots.unwrap_or(0),
        subdivide: cli.render.subdivide,
        shading: cli.render.shading,
        no_objects: cli.render.no_objects,
        wireframe: cli.render.wireframe,
        launch: match &cli.launch {
            Some(s) => {
                let (name, at) = s.rsplit_once('@').with_context(|| format!("--launch {s}: UI_DROP@SECONDS"))?;
                Some((name.to_string(), at.parse().with_context(|| format!("--launch {s}"))?))
            }
            None => None,
        },
    })
}
