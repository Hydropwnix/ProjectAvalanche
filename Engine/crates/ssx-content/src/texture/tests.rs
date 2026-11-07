use super::*;

mod files;
use files::*;

fn none() -> TextureSettings {
    TextureSettings::default()
}

fn decode(name: &str, bytes: &[u8]) -> Result<Texture, TextureError> {
    decode_file(name, bytes, &none(), &Limits::default())
}

fn err(name: &str, bytes: &[u8]) -> String {
    let e = decode(name, bytes).unwrap_err();
    assert_eq!(e.file, name);
    e.message
}

#[test]
fn png_colour_types_round_trip_and_get_mips() {
    let rgba: Vec<u8> = (0..8 * 4).map(|i| (i * 7) as u8).collect();
    let t = decode("a.png", &png_file(4, 2, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &rgba)).unwrap();
    assert_eq!((t.shape, t.width(), t.height(), t.levels.len()), (Shape::Plane, 4, 2, 3));
    assert_eq!(t.levels[0].images, [rgba]);
    assert!(t.blocks.is_none() && t.kind.is_none() && t.source.is_some());
    let grey =
        decode("g.png", &png_file(2, 1, ::png::ColorType::Grayscale, ::png::BitDepth::Eight, &[9, 200])).unwrap();
    assert_eq!(grey.levels[0].images[0], [9, 9, 9, 255, 200, 200, 200, 255]);
    let ga = png_file(1, 1, ::png::ColorType::GrayscaleAlpha, ::png::BitDepth::Eight, &[7, 70]);
    assert_eq!(decode("ga.png", &ga).unwrap().levels[0].images[0], [7, 7, 7, 70]);
    let rgb16 = png_file(1, 1, ::png::ColorType::Rgb, ::png::BitDepth::Sixteen, &[0x12, 0x34, 0xAB, 0xCD, 0xFF, 0x00]);
    assert_eq!(decode("w.png", &rgb16).unwrap().levels[0].images[0], [0x12, 0xAB, 0xFF, 255]);
    let indexed = palette_png(&[[1, 2, 3], [40, 50, 60]], &[1, 0]);
    assert_eq!(decode("p.png", &indexed).unwrap().levels[0].images[0], [40, 50, 60, 255, 1, 2, 3, 255]);
}

#[test]
fn jpeg_baseline_progressive_and_grey() {
    for (name, bytes) in [("b.jpg", hex(JPEG_BASE)), ("p.jpeg", hex(JPEG_PROG))] {
        let t = decode(name, &bytes).unwrap();
        assert_eq!((t.width(), t.height(), t.levels.len()), (16, 8, 5));
        let img = &t.levels[0].images[0];
        assert!(img[0] > 200 && img[2] < 60, "{:?}", &img[..4]);
        let bottom = 7 * 16 * 4;
        assert!(img[bottom] < 60 && img[bottom + 2] > 200, "{:?}", &img[bottom..bottom + 4]);
    }
    let g = decode("g.jpg", &hex(JPEG_GREY)).unwrap();
    assert!(g.levels[0].images[0].as_chunks::<4>().0.iter().all(|t| t[0] == t[1] && t[1] == t[2] && t[3] == 255));
}

#[test]
fn dds_blocks_bytes_and_cubes() {
    let t = decode("a.dds", &dds_legacy_fourcc(4, 4, 3, b"DXT1", &[RED_BLUE_BC1; 3].concat())).unwrap();
    let chain = t.blocks.as_ref().unwrap();
    assert_eq!((chain.format, chain.levels.len(), t.levels.len()), (BlockFormat::Bc1, 3, 3));
    assert_eq!(&t.levels[0].images[0][..8], &[255, 0, 0, 255, 0, 0, 255, 255]);
    let bgra = dds_legacy_rgb(2, 1, 32, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000], &[1, 2, 3, 4, 5, 6, 7, 8]);
    let t = decode("b.dds", &bgra).unwrap();
    assert_eq!((t.levels.len(), &t.levels[0].images[0][..]), (2, &[3, 2, 1, 4, 7, 6, 5, 8][..]));
    let t = decode("c.dds", &dds_dx10(4, 4, 1, 98, 0, 1, &[0x40; 16])).unwrap();
    assert!(t.blocks.is_none() && t.levels.len() == 3);
    let faces: Vec<u8> = (0..6).flat_map(|f| [f * 10, 0, 0, 255]).collect();
    let t = decode("d.dds", &dds_dx10(1, 1, 1, 28, 0x4, 1, &faces)).unwrap();
    assert_eq!((t.shape, t.levels[0].images.len(), t.levels[0].images[5][0]), (Shape::Cube, 6, 50));
}

