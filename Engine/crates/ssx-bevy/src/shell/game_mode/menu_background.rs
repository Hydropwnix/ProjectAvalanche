//! The menus' own background (a port addition for mods: `chooseMenuBackground`): a picture or a video the player
//! picks on this PC, shown behind the globe in place of its backdrop, at once, and again at the next start. Every pick
//! is kept in a library ("MenuBackground\Library" in the game's folder) with a small preview, to go back to later.
//!
//! Picking, reading a picture and making a video playable run on a thread of their own (the game keeps running); the
//! result comes back here. A video the game can't play as it is (anything but AV1 in WebM up to 2560 x 1440, 60 fps)
//! is converted with ffmpeg when it's installed (up to 2560 x 1440, high quality): its first two minutes, no sound. The picked file is never
//! changed: the library keeps its own copy. A video's preview is its first picture, taken when it first plays.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::{Assets, Handle, Image, error, info};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssx_core::menu_background::{self as bg, Entry, Order, Status};

use crate::ui;

const PICTURES: &[&str] = &["png", "jpg", "jpeg", "webp", "bmp", "gif", "tga", "tif", "tiff"];
const VIDEOS: &[&str] = &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "3gp"];
/// The largest picture kept (a bigger one is scaled down to fit).
const MAX_PICTURE: (u32, u32) = (3840, 2160);
/// How much of a converted video is kept, in seconds (it loops).
const MAX_SECONDS: f64 = 120.0;
/// A preview's size (16:9; the picture is cropped to fill it).
const THUMB: (u32, u32) = (192, 108);
const CHOICE: &str = "choice.txt";
const INDEX: &str = "index.tsv";

