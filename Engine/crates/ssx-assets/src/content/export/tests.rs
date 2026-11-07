use super::*;
use ssx_content::texture::{BlockLevel, Level, Limits, TextureSettings, decode_file};

fn temp(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ssx-export-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn blocked(format: BlockFormat) -> Texture {
    let shapes = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
    let levels: Vec<BlockLevel> = shapes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            let n = format.level_bytes(w, h) as usize;
            BlockLevel { width: w, height: h, data: (0..n).map(|k| (k * 37 + i * 11) as u8).collect() }
        })
        .collect();
    let rgba = levels.iter().map(|l| Level {
        width: l.width,
        height: l.height,
        images: vec![ssx_content::texture::bc::decode_level(format, &l.data, l.width, l.height).unwrap()],
    });
    Texture {
        shape: Shape::Plane,
        levels: rgba.collect(),
        blocks: Some(BlockChain { format, levels }),
        kind: None,
        source: None,
    }
}

#[test]
fn block_exports_round_trip_exactly() {
    for format in [BlockFormat::Bc1, BlockFormat::Bc2, BlockFormat::Bc3, BlockFormat::Bc5] {
        let t = blocked(format);
        let files = export_files(&t, "tex", true).unwrap();
        let (name, bytes) = &files[0];
        assert_eq!(name, "tex.dds");
        let settings = if format == BlockFormat::Bc5 {
            assert_eq!(files[1].0, "textures.toml");
            ssx_content::texture::FolderSettings::parse(&files[1].1, "t").unwrap().of("tex.dds")
        } else {
            TextureSettings::default()
        };
        let back = decode_file(name, bytes, &settings, &Limits::default()).unwrap();
        assert_eq!(back.blocks, t.blocks, "{format:?}");
        assert_eq!(back.levels, t.levels, "{format:?}");
    }
}

#[test]
fn colour_exports_are_png_and_cubes_dds() {
    let mut t = blocked(BlockFormat::Bc1);
    let files = export_files(&t, "tex", false).unwrap();
    assert_eq!(files.len(), 1);
    let back = decode_file(&files[0].0, &files[0].1, &TextureSettings::default(), &Limits::default()).unwrap();
    assert_eq!((files[0].0.as_str(), &back.levels[0]), ("tex.png", &t.levels[0]));
    t.shape = Shape::Cube;
    t.blocks = None;
    for l in &mut t.levels {
        l.images = (0..6u8).map(|f| l.images[0].iter().map(|v| v ^ f).collect()).collect();
    }
    let files = export_files(&t, "cube", false).unwrap();
    let back = decode_file(&files[0].0, &files[0].1, &TextureSettings::default(), &Limits::default()).unwrap();
    assert_eq!((back.shape, &back.levels), (Shape::Cube, &t.levels));
    t.shape = Shape::Volume;
    assert!(export_files(&t, "v", false).is_err());
}

#[test]
fn the_export_index_records_and_parses() {
    let app = temp("index");
    let row = |n: u8| ExportRow {
        hash: ContentHash::of(&[n]),
        size: n as u64,
        id: "ssx:texture/a".into(),
        file: format!("/x/{n}.png"),
    };
    record_exports(&app, &[row(1)]).unwrap();
    record_exports(&app, &[row(2), row(3)]).unwrap();
    let rows = parse_exports(&std::fs::read_to_string(app.join(EXPORTS_FILE)).unwrap()).unwrap();
    assert_eq!(rows, [row(1), row(2), row(3)]);
    let bad = format!("{HEADER}{}\t01\tx\ty\n", ContentHash::of(b"a"));
    assert!(parse_exports(&bad).unwrap_err().contains(":2:"));
    assert!(parse_exports("nope\n").is_err());
    let tab = ExportRow { file: "a\tb".into(), ..row(4) };
    assert!(record_exports(&app, &[tab]).is_err());
    let _ = std::fs::remove_dir_all(&app);
}