#[test]
fn ktx2_plain_zstd_zlib_and_blocks() {
    let rgba: Vec<u8> = (0..16 * 4).map(|i| i as u8).collect();
    let plain = decode("a.ktx2", &ktx2_file(37, 4, 4, 1, 0, std::slice::from_ref(&rgba))).unwrap();
    assert_eq!((plain.levels.len(), &plain.levels[0].images[0]), (3, &rgba));
    let zstd = ruzstd::encoding::compress_to_vec(&rgba[..], ruzstd::encoding::CompressionLevel::Fastest);
    let z = decode("z.ktx2", &ktx2_file_with(37, 4, 4, 1, 2, &[(zstd, rgba.len() as u64)])).unwrap();
    assert_eq!(z.levels[0], plain.levels[0]);
    let zlib = zlib(&rgba);
    let z = decode("l.ktx2", &ktx2_file_with(37, 4, 4, 1, 3, &[(zlib, rgba.len() as u64)])).unwrap();
    assert_eq!(z.levels[0], plain.levels[0]);
    let bc = decode("b.ktx2", &ktx2_file(133, 4, 4, 1, 0, &[RED_BLUE_BC1.to_vec()])).unwrap();
    assert_eq!(bc.levels.len(), 3);
    let punch = [0x1f, 0x00, 0x00, 0xf8, 0xff, 0xff, 0xff, 0xff];
    let mut levels = vec![punch.to_vec()];
    levels.extend([RED_BLUE_BC1.to_vec(), RED_BLUE_BC1.to_vec()]);
    let rgb = decode("r.ktx2", &ktx2_file(131, 4, 4, 3, 0, &levels)).unwrap();
    assert_eq!(&rgb.levels[0].images[0][..4], &[0, 0, 0, 255]);
    assert!(rgb.blocks.is_none());
}

#[test]
fn normal_kind_keeps_two_channels() {
    let s = TextureSettings { kind: Some(TextureKind::Normal) };
    let png = png_file(1, 1, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[10, 20, 30, 40]);
    let t = decode_file("n.png", &png, &s, &Limits::default()).unwrap();
    assert_eq!((t.kind, &t.levels[0].images[0][..]), (Some(TextureKind::Normal), &[10, 20, 0, 255][..]));
}

#[test]
fn wrong_names_and_magic() {
    let png = png_file(1, 1, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[1, 2, 3, 4]);
    assert!(err("a.txt", &png).contains("not a texture file"));
    assert!(err("a.jpg", &png).contains("named .jpg, but its content is PNG"));
    assert!(err("a.dds", &hex(JPEG_BASE)).contains("content is JPEG"));
    assert!(err("a.ktx2", b"\0\0\0\0garbage").contains("of no texture format"));
    assert!(err("a.png", b"").contains("of no texture format"));
}

#[test]
fn truncated_files_fail() {
    let png = png_file(8, 8, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[7; 256]);
    assert!(err("a.png", &png[..png.len() - 20]).starts_with("PNG:"));
    let jpg = hex(JPEG_BASE);
    assert!(err("a.jpg", &jpg[..jpg.len() / 2]).starts_with("JPEG:"));
    let dds = dds_legacy_fourcc(4, 4, 3, b"DXT1", &[RED_BLUE_BC1; 3].concat());
    assert!(err("a.dds", &dds[..dds.len() - 1]).contains("bytes of texels"));
    assert!(err("a.dds", &dds[..100]).contains("shorter than its header"));
    let mut longer = dds.clone();
    longer.push(0);
    assert!(err("a.dds", &longer).contains("bytes of texels"));
    let k = ktx2_file(37, 4, 4, 1, 0, &[vec![1; 64]]);
    assert!(err("a.ktx2", &k[..k.len() - 1]).contains("past the end"));
    assert!(err("a.ktx2", &k[..70]).contains("shorter than its header"));
}