enum Done {
    Picture {
        id: String,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
    Video {
        id: String,
        path: PathBuf,
    },
    Cleared,
    /// Nothing changes (the picker closed, or it failed: the status says so).
    Nothing,
}

#[derive(Default)]
pub(super) struct UserBackground {
    started: bool,
    /// The work running (held in a lock: the game's state is shared between threads).
    work: Option<Mutex<Receiver<Done>>>,
    picture: Option<Handle<Image>>,
    video: Option<PathBuf>,
    movie: Option<ui::movie::MoviePlayback>,
    /// The video's last picture (kept while it opens again for its loop).
    shown: Option<Handle<Image>>,
    /// The showing video's library id while it has no preview yet (taken from its first picture).
    needs_thumb: Option<String>,
}

fn folder(big_root: &Path) -> PathBuf {
    big_root.parent().unwrap_or(big_root).join("MenuBackground")
}

fn library_dir(dir: &Path) -> PathBuf {
    dir.join("Library")
}

fn ext_of(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn status(state: &'static str, kind: &'static str, name: &str, progress: f32, message: impl Into<String>) {
    bg::set_status(Status { state, kind, name: name.to_string(), progress, message: message.into(), id: String::new() });
}

fn status_of(e: &Kept, state: &'static str, message: impl Into<String>) {
    bg::set_status(Status {
        state,
        kind: e.kind,
        name: e.name.clone(),
        progress: if state == "ready" { 1.0 } else { 0.0 },
        message: message.into(),
        id: e.id.clone(),
    });
}

impl UserBackground {
    /// Takes the scripts' orders and the finished work; the picture to show behind the globe (`wanted`: the globe
    /// shows this frame, so a video plays), or None for the game's own (or a mod package's) background.
    pub(super) fn step(
        &mut self,
        big_root: &Path,
        wanted: bool,
        dt: f64,
        images: &mut Assets<Image>,
    ) -> Option<Handle<Image>> {
        if !self.started {
            self.started = true;
            let dir = folder(big_root);
            bg::set_thumb_dir(library_dir(&dir));
            self.spawn(move || load_saved(&dir));
        }
        for order in bg::take_orders() {
            if self.work.is_some() {
                continue; // (one thing at a time)
            }
            let dir = folder(big_root);
            match order {
                Order::Choose => self.spawn(move || choose(&dir)),
                Order::Clear => self.spawn(move || clear(&dir)),
                Order::Use(id) => self.spawn(move || use_kept(&dir, &id)),
                Order::Delete(id) => self.spawn(move || delete(&dir, &id)),
            }
        }
        let polled = match &self.work {
            Some(m) => match m.lock() {
                Ok(rx) => Some(rx.try_recv()),
                Err(_) => Some(Err(TryRecvError::Disconnected)),
            },
            None => None,
        };
        match polled {
            Some(Ok(done)) => {
                self.work = None;
                self.take(done, images);
            }
            Some(Err(TryRecvError::Disconnected)) => self.work = None,
            Some(Err(TryRecvError::Empty)) | None => {}
        }
        if let Some(p) = &self.picture {
            return Some(p.clone());
        }
        let path = self.video.clone()?;
        if !wanted {
            self.movie = None; // (it starts again when the globe shows)
            return self.shown.clone();
        }
        if let Some(m) = self.movie.as_mut()
            && m.advance(dt, images)
        {
            if let Some((_, _, why)) = m.failure() {
                error!("shell: the menus' own background ({}) stopped: {why}", path.display());
                let s = bg::status();
                bg::set_status(Status { state: "failed", progress: 0.0, message: format!("The video stopped playing: {why}"), ..s });
                self.video = None;
                self.shown = None;
            }
            self.movie = None; // (its end: it opens again, for the loop)
        }
        if self.movie.is_none() && self.video.is_some() {
            match ui::movie::MoviePlayback::open_file("menus' own background", &path, images) {
                Ok(m) => self.movie = Some(m),
                Err(e) => {
                    error!("shell: the menus' own background ({}): {e:#}", path.display());
                    let s = bg::status();
                    bg::set_status(Status { state: "failed", progress: 0.0, message: format!("The video can't be played: {e}"), ..s });
                    self.video = None;
                    self.shown = None;
                    return None;
                }
            }
        }
        if let Some(m) = &self.movie
            && m.frame() > 0
        {
            self.shown = Some(m.image.clone());
            // Its first picture: the video's preview, when it has none yet.
            if self.needs_thumb.is_some()
                && let Some(img) = images.get(&m.image)
                && let Some(data) = img.data.clone()
                && let Some(id) = self.needs_thumb.take()
            {
                let (w, h) = (img.width(), img.height());
                let dir = library_dir(&folder(big_root));
                std::thread::spawn(move || {
                    if let Some(rgba) = image::RgbaImage::from_raw(w, h, data) {
                        save_thumb(&image::DynamicImage::ImageRgba8(rgba), &dir.join(format!("{id}.thumb.png")));
                        publish(&dir);
                    }
                });
            }
        }
        self.shown.clone()
    }

    fn spawn(&mut self, work: impl FnOnce() -> Done + Send + 'static) {
        let (tx, rx) = channel();
        self.work = Some(Mutex::new(rx));
        std::thread::spawn(move || {
            let _ = tx.send(work());
        });
    }

    fn take(&mut self, done: Done, images: &mut Assets<Image>) {
        match done {
            Done::Picture { id, rgba, width, height } => {
                let size = Extent3d { width, height, depth_or_array_layers: 1 };
                self.picture = Some(images.add(Image::new(
                    size,
                    TextureDimension::D2,
                    rgba,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                )));
                self.video = None;
                self.movie = None;
                self.shown = None;
                self.needs_thumb = None;
                info!("shell: the menus' own background: a picture ({width} x {height}, {id})");
            }
            Done::Video { id, path } => {
                info!("shell: the menus' own background: a video ({})", path.display());
                let thumb = path.with_file_name(format!("{id}.thumb.png"));
                self.needs_thumb = (!thumb.is_file()).then_some(id);
                self.picture = None;
                self.movie = None;
                self.shown = None;
                self.video = Some(path);
            }
            Done::Cleared => {
                info!("shell: the menus' own background cleared: the game's own");
                self.picture = None;
                self.video = None;
                self.movie = None;
                self.shown = None;
                self.needs_thumb = None;
            }
            Done::Nothing => {}
        }
    }
}

// ----------------------------------------------------------------------------------------------- the library ----

/// A kept background: a line of the library's index.
#[derive(Clone)]
struct Kept {
    id: String,
    kind: &'static str,
    file: String,
    name: String,
}

fn read_index(lib: &Path) -> Vec<Kept> {
    let text = std::fs::read_to_string(lib.join(INDEX)).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let (id, kind, file, name) = (f.next()?, f.next()?, f.next()?, f.next().unwrap_or(""));
            let kind = match kind {
                "image" => "image",
                "video" => "video",
                _ => return None,
            };
            let ok = |s: &str| !s.is_empty() && !s.contains(['/', '\\']) && s != "." && s != "..";
            (ok(id) && ok(file) && lib.join(file).is_file()).then(|| Kept {
                id: id.to_string(),
                kind,
                file: file.to_string(),
                name: name.to_string(),
            })
        })
        .collect()
}

