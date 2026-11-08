use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ssx_content::Kind;

use super::vaults::{archives, files_with_ext, rel};
use super::{Catalogue, DataDirs, path_segments, segment};

pub(super) fn members(big: &Path, root: &str, ext: &str) -> Result<BTreeMap<String, Vec<PathBuf>>> {
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for a in archives(big)? {
        let base = a.join(root);
        for f in files_with_ext(&base, ext)? {
            let r = rel(&base, &f);
            let stem = r.strip_suffix(&format!(".{ext}")).unwrap_or(&r).to_string();
            out.entry(stem).or_default().push(f);
        }
    }
    Ok(out)
}

fn file_ids(c: &mut Catalogue, big: &Path, kind: Kind, roots: &[&str], ext: &str) -> Result<()> {
    let mut several = 0usize;
    for root in roots {
        for (stem, holders) in members(big, root, ext)? {
            several += usize::from(holders.len() > 1);
            match path_segments(&stem) {
                Some(id) => c.add(kind, &id, rel(big, &holders[0])),
                None => c.note(kind, format!("{root}/{stem}.{ext}: outside the id alphabet (left out)")),
            }
        }
    }
    if several > 0 {
        c.note(kind, format!("{several} files are in several archives (the first archive in name order is listed)"));
    }
    Ok(())
}

pub fn add(c: &mut Catalogue, dirs: &DataDirs<'_>, want: &dyn Fn(Kind) -> bool) -> Result<()> {
    let big = dirs.big;
    if want(Kind::Script) {
        file_ids(c, big, Kind::Script, &["data/common", "data/xenon/gameexplorer"], "lua")?;
    }
    if want(Kind::Ui) {
        file_ids(c, big, Kind::Ui, &["data/common/fe"], "gfx")?;
    }
    if want(Kind::Particles) {
        file_ids(c, big, Kind::Particles, &["data/xenon/gameexplorer"], "lnx")?;
        let vault = "bootcache/data/xenon/attribdb/ge_attribdb.vlt";
        for (name, _) in super::particles::SPRAY_TABLES {
            let path = format!("{}{name}", ssx_content::particles::SPRAY_PREFIX);
            c.add(Kind::Particles, &path, format!("{vault}#ge_snowspray/{name}"));
        }
        c.note(Kind::Particles, "the .lnx systems by path, and the two snow-spray tables as snowspray/<name>");
    }
    if want(Kind::Movie) {
        movies(c, dirs)?;
    }
    if want(Kind::Texture) {
        texture_files(c, big)?;
    }
    if want(Kind::Font) {
        fonts(c, big)?;
    }
    if want(Kind::String) {
        strings(c, big)?;
    }
    if want(Kind::Sound) {
        sounds(c, big)?;
    }
    if want(Kind::Music) {
        music(c, big)?;
    }
    if want(Kind::Anim) {
        anims(c, big)?;
    }
    if want(Kind::Tuning) || want(Kind::Scoring) {
        super::ant::add(c, big, want)?;
    }
    Ok(())
}

fn texture_files(c: &mut Catalogue, big: &Path) -> Result<()> {
    let disc = super::textures::DiscTextures::index(big)?;
    let (mut fe, mut other) = (0usize, 0usize);
    for (id, path) in disc.files() {
        if id.path().starts_with("fe/") {
            fe += 1;
        } else {
            other += 1;
        }
        c.add(Kind::Texture, id.path(), rel(big, path));
    }
    c.note(Kind::Texture, format!("from the files: {fe} front-end images (`fe/…`), {other} game-explorer textures"));
    for (id, paths) in &disc.collisions {
        let shown: Vec<String> = paths.iter().map(|p| rel(big, p)).collect();
        c.note(Kind::Texture, format!("{id}: several files take it (left out): {}", shown.join(", ")));
    }
    Ok(())
}

fn movies(c: &mut Catalogue, dirs: &DataDirs<'_>) -> Result<()> {
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let disc_dir = dirs.disc.join(crate::setup::MOVIE_DIR);
    for f in files_with_ext(&disc_dir, "vp6")? {
        let stem = f.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        found.entry(stem).or_default().push(format!("disc/{}", rel(dirs.disc, &f)));
    }
    for (stem, holders) in members(dirs.big, crate::setup::MOVIE_DIR, "vp6")? {
        found.entry(stem).or_default().extend(holders.iter().map(|h| rel(dirs.big, h)));
    }
    let mut both = 0usize;
    for (stem, places) in found {
        both += usize::from(places.len() > 1);
        match segment(&stem) {
            Some(id) => c.add(Kind::Movie, &id, places[0].clone()),
            None => c.note(Kind::Movie, format!("{stem}.vp6: outside the id alphabet (left out)")),
        }
    }
    if both > 0 {
        c.note(
            Kind::Movie,
            format!("{both} movies are both loose on the disc and in an archive (the disc copy is listed)"),
        );
    }
    Ok(())
}