#[test]
fn oversized_and_bombs_are_refused_before_decoding() {
    assert!(err("a.png", &png_header_only(100_000, 100_000)).contains("larger than 4096×4096"));
    let big = hex(JPEG_BASE.replace("ffc000110800080010", "ffc000110814001400").as_str());
    assert!(err("a.jpg", &big).starts_with("JPEG:"));
    assert!(err("a.dds", &dds_legacy_fourcc(100_000, 4, 1, b"DXT1", &[])).contains("larger than"));
    assert!(err("a.ktx2", &ktx2_file(37, 100_000, 100_000, 1, 0, &[])).contains("larger than"));
    let small = Limits { max_decoded_bytes: 1000, ..Limits::default() };
    let png = png_file(32, 32, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[0; 4096]);
    let e = decode_file("a.png", &png, &none(), &small).unwrap_err();
    assert!(e.message.contains("over the cap of 1000"), "{e}");
    let tiny = Limits { max_file_bytes: 10, ..Limits::default() };
    assert!(decode_file("a.png", &png, &none(), &tiny).unwrap_err().message.contains("file cap"));
    let bomb = ruzstd::encoding::compress_to_vec(&vec![0u8; 1 << 20][..], ruzstd::encoding::CompressionLevel::Fastest);
    assert!(bomb.len() < 4096);
    let k = ktx2_file_with(37, 4, 4, 1, 2, &[(bomb, 64)]);
    assert!(err("a.ktx2", &k).contains("holds 65 bytes"));
    let k = ktx2_file_with(37, 4, 4, 1, 3, &[(zlib(&vec![0u8; 1 << 20]), 64)]);
    assert!(err("a.ktx2", &k).contains("holds 65 bytes"));
    let k = ktx2_file_with(37, 4, 4, 1, 2, &[(vec![0; 8], 1 << 30)]);
    assert!(err("a.ktx2", &k).contains("states 1073741824 bytes"));
}

#[test]
fn unsupported_layouts_name_what_they_are() {
    assert!(err("a.dds", &dds_dx10(4, 4, 1, 95, 0, 1, &[0; 16])).contains("BC6H"));
    assert!(err("a.dds", &dds_dx10(4, 4, 1, 77, 0, 2, &[0; 32])).contains("an array of 2"));
    assert!(err("a.dds", &dds_dx10(4, 4, 1, 76, 0, 1, &[0; 16])).contains("typeless"));
    assert!(err("a.dds", &dds_legacy_fourcc(4, 4, 1, b"XYZW", &[0; 8])).contains("FourCC"));
    assert!(err("a.dds", &dds_legacy_fourcc(4, 4, 9, b"DXT1", &[0; 72])).contains("9 mip levels"));
    let mut volume = dds_legacy_fourcc(4, 4, 1, b"DXT1", &[0; 8]);
    volume[112..116].copy_from_slice(&0x20_0000u32.to_le_bytes());
    assert!(err("a.dds", &volume).contains("volume"));
    let mut partial = dds_legacy_fourcc(4, 4, 1, b"DXT1", &[0; 48]);
    partial[112..116].copy_from_slice(&(0x200u32 | 0x400).to_le_bytes());
    assert!(err("a.dds", &partial).contains("six faces"));
    assert!(err("a.ktx2", &ktx2_file(0, 4, 4, 1, 0, &[vec![0; 16]])).contains("UNDEFINED"));
    assert!(err("a.ktx2", &ktx2_file(147, 4, 4, 1, 0, &[vec![0; 8]])).contains("mobile block format"));
    assert!(err("a.ktx2", &ktx2_file_with(37, 4, 4, 1, 1, &[(vec![0; 64], 64)])).contains("BasisLZ"));
    let mut faces3 = ktx2_file(37, 4, 4, 1, 0, &[vec![0; 64]]);
    faces3[36..40].copy_from_slice(&3u32.to_le_bytes());
    assert!(err("a.ktx2", &faces3).contains("3 faces"));
    let mut dfd = ktx2_file(37, 4, 4, 1, 0, &[vec![0; 64]]);
    let at = u32::from_le_bytes(dfd[48..52].try_into().unwrap()) as usize;
    dfd[at] ^= 1;
    assert!(err("a.ktx2", &dfd).contains("data format descriptor"));
}