fn write_index(lib: &Path, list: &[Kept]) {
    let text: String = list
        .iter()
        .map(|k| {
            let clean = |s: &str| s.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect::<String>();
            format!("{}\t{}\t{}\t{}\n", k.id, k.kind, clean(&k.file), clean(&k.name))
        })
        .collect();
    let _ = std::fs::create_dir_all(lib);
    let _ = std::fs::write(lib.join(INDEX), text);
}

/// Tells the scripts what the library holds.
fn publish(lib: &Path) {
    let list = read_index(lib)
        .into_iter()
        .map(|k| Entry {
            thumb: lib.join(format!("{}.thumb.png", k.id)).is_file(),
            id: k.id,
            kind: k.kind,
            name: k.name,
        })
        .collect();
    bg::set_library(list);
}

/// A new library id (lower-case letters and digits, as texture ids need).
fn new_id() -> String {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    format!("b{ms}")
}

/// Puts a new entry at the top of the library.
fn add_kept(lib: &Path, k: Kept) {
    let mut list = read_index(lib);
    list.retain(|o| o.id != k.id);
    list.insert(0, k);
    write_index(lib, &list);
    publish(lib);
}

/// A preview: the picture cropped to fill 16:9 and scaled down.
fn save_thumb(img: &image::DynamicImage, to: &Path) {
    let (tw, th) = THUMB;
    let thumb = img.resize_to_fill(tw, th, image::imageops::FilterType::Triangle);
    if let Err(e) = thumb.to_rgba8().save_with_format(to, image::ImageFormat::Png) {
        error!("shell: the menus' background preview ({}): {e}", to.display());
    }
}

fn write_choice(dir: &Path, id: &str) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join(CHOICE), format!("{id}\n"));
}

/// Shows a kept entry (and keeps it as the choice for next time).
fn show(dir: &Path, k: &Kept) -> Done {
    let lib = library_dir(dir);
    let path = lib.join(&k.file);
    match k.kind {
        "image" => match read_picture(&path) {
            Ok((rgba, width, height)) => {
                write_choice(dir, &k.id);
                status_of(k, "ready", "");
                Done::Picture { id: k.id.clone(), rgba, width, height }
            }
            Err(e) => {
                status_of(k, "failed", format!("The picture can't be read: {e}"));
                Done::Nothing
            }
        },
        _ => {
            write_choice(dir, &k.id);
            status_of(k, "ready", "");
            Done::Video { id: k.id.clone(), path }
        }
    }
}

// ------------------------------------------------------------------------------------------------- the work ----

