use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_assets::setup;
use ssx_assets::texture::{DxnExport, decode_resource};
use ssx_formats::ea_movie::Movie;
use ssx_formats::vp6::Decoder;
use ssx_formats::xpr::XprFile;

use super::Slot;
use super::image::Picture;

// TODO(guess): the rectangles and frame numbers were measured on the one disc we have (USA, En / Fr / De / Es / It); other pressings with the same atlas size and frame count are assumed to match (sizes are checked, contents cannot be); settle: a second disc image.

const LOGO_XPR: &str = "bootcache/data/common/fe/views/bootlogo_i6.xpr";
const LOGO_ATLAS: (usize, usize) = (991, 981);
const LOGO_RECT: [usize; 4] = [1, 393, 593, 380];
const SPLASH_XPR: [&str; 3] = [
    "bootcache/data/xenon/gameexplorer/frontend/textures/fe_splashscreen_eu.xpr",
    "bootcache/data/xenon/gameexplorer/frontend/textures/fe_splashscreen_na_eng.xpr",
    "bootcache/data/xenon/gameexplorer/frontend/textures/fe_splashscreen_na_fre.xpr",
];
const SPLASH_BAND: [usize; 4] = [0, 8, 1280, 412];

pub const MOVIE: &str = "intromovie.vp6";
const MOVIE_FRAMES: usize = 2568;
const MOVIE_SIZE: (usize, usize) = (1280, 720);
struct Shot {
    slot: Slot,
    frame: usize,
    crop: [usize; 4],
    size: (usize, usize),
}

const fn shot(slot: Slot, frame: usize, crop: [usize; 4], size: (usize, usize)) -> Shot {
    Shot { slot, frame, crop, size }
}

const MOVIE_SHOTS: [Shot; 4] = [
    shot(Slot::Background, 180, [0, 0, 1280, 720], (1280, 720)),
    shot(Slot::TileMods, 500, [336, 40, 760, 486], (576, 368)),
    shot(Slot::TileSettings, 1756, [180, 60, 940, 600], (576, 368)),
    shot(Slot::TileExtras, 2128, [140, 81, 1000, 639], (576, 368)),
];

fn xpr(path: &Path) -> Result<Picture> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let file = XprFile::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    let resource = file.resources.first().context("no texture in the file")?;
    let level = decode_resource(&data, &file, resource, false, DxnExport::Raw)?.into_iter().next();
    let image = level.context("no level decoded")?.image;
    Picture::new(image.width as usize, image.height as usize, image.rgba)
}

pub fn logo(big_root: &Path) -> Result<(Picture, String)> {
    let atlas = xpr(&big_root.join(LOGO_XPR))?;
    if (atlas.width, atlas.height) != LOGO_ATLAS {
        bail!("{LOGO_XPR} is {} x {}, not the atlas the logo was measured in", atlas.width, atlas.height);
    }
    let [x, y, w, h] = LOGO_RECT;
    Ok((atlas.crop(x, y, w, h), format!("{LOGO_XPR} [{x}, {y}, {w}, {h}]")))
}

pub fn hero(big_root: &Path) -> Result<(Picture, String)> {
    let (name, splash) = SPLASH_XPR
        .iter()
        .find_map(|n| xpr(&big_root.join(n)).ok().map(|p| (n, p)))
        .context("no splash screen in bootcache")?;
    if (splash.width, splash.height) != MOVIE_SIZE {
        bail!("{name} is {} x {}, not 1280 x 720", splash.width, splash.height);
    }
    let [x, y, w, h] = SPLASH_BAND;
    Ok((splash.crop(x, y, w, h), format!("{name} [{x}, {y}, {w}, {h}]")))
}

pub fn movie_file(game_data: &Path) -> PathBuf {
    setup::disc_root(game_data).join(setup::MOVIE_DIR).join(MOVIE)
}

pub fn movie_shots(file: &Path, wanted: impl Fn(Slot) -> bool) -> Result<Vec<(Slot, Picture, String)>> {
    let shots: Vec<&Shot> = MOVIE_SHOTS.iter().filter(|s| wanted(s.slot)).collect();
    if shots.is_empty() {
        return Ok(Vec::new());
    }
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let movie = Movie::parse(&data).context("parsing the intro movie")?;
    let size = (movie.video.width as usize, movie.video.height as usize);
    if movie.frames.len() != MOVIE_FRAMES || size != MOVIE_SIZE {
        bail!(
            "{MOVIE} has {} frames of {} x {}: not the movie the shots were chosen in",
            movie.frames.len(),
            size.0,
            size.1
        );
    }
    let mut out = Vec::new();
    for &&Shot { slot, frame, crop: [x, y, w, h], size: (tw, th) } in &shots {
        let key = (0..=frame).rev().find(|&i| movie.frames[i].key).unwrap_or(0);
        let mut decoder = Decoder::new();
        for f in &movie.frames[key..=frame] {
            decoder.decode_frame(f.data).with_context(|| format!("decoding frame {frame} of {MOVIE}"))?;
        }
        let mut rgba = Vec::new();
        decoder.picture().context("no picture decoded")?.write_rgba(&mut rgba);
        let picture = Picture::new(size.0, size.1, rgba)?.crop(x, y, w, h).resize(tw, th);
        out.push((slot, picture, format!("disc/{}/{MOVIE} frame {frame} [{x}, {y}, {w}, {h}]", setup::MOVIE_DIR)));
    }
    Ok(out)
}
