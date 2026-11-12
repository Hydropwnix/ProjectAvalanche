use std::fs::{self, File};
use std::path::PathBuf;

use anyhow::{Context, Result};
use ssx_formats::xdvdfs::Disc;

const USAGE: &str = "usage: xex-dump <disc image .iso> <output folder>";

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let iso = PathBuf::from(args.next().context(USAGE)?);
    let out = PathBuf::from(args.next().context(USAGE)?);
    let file = File::open(&iso).with_context(|| format!("opening {}", iso.display()))?;
    let mut disc = Disc::open(file).with_context(|| format!("{} is not an Xbox 360 disc image", iso.display()))?;
    let extent = disc.find("default.xex").context("the image has no default.xex")?.extent();
    let xex = disc.read_extent(extent)?;
    fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let xex_path = out.join("default.xex");
    fs::write(&xex_path, &xex).with_context(|| format!("writing {}", xex_path.display()))?;
    let d = ssx_assets::exe::dump_xex(&xex_path, &out)?;
    let info = format!(
        "name={}\nimage={}\nload_address={:#010x}\nentry_point={:#010x}\nimage_size={:#x}\n",
        d.pe_name,
        d.image_path.display(),
        d.load_address,
        d.entry_point,
        d.image_size
    );
    fs::write(out.join("xex-info.txt"), &info)?;
    print!("{info}");
    Ok(())
}