/// The library and the background kept from last time (a background kept before the library is moved into it).
fn load_saved(dir: &Path) -> Done {
    let lib = library_dir(dir);
    let choice = std::fs::read_to_string(dir.join(CHOICE)).unwrap_or_default();
    let lines: Vec<&str> = choice.lines().collect();
    let mut chosen = lines.first().map(|s| s.trim().to_string()).unwrap_or_default();
    if lines.len() >= 2 && matches!(lines[0], "image" | "video") {
        // (the first version's choice: kind, file, name)
        let old = dir.join(lines[1]);
        let id = new_id();
        let kind = if lines[0] == "image" { "image" } else { "video" };
        let file = format!("{id}.{}", ext_of(&old));
        let _ = std::fs::create_dir_all(&lib);
        if old.is_file() && std::fs::rename(&old, lib.join(&file)).is_ok() {
            if kind == "image"
                && let Ok(img) = image::ImageReader::open(lib.join(&file)).and_then(|r| r.with_guessed_format())
                && let Ok(img) = img.decode()
            {
                save_thumb(&img, &lib.join(format!("{id}.thumb.png")));
            }
            add_kept(&lib, Kept { id: id.clone(), kind, file, name: lines.get(2).unwrap_or(&"").to_string() });
            chosen = id;
        } else {
            chosen.clear();
        }
        write_choice(dir, &chosen);
    }
    publish(&lib);
    match read_index(&lib).into_iter().find(|k| k.id == chosen) {
        Some(k) => show(dir, &k),
        None => {
            status("none", "", "", 0.0, "");
            Done::Nothing
        }
    }
}

fn clear(dir: &Path) -> Done {
    write_choice(dir, "none");
    status("none", "", "", 0.0, "");
    Done::Cleared
}

fn use_kept(dir: &Path, id: &str) -> Done {
    match read_index(&library_dir(dir)).into_iter().find(|k| k.id == id) {
        Some(k) => show(dir, &k),
        None => Done::Nothing,
    }
}

fn delete(dir: &Path, id: &str) -> Done {
    let lib = library_dir(dir);
    let mut list = read_index(&lib);
    let Some(at) = list.iter().position(|k| k.id == id) else { return Done::Nothing };
    let k = list.remove(at);
    let _ = std::fs::remove_file(lib.join(&k.file));
    let _ = std::fs::remove_file(lib.join(format!("{}.thumb.png", k.id)));
    write_index(&lib, &list);
    publish(&lib);
    if bg::status().id == id {
        return clear(dir);
    }
    Done::Nothing
}

fn choose(dir: &Path) -> Done {
    let before = bg::status();
    bg::set_status(Status {
        state: "picking",
        progress: 0.0,
        message: "Pick a picture or a video in the window that opened".into(),
        ..before.clone()
    });
    let mut all: Vec<&str> = PICTURES.to_vec();
    all.extend_from_slice(VIDEOS);
    let picked = rfd::FileDialog::new()
        .set_title("Project Avalanche: pick a picture or a video for the menus' background")
        .add_filter("Pictures and videos", &all[..])
        .add_filter("Pictures", PICTURES)
        .add_filter("Videos", VIDEOS)
        .pick_file();
    let Some(path) = picked else {
        bg::set_status(before); // (closed: as it was)
        return Done::Nothing;
    };
    let name = name_of(&path);
    let ext = ext_of(&path);
    let _ = std::fs::create_dir_all(library_dir(dir));
    let done = if PICTURES.contains(&ext.as_str()) {
        choose_picture(dir, &path, &name, &ext)
    } else {
        choose_video(dir, &path, &name)
    };
    if matches!(done, Done::Nothing) {
        // (failed: what showed before still shows)
        bg::set_status(Status { id: before.id, ..bg::status() });
    }
    done
}