#[test]
fn the_fuzzers_crash_inputs_do_not_panic() {
    for (name, input) in [("a.png", FUZZ_PNG), ("a.jpg", FUZZ_JPEG), ("a.dds", FUZZ_DDS)] {
        match decode(name, &hex(input)) {
            Ok(t) => {
                let (w, h) = (t.width(), t.height());
                assert_eq!(t.levels.len() as u32, chain_len(w, h), "{name}: {w}×{h}");
            }
            Err(e) => assert!(e.message.contains("CRC"), "{name}: {e}"),
        }
    }
}

#[test]
fn mutations_never_panic() {
    let rgba: Vec<u8> = (0..16 * 4).map(|i| i as u8).collect();
    let zstd = ruzstd::encoding::compress_to_vec(&rgba[..], ruzstd::encoding::CompressionLevel::Fastest);
    let seeds: Vec<(&str, Vec<u8>)> = vec![
        ("a.png", png_file(4, 4, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &rgba)),
        ("a.jpg", hex(JPEG_BASE)),
        ("a.jpg", hex(JPEG_PROG)),
        ("a.dds", dds_legacy_fourcc(4, 4, 3, b"DXT1", &[RED_BLUE_BC1; 3].concat())),
        ("a.dds", dds_dx10(4, 4, 1, 98, 0, 1, &[0x40; 16])),
        ("a.ktx2", ktx2_file_with(37, 4, 4, 1, 2, &[(zstd, rgba.len() as u64)])),
        ("a.ktx2", ktx2_file(133, 4, 4, 1, 0, &[RED_BLUE_BC1.to_vec()])),
    ];
    let mut tried = 0;
    for (name, seed) in &seeds {
        for at in 0..seed.len() {
            for v in [0u8, 0xFF, seed[at] ^ 0x01, seed[at] ^ 0x80] {
                let mut m = seed.clone();
                m[at] = v;
                let _ = decode(name, &m);
                tried += 1;
            }
            let _ = decode(name, &seed[..at]);
        }
    }
    assert!(tried > 5000, "{tried}");
}

#[test]
fn headers_are_measured_before_decoding() {
    let limits = Limits::default();
    let probe = |name: &str, bytes: &[u8]| {
        let mut read = |n: u64| Ok(bytes[..(n as usize).min(bytes.len())].to_vec());
        probe_file(name, bytes.len() as u64, &mut read, &limits)
    };
    let p = probe("a.png", &crate::test_dir::png_rgba(2, 3, [1, 2, 3, 4])).unwrap();
    assert_eq!((p.width, p.height, p.faces), (2, 3, 1));
    let big = probe("b.png", &png_header_only(4096, 4096)).unwrap();
    assert_eq!(big.decoded_bytes(), 89_478_484);
    assert!(probe("c.png", &png_header_only(8192, 16)).unwrap_err().to_string().contains("larger than 4096"));
    let mut cube = ktx2_file(37, 64, 64, 1, 0, &[vec![0; 64 * 64 * 4 * 6]]);
    cube[12 + 4 * 6..12 + 4 * 7].copy_from_slice(&6u32.to_le_bytes());
    assert_eq!(probe("d.ktx2", &cube).unwrap().faces, 6);
    let dds = dds_dx10(32, 16, 1, 28, 0x4, 1, &[]);
    let d = probe("e.dds", &dds).unwrap();
    assert_eq!((d.width, d.height, d.faces), (32, 16, 6));
    let jpeg = hex(JPEG_BASE);
    let behind = |segments: usize| {
        let mut out = jpeg[..2].to_vec();
        for _ in 0..segments {
            out.extend([0xFF, 0xFE, 0xFF, 0xFF]);
            out.extend(std::iter::repeat_n(b'x', 0xFFFD));
        }
        out.extend(&jpeg[2..]);
        out
    };
    let j = probe("f.jpg", &behind(2)).unwrap();
    assert_eq!((j.width, j.height), (16, 8));
    assert!(probe("g.jpg", &behind(17)).is_err(), "a frame header past the first 1 MiB");
    let never = |_: u64| -> Result<Vec<u8>, String> { panic!("read") };
    let over = Limits { max_file_bytes: 100, ..Limits::default() };
    assert!(probe_file("h.png", 101, &mut { never }, &over).unwrap_err().to_string().contains("over the texture file"));
}
