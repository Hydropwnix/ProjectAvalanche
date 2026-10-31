#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod args;
mod art;
mod catalog;
mod command;
mod jobs;
mod main_screen;
mod settings;
mod setup_flow;
mod ui;
mod update;

use std::path::Path;

use eframe::egui;

use crate::ui::{ExtrasTab, MenuItem, SettingsTab, layout};

fn main() -> eframe::Result {
    let started = std::time::Instant::now();
    let words: Vec<String> = std::env::args().skip(1).collect();
    if words.first().is_some_and(|w| w == "--apply-update") {
        std::process::exit(update::install::helper(&words[1..]));
    }
    update::install::cleanup_leftovers();
    let args = match args::Args::parse(&words) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    let settings = settings::Settings::load();
    if args.print {
        std::process::exit(print(&settings));
    }
    if let Some(dir) = &args.dump_art {
        std::process::exit(dump_art(&settings, dir));
    }
    let gallery = art::Gallery::start(app::art_request(&settings));
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Project Avalanche")
        .with_app_id("project-avalanche")
        .with_inner_size(args.window.unwrap_or([layout::CANVAS.x, layout::CANVAS.y]));
    if args.window.is_none() {
        viewport = viewport.with_min_inner_size([960.0, 540.0]);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Project Avalanche",
        options,
        Box::new(move |cc| {
            ui::theme::install(&cc.egui_ctx);
            let mut app = app::Launcher::new(
                &cc.egui_ctx,
                settings,
                args.screenshot.clone(),
                gallery,
                !args.no_update_check && update::auto_check_enabled(),
            );
            if let Some(page) = args.start_page() {
                app.go(page);
            }
            if let Some(tab) = &args.tab {
                app.settings_tab = SettingsTab::parse(tab).unwrap_or_default();
                app.extras_tab = ExtrasTab::parse(tab).unwrap_or_default();
            }
            if args.hover.as_deref() == Some(MenuItem::Exit.name()) {
                app.cursor = MenuItem::Exit;
            }
            app.forced_hover = args.hover.clone();
            app.window_pixels = args.window;
            app.scripted_keys = args.keys.iter().copied().collect();
            app.timing = std::env::var_os("PROJECT_AVALANCHE_TIMING").map(|_| started);
            app.auto_extract = args.extract;
            app.shot_after_setup = args.after_setup;
            app.auto_launch = args.launch;
            app.auto_launch_pending = args.launch;
            app.auto_play_pending = args.play;
            Ok(Box::new(app))
        }),
    )
}

fn print(s: &settings::Settings) -> i32 {
    println!("settings {}", settings::Settings::path().display());
    println!("game data {} (ready: {})", s.game_data().display(), s.data_ready());
    if !s.data_ready() {
        return 1;
    }
    let big = s.big_root();
    let tracks = catalog::tracks(&big);
    println!("{} tracks: {}", tracks.len(), tracks.join(" "));
    match catalog::riders(&big) {
        Ok(r) => println!(
            "{} riders: {}",
            r.len(),
            r.iter().map(|r| format!("{} ({})", r.name, r.label)).collect::<Vec<_>>().join(", ")
        ),
        Err(e) => println!("riders: {e:#}"),
    }
    match catalog::DropCache::default().get(&big, &s.track) {
        Ok(drops) => {
            for (i, d) in drops.iter().enumerate() {
                println!("  {} drop {}: {} peak {} lighting {}", s.track, i + 1, d.text_id, d.peak, d.lighting);
            }
            println!("  presets: {}", catalog::presets(drops).join(", "));
        }
        Err(e) => println!("  drops: {e}"),
    }
    println!("{}", command::viewer(s, &big).display());
    println!("{}", command::game(s, &big).display());
    0
}

fn dump_art(s: &settings::Settings, dir: &Path) -> i32 {
    if let Err(e) = std::fs::create_dir_all(dir) {
        eprintln!("error: {}: {e}", dir.display());
        return 1;
    }
    let started = std::time::Instant::now();
    let mut failed = false;
    let problems = art::load(&app::art_request(s), &mut |l| {
        let file = dir.join(format!("{}.png", l.slot.name()));
        let (w, h) = (l.picture.width, l.picture.height);
        println!(
            "{:>6.0} ms  {:<13} {:?}: {w} x {h}  {}",
            started.elapsed().as_secs_f64() * 1e3,
            l.slot.name(),
            l.source,
            l.note
        );
        if let Err(e) = l.picture.write_png(&file) {
            eprintln!("error: {e:#}");
            failed = true;
        }
    });
    for p in &problems {
        println!("note: {p}");
    }
    failed as i32
}

fn save_png(path: &Path, image: &egui::ColorImage) -> anyhow::Result<()> {
    let [w, h] = image.size;
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(file, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    enc.write_header()?.write_image_data(&rgba)?;
    Ok(())
}