fn read_picture(path: &Path) -> anyhow::Result<(Vec<u8>, u32, u32)> {
    let img = image::ImageReader::open(path)?.with_guessed_format()?.decode()?;
    let (mw, mh) = MAX_PICTURE;
    let img = if img.width() > mw || img.height() > mh {
        img.resize(mw, mh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    Ok((rgba.into_raw(), width, height))
}

fn choose_picture(dir: &Path, path: &Path, name: &str, ext: &str) -> Done {
    let lib = library_dir(dir);
    let img = match image::ImageReader::open(path).map_err(anyhow::Error::from).and_then(|r| {
        r.with_guessed_format().map_err(anyhow::Error::from)?.decode().map_err(anyhow::Error::from)
    }) {
        Ok(img) => img,
        Err(e) => {
            status("failed", "image", name, 0.0, format!("The picture can't be read: {e}"));
            return Done::Nothing;
        }
    };
    let id = new_id();
    let file = format!("{id}.{ext}");
    if let Err(e) = std::fs::copy(path, lib.join(&file)) {
        status("failed", "image", name, 0.0, format!("The picture couldn't be saved: {e}"));
        return Done::Nothing;
    }
    save_thumb(&img, &lib.join(format!("{id}.thumb.png")));
    let k = Kept { id, kind: "image", file, name: name.to_string() };
    add_kept(&lib, k.clone());
    show(dir, &k)
}

fn choose_video(dir: &Path, path: &Path, name: &str) -> Done {
    let lib = library_dir(dir);
    let id = new_id();
    let file = format!("{id}.webm");
    let out = lib.join(&file);
    let part = lib.join(format!("{id}.webm.part"));
    let keep = |dir: &Path| -> Done {
        let k = Kept { id: id.clone(), kind: "video", file: file.clone(), name: name.to_string() };
        add_kept(&library_dir(dir), k.clone());
        show(dir, &k)
    };
    // Already right for the game (AV1 in WebM, within its limits): kept as it is.
    if opens(path) {
        status("converting", "video", name, 0.0, "Copying the video");
        if std::fs::copy(path, &part).is_ok() && make_playable(&part) && std::fs::rename(&part, &out).is_ok() {
            return keep(dir);
        }
        let _ = std::fs::remove_file(&part);
    }
    let Some(ffmpeg) = find_ffmpeg() else {
        status(
            "failed",
            "video",
            name,
            0.0,
            "This video needs ffmpeg to be converted. Install it (PowerShell: winget install ffmpeg), restart the \
             game and pick the video again, or pick a picture or an AV1 WebM video.",
        );
        return Done::Nothing;
    };
    status("converting", "video", name, 0.0, "Making the video ready (its first two minutes)");
    let total = duration(&ffmpeg, path).map_or(MAX_SECONDS, |d| d.min(MAX_SECONDS)).max(0.1);
    // libaom first: its track header matches the one in the video (SVT-AV1's doesn't; `make_playable` fixes that).
    // High quality (it's on screen the whole time in the menus): a low CRF, a little slower than real time.
    let encoders: [&[&str]; 2] = [
        &["-c:v", "libaom-av1", "-usage", "realtime", "-cpu-used", "7", "-row-mt", "1", "-tiles", "2x2", "-crf", "18", "-b:v", "0", "-g", "240"],
        &["-c:v", "libsvtav1", "-preset", "8", "-crf", "20", "-g", "240"],
    ];
    let mut last = String::new();
    for enc in encoders {
        let _ = std::fs::remove_file(&part);
        match convert(&ffmpeg, path, &part, enc, total, name) {
            Ok(()) if make_playable(&part) => {
                if std::fs::rename(&part, &out).is_ok() {
                    return keep(dir);
                }
                last = "the converted video couldn't be kept".into();
            }
            Ok(()) => last = "the converted video isn't playable".into(),
            Err(e) => last = e,
        }
    }
    let _ = std::fs::remove_file(&part);
    status("failed", "video", name, 0.0, format!("The video couldn't be converted: {last}"));
    Done::Nothing
}

/// Whether the file is AV1 in WebM within the movie limits (its start; `playable` reads all of it).
fn opens(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    let Ok(len) = file.metadata().map(|m| m.len()) else { return false };
    ssx_content::movie::Demuxer::open(BufReader::new(file), len, &ssx_content::movie::Limits::menu_background()).is_ok()
}

/// Whether the game plays all of this file: every picture is read and checked, as playing it would.
fn playable(path: &Path) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    ssx_content::movie::walk(BufReader::new(file), len, &ssx_content::movie::Limits::menu_background())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Makes the background's own copy playable when it can: some encoders (SVT-AV1) write a sequence header in the
/// track's header that differs from the one in the video, which the game refuses; the track's is replaced with the
/// video's (same size, in place). True when the whole file then plays.
fn make_playable(path: &Path) -> bool {
    let first = playable(path);
    if first.is_ok() {
        return true;
    }
    let fixed = (|| -> Option<()> {
        let file = std::fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let mut d = ssx_content::movie::Demuxer::open(BufReader::new(file), len, &ssx_content::movie::Limits::menu_background()).ok()?;
        let track = d.info().video.config.sequence_payload.clone();
        let mut stream = None;
        while let Some(p) = d.next_packet().ok()? {
            if p.track != ssx_content::movie::Track::Video {
                continue;
            }
            let list = ssx_content::movie::av1::obus(&p.data, 4096).ok()?;
            let o = list.iter().find(|o| o.kind == ssx_content::movie::av1::OBU_SEQUENCE_HEADER)?;
            stream = Some(p.data[o.payload.0..o.payload.1].to_vec());
            break;
        }
        let stream = stream?;
        if stream == track || stream.len() != track.len() || track.is_empty() {
            return None;
        }
        let mut bytes = std::fs::read(path).ok()?;
        // The track's header comes before the first picture: its first copy in the file.
        let at = bytes.windows(track.len()).position(|w| w == track.as_slice())?;
        bytes[at..at + track.len()].copy_from_slice(&stream);
        std::fs::write(path, &bytes).ok()
    })();
    match (fixed, playable(path)) {
        (Some(()), Ok(())) => {
            info!("shell: the menus' own background: the video's track header was made to match its pictures");
            true
        }
        (_, r) => {
            error!("shell: the menus' own background isn't playable: {}", r.err().unwrap_or_default());
            false
        }
    }
}

fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    cmd
}

/// ffmpeg on the PATH, or where winget puts it.
fn find_ffmpeg() -> Option<PathBuf> {
    let mut candidates = vec![PathBuf::from("ffmpeg")];
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Microsoft").join("WinGet").join("Links").join("ffmpeg.exe"));
    }
    candidates.into_iter().find(|c| {
        quiet(Command::new(c).arg("-version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()))
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// The video's length in seconds, from ffmpeg's look at it.
fn duration(ffmpeg: &Path, path: &Path) -> Option<f64> {
    let out = quiet(Command::new(ffmpeg).args(["-hide_banner", "-nostdin", "-i"]).arg(path).stdin(Stdio::null()))
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let at = text.find("Duration: ")? + "Duration: ".len();
    let t = text.get(at..at + 11)?;
    let mut parts = t.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.trim().parse().ok()?;
    let s: f64 = parts.next()?.trim().parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Converts the video to AV1 in WebM for the game: its first two minutes, at most 2560 x 1440 and 60 fps, no sound.
fn convert(ffmpeg: &Path, input: &Path, out: &Path, encoder: &[&str], total: f64, name: &str) -> Result<(), String> {
    let seconds = format!("{MAX_SECONDS}");
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-nostdin", "-loglevel", "error", "-progress", "pipe:1", "-nostats", "-i"])
        .arg(input)
        .args(["-t", seconds.as_str(), "-an", "-sn", "-dn", "-map", "0:v:0"])
        .args([
            "-vf",
            "scale=w='min(2560,iw)':h='min(1440,ih)':force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos,format=yuv420p",
            "-fpsmax",
            "60",
        ])
        .args(encoder)
        .args(["-f", "webm"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = quiet(&mut cmd).spawn().map_err(|e| format!("ffmpeg didn't start: {e}"))?;
    let errors = child.stderr.take().map(|e| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(e), &mut s);
            s
        })
    });
    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms="))
                && let Ok(us) = v.trim().parse::<f64>()
            {
                let p = (us / 1e6 / total).clamp(0.0, 0.99) as f32;
                status("converting", "video", name, p, format!("Making the video ready: {:.0}%", p * 100.0));
            }
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    let errs = errors.and_then(|h| h.join().ok()).unwrap_or_default();
    if ok {
        Ok(())
    } else {
        let first = errs.lines().find(|l| !l.trim().is_empty()).unwrap_or("ffmpeg failed").trim().to_string();
        Err(first.chars().take(300).collect())
    }
}