fn fonts(c: &mut Catalogue, big: &Path) -> Result<()> {
    for (stem, holders) in members(big, "data/common/fe/gfxfontlib", "gfx")? {
        let Some(region) = stem.strip_suffix("/gfxfontlib") else { continue };
        let path = &holders[0];
        let movie =
            ssx_formats::gfx::Movie::parse(&std::fs::read(path)?).with_context(|| format!("{}", path.display()))?;
        for (_, t) in movie.all_tags() {
            if let ssx_formats::gfx::Tag::Font(f) = &t.tag
                && !f.glyphs.is_empty()
            {
                match (segment(region), segment(&f.name)) {
                    (Some(r), Some(n)) => {
                        let source = format!("{}#{}", rel(big, path), f.name);
                        c.add_record(Kind::Font, &format!("{r}/{n}"), format!("{source} (font {})", f.id), source);
                    }
                    _ => c.note(Kind::Font, format!("{region}: font {:?} outside the id alphabet (left out)", f.name)),
                }
            }
        }
    }
    let ttf: Vec<String> = members(big, "data", "ttf")?.values().map(|h| rel(big, &h[0])).collect();
    c.note(
        Kind::Font,
        format!(
            "{} TrueType files are not in a font library and are left out: {} (dinengschriftstd.ttf is the 3-D text \
             font the executable loads by name, 0x82FB6FC4; no reader of the other two is known)",
            ttf.len(),
            ttf.join(", ")
        ),
    );
    Ok(())
}

fn strings(c: &mut Catalogue, big: &Path) -> Result<()> {
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tables = members(big, "data/xenon/loc", "db")?;
    for (lang, holders) in &tables {
        let l = ssx_formats::t3db::LocStrings::parse(&std::fs::read(&holders[0])?)
            .with_context(|| format!("{}", holders[0].display()))?;
        for (key, _) in l.by_hash.values() {
            keys.entry(key.clone()).or_default().push(lang.clone());
        }
    }
    let dir = tables.values().next().and_then(|h| h[0].parent()).map(|p| rel(big, p)).unwrap_or_default();
    for (key, langs) in keys {
        let source = if langs.len() == tables.len() {
            format!("{dir}/*.db")
        } else {
            format!("{dir}/{{{}}}.db", langs.join(","))
        };
        match segment(&key) {
            Some(id) => c.add_record(Kind::String, &id, key.clone(), source),
            None => c.note(Kind::String, format!("key {key:?} has characters outside the id alphabet (left out)")),
        }
    }
    Ok(())
}

fn sounds(c: &mut Catalogue, big: &Path) -> Result<()> {
    use ssx_formats::csi::{CsiFile, Kind as CsiKind};
    let mut globals = 0usize;
    for (stem, holders) in members(big, "data/xenon/audio", "csi")? {
        let path = &holders[0];
        let f = CsiFile::parse(&std::fs::read(path)?).with_context(|| format!("{}", path.display()))?;
        for e in &f.entries {
            if e.kind == CsiKind::Global {
                globals += 1;
                continue;
            }
            match (path_segments(&stem), segment(&e.name)) {
                (Some(m), Some(n)) => {
                    let source = format!("{}#{}", rel(big, path), e.name);
                    c.add_record(Kind::Sound, &format!("{m}/{n}"), format!("{source} ({:08x})", e.id), source);
                }
                _ => c.note(Kind::Sound, format!("{stem}: {:?} outside the id alphabet (left out)", e.name)),
            }
        }
    }
    c.note(Kind::Sound, format!("{globals} CSIS globals are shared integers, not sounds (left out)"));
    Ok(())
}

fn music(c: &mut Catalogue, big: &Path) -> Result<()> {
    let info = members(big, "data/xenon/audio/music", "bin")?;
    if let Some(holders) = info.get("music_musicboxsonginfo") {
        let path = &holders[0];
        for s in crate::musicbox::parse_song_info(&std::fs::read(path)?)? {
            c.add(Kind::Music, &format!("licensed/{}", s.id), format!("{}#MBSI {}", rel(big, path), s.id));
        }
    } else {
        c.note(Kind::Music, "music_musicboxsonginfo.bin not found: no licensed songs listed");
    }
    for (stem, holders) in members(big, "data/xenon/audio/pathfinder", "mpf")? {
        let path = &holders[0];
        let Some(project) = stem.split('/').next_back().and_then(segment) else { continue };
        let mpf =
            ssx_formats::mpf::MpfFile::parse(&std::fs::read(path)?).with_context(|| format!("{}", path.display()))?;
        for i in 0..mpf.tracks.len() {
            c.add(Kind::Music, &format!("pathfinder/{project}/{i}"), format!("{}#track {i}", rel(big, path)));
        }
    }
    Ok(())
}

fn anims(c: &mut Catalogue, big: &Path) -> Result<()> {
    for (stem, holders) in members(big, "data/common/anim", "cba")? {
        let path = &holders[0];
        let s =
            ssx_formats::ant::Stream::parse(&std::fs::read(path)?).with_context(|| format!("{}", path.display()))?;
        let mut left_out = Vec::new();
        for n in super::anims::index::named_clips(&stem, &s, &mut left_out) {
            let source = format!("{}#{} {}", rel(big, path), n.class, n.name);
            c.add_record(Kind::Anim, &n.path, format!("{source} {{{}}}", n.guid), source);
        }
        for l in left_out {
            c.note(Kind::Anim, format!("{l} (left out)"));
        }
    }
    Ok(())
}
