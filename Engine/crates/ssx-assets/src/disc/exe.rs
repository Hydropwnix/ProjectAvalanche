use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_formats::pe::PeImage;
use ssx_formats::xex::{self, Xex};

pub struct XexDump {
    pub pe_name: String,
    pub image_path: PathBuf,
    pub sections_path: PathBuf,
    pub imports_path: PathBuf,
    pub resources: Vec<(PathBuf, usize)>,
    pub load_address: u32,
    pub entry_point: u32,
    pub image_size: usize,
    pub blocks: usize,
    pub frames: usize,
    pub pages: u32,
    pub page_size: u32,
    pub pe: PeImage,
    pub import_counts: Vec<(String, usize)>,
}

pub fn dump_xex(xex_path: &Path, out: &Path) -> Result<XexDump> {
    let data = fs::read(xex_path).with_context(|| format!("reading {}", xex_path.display()))?;
    let x = Xex::parse(&data)?;
    let pe_name = x.original_pe_name()?.unwrap_or_else(|| "image.exe".into());
    let stem = Path::new(&pe_name).file_stem().map_or("image".into(), |s| s.to_string_lossy().into_owned());
    let entry_point = x.entry_point().context("XEX has no entry point header")?;
    let load_address = x.security.load_address;
    if x.image_base() != Some(load_address) {
        bail!("XEX image base {:x?} differs from the load address {load_address:#x}", x.image_base());
    }

    let image = x.image(&xex::RETAIL_KEY)?;
    let pages = x.verify_page_digests(&image.data)?;
    if pages as usize * x.security.page_size() as usize != image.data.len() {
        bail!("page descriptors cover {pages} pages, not the whole {:#x}-byte image", image.data.len());
    }
    let pe = PeImage::parse(&image.data)?;
    if pe.image_base != load_address {
        bail!("PE image base {:#x} differs from the XEX load address {load_address:#x}", pe.image_base);
    }
    pe.validate(image.data.len(), entry_point)?;

    let libraries = x.import_libraries()?;
    let records = x.import_records(&libraries, &image.data)?;
    let mut imports = String::from("library\tordinal\tkind\taddress\n");
    let mut import_counts = Vec::new();
    for (lib, recs) in libraries.iter().zip(&records) {
        for r in recs {
            let kind = match r.kind {
                0 => "variable",
                1 => "thunk",
                _ => "unknown",
            };
            writeln!(imports, "{}\t{:#05x}\t{kind}\t{:#010x}", lib.name, r.ordinal, r.address)?;
        }
        import_counts.push((lib.name.clone(), recs.len()));
    }

    let mut sections = String::from("name\tva\tvirtual_size\tcharacteristics\n");
    for s in &pe.sections {
        let va = load_address.wrapping_add(s.virtual_address);
        writeln!(sections, "{}\t{va:#010x}\t{:#x}\t{:#010x}", s.name, s.virtual_size, s.characteristics)?;
    }

    fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let image_path = out.join(format!("{stem}.bin"));
    let sections_path = out.join(format!("{stem}.sections.tsv"));
    let imports_path = out.join(format!("{stem}.imports.tsv"));
    fs::write(&image_path, &image.data)?;
    fs::write(&sections_path, sections)?;
    fs::write(&imports_path, imports)?;

    let mut resources = Vec::new();
    for res in x.resources()? {
        let bytes = x.resource_data(&res, &image.data)?;
        let name: String =
            res.name.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' }).collect();
        let dir = out.join("resources");
        fs::create_dir_all(&dir)?;
        let path = dir.join(if name.is_empty() { "unnamed".into() } else { name });
        fs::write(&path, bytes)?;
        resources.push((path, bytes.len()));
    }

    Ok(XexDump {
        pe_name,
        image_path,
        sections_path,
        imports_path,
        resources,
        load_address,
        entry_point,
        image_size: image.data.len(),
        blocks: image.blocks,
        frames: image.frames,
        pages,
        page_size: x.security.page_size(),
        pe,
        import_counts,
    })
}
