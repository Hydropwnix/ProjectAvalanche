use std::collections::BTreeMap;

use super::*;

struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        if b.len() as u64 > max { Err(format!("{path}: over {max} bytes")) } else { Ok(b.clone()) }
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        self.0.get(path).map(|b| b.len() as u64).ok_or_else(|| format!("{path}: not in the package"))
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        let end = (offset + len) as usize;
        b.get(offset as usize..end).map(<[u8]>::to_vec).ok_or_else(|| format!("{path}: past its end"))
    }
}

pub(crate) fn hdr_file(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Vec<u8> {
    use image::codecs::hdr::HdrEncoder;
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.push(image::Rgb(f(x, y)));
        }
    }
    let mut out = Vec::new();
    HdrEncoder::new(&mut out).encode(&px, w as usize, h as usize).expect("an HDR image");
    out
}

pub(crate) fn exr_file(w: usize, h: usize, f: impl Fn(usize, usize) -> [f32; 3] + Sync) -> Vec<u8> {
    use ::exr::prelude::*;
    let channels = SpecificChannels::rgb(|p: Vec2<usize>| {
        let [r, g, b] = f(p.x(), p.y());
        (r, g, b)
    });
    let image = Image::from_channels((w, h), channels);
    let mut out = std::io::Cursor::new(Vec::new());
    image.write().non_parallel().to_buffered(&mut out).expect("an EXR image");
    out.into_inner()
}

#[test]
fn a_sky_toml_reads_and_refuses_with_lines() {
    let ok = br#"type = "cubemap"
equirect = "skies/aurora.hdr"
tint = [1.0, 0.9, 0.8]
exposure = 2.0
rotation = 90
clouds = true
reflection = true
face_size = 256
"#;
    let s = parse_toml(ok, "skies/aurora.toml").unwrap();
    assert!(
        matches!(&s.kind, SkyKind::Cubemap { source: CubeSource::Equirect(f), face_size: Some(256), .. } if f == "skies/aurora.hdr")
    );
    assert_eq!(
        (s.exposure, s.rotation, s.sun, s.clouds, s.midground, s.reflection),
        (2.0, 90.0, true, true, true, true)
    );
    let dome =
        parse_toml(b"type = \"dome\"\nmodel = \"p:model/dome\"\nmaterial = \"p:material/dome\"\nsun = false\n", "d")
            .unwrap();
    assert!(matches!(dome.kind, SkyKind::Dome { .. }) && !dome.sun);
    let bad = br#"type = "cubemap"
cube = "skies/a.png"
faces = ["a.png"]
exposure = -1
rotation = 400
tint = [1, 2]
face_size = 300
"#;
    let p = parse_toml(bad, "s").unwrap_err();
    let lines: Vec<usize> = p.iter().map(|(l, _, _)| *l).collect();
    assert_eq!(lines, [1, 2, 3, 4, 5, 6, 7], "{p:?}");
    assert!(parse_toml(b"type = \"scattering\"\n", "s").unwrap_err()[0].2.contains("game's own sky"));
    assert!(parse_toml(b"type = \"dome\"\nmodel = \"p:texture/x\"\n", "s").is_err());
    assert!(parse_toml(b"type = \"cubemap\"\ncube = \"../x.ktx2\"\n", "s").is_err());
    assert!(
        parse_toml(b"type = \"cubemap\"\ncube = \"x.ktx2\"\nwhat = 1\n", "s").unwrap_err()[0]
            .2
            .contains("unknown field")
    );
}

#[test]
fn an_equirectangular_image_maps_onto_the_cube_by_direction() {
    let (w, h) = (64u32, 32u32);
    let img = FloatImage {
        width: w,
        height: h,
        rgb: (0..h).flat_map(|y| (0..w).map(move |x| [x as f32 / w as f32, y as f32 / h as f32, 1.0])).collect(),
    };
    let (u, v) = equirect_texel([0.0, 0.0, 1.0], w, h);
    assert!((u - 31.5).abs() < 1e-3 && (v - 15.5).abs() < 1e-3, "{u} {v}");
    let (u, _) = equirect_texel([1.0, 0.0, 0.0], w, h);
    assert!((u - 47.5).abs() < 1e-3, "{u}");
    let cube = equirect_to_cube(&img, 8);
    assert_eq!(cube.levels.len(), 4, "8, 4, 2, 1");
    assert_eq!(cube.bytes(), cube_bytes(8));
    let z = cube.texel(4, 4, 4);
    assert!((z[0] - 0.5).abs() < 0.08, "{z:?}");
    let up = cube.texel(2, 4, 4);
    assert!(up[1] < 0.15, "{up:?}");
    for f in 0..6 {
        let d = face_direction(f, 1, 6, 8);
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-5);
    }
}

#[test]
fn hdr_and_exr_skies_decode_and_hostile_files_are_refused() {
    let hdr = hdr_file(32, 16, |x, y| [x as f32, y as f32, 0.5]);
    let exr = exr_file(32, 16, |x, y| [x as f32, y as f32, 0.25]);
    let files = Files([("a.hdr".to_string(), hdr.clone()), ("b.exr".to_string(), exr.clone())].into());
    let limits = Limits::default();
    for (file, b) in [("a.hdr", 0.5), ("b.exr", 0.25)] {
        let def = parse_toml(format!("type = \"cubemap\"\nequirect = \"{file}\"\n").as_bytes(), file).unwrap();
        let cost = probe(&def, &files, &limits).unwrap();
        assert_eq!(cost.face, 8, "{file}: a quarter of the width");
        let cube = decode(&def, &files, &limits).unwrap().unwrap();
        assert_eq!(cube.size(), 8);
        assert!((cube.texel(0, 0, 0)[2] - b).abs() < 0.01, "{file}");
    }
    let small = Limits { max_source_bytes: 1000, ..Limits::default() };
    let def = parse_toml(b"type = \"cubemap\"\nequirect = \"a.hdr\"\n", "s").unwrap();
    assert!(probe(&def, &files, &small).unwrap_err().message.contains("over the cap"));
    let mut huge = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 60000 +X 60000\n".to_vec();
    huge.extend([2, 2, 0, 0]);
    let files = Files([("h.hdr".to_string(), huge)].into());
    let def = parse_toml(b"type = \"cubemap\"\nequirect = \"h.hdr\"\n", "s").unwrap();
    assert!(probe(&def, &files, &limits).unwrap_err().message.contains("1 to 8192 a side"));
    let cut = Files(
        [("c.hdr".to_string(), hdr[..hdr.len() / 2].to_vec()), ("d.exr".to_string(), exr[..exr.len() / 2].to_vec())]
            .into(),
    );
    for f in ["c.hdr", "d.exr"] {
        let def = parse_toml(format!("type = \"cubemap\"\nequirect = \"{f}\"\n").as_bytes(), f).unwrap();
        assert!(decode(&def, &cut, &limits).is_err(), "{f}");
    }
    let junk = Files([("j.hdr".to_string(), b"\xFF\xD8\xFFjunk".to_vec()), ("k.exr".to_string(), vec![0; 40])].into());
    for f in ["j.hdr", "k.exr"] {
        let def = parse_toml(format!("type = \"cubemap\"\nequirect = \"{f}\"\n").as_bytes(), f).unwrap();
        assert!(probe(&def, &junk, &limits).is_err(), "{f}");
    }
}

#[test]
fn six_faces_make_a_cube_and_mismatched_ones_are_refused() {
    let png = |side: u32, v: u8| {
        let mut out = Vec::new();
        let mut e = ::png::Encoder::new(&mut out, side, side);
        e.set_color(::png::ColorType::Rgba);
        e.set_depth(::png::BitDepth::Eight);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&vec![v; (side * side * 4) as usize]).unwrap();
        drop(w);
        out
    };
    let names = ["px.png", "nx.png", "py.png", "ny.png", "pz.png", "nz.png"];
    let mut files: BTreeMap<String, Vec<u8>> = names.iter().map(|n| (n.to_string(), png(4, 255))).collect();
    let toml = b"type = \"cubemap\"\nfaces = [\"px.png\", \"nx.png\", \"py.png\", \"ny.png\", \"pz.png\", \"nz.png\"]\ncolor = \"linear\"\n";
    let def = parse_toml(toml, "s").unwrap();
    let cube = decode(&def, &Files(files.clone()), &Limits::default()).unwrap().unwrap();
    assert_eq!((cube.size(), cube.levels.len()), (4, 3));
    assert_eq!(cube.texel(5, 3, 3), [1.0; 4]);
    files.insert("nz.png".into(), png(8, 0));
    assert!(probe(&def, &Files(files), &Limits::default()).unwrap_err().message.contains("one size"));
}
