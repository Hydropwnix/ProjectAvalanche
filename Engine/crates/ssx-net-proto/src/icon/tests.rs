use std::io::Write;

use super::*;

fn encoded(
    w: u32,
    h: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    f: impl Fn(&mut png::Encoder<&mut Vec<u8>>),
) -> Vec<u8> {
    let channels = match color {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    };
    let bytes_per = if depth == png::BitDepth::Sixteen { 2 } else { 1 };
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(depth);
        f(&mut e);
        let mut wr = e.write_header().expect("header");
        let data: Vec<u8> = (0..(w * h) as usize * channels * bytes_per).map(|i| (i % 7) as u8).collect();
        wr.write_image_data(&data).expect("data");
    }
    out
}

fn rgba(w: u32, h: u32) -> Vec<u8> {
    encoded(w, h, png::ColorType::Rgba, png::BitDepth::Eight, |_| {})
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    [&(data.len() as u32).to_be_bytes()[..], kind, data, &crc.sum().to_be_bytes()].concat()
}

fn handmade(raw: &[u8]) -> Vec<u8> {
    handmade_with(raw, false)
}

fn handmade_with(raw: &[u8], interlaced: bool) -> Vec<u8> {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    z.write_all(raw).expect("compress");
    let idat = z.finish().expect("compress");
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&96u32.to_be_bytes());
    ihdr[4..8].copy_from_slice(&96u32.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    ihdr[12] = u8::from(interlaced);
    [&SIGNATURE[..], &chunk(b"IHDR", &ihdr), &chunk(b"IDAT", &idat), &chunk(b"IEND", &[])].concat()
}

const RAW: usize = 96 * (1 + 96 * 4);

#[test]
fn icons_of_every_colour_type_read_as_rgba() {
    use png::BitDepth::*;
    use png::ColorType::*;
    for (color, depth) in [(Rgba, Eight), (Rgb, Eight), (Grayscale, Eight), (GrayscaleAlpha, Eight), (Rgba, Sixteen)] {
        let png = encoded(96, 96, color, depth, |_| {});
        let icon = read_icon(&png).unwrap_or_else(|e| panic!("{color:?} {depth:?}: {e}"));
        assert_eq!(icon.rgba.len(), 96 * 96 * 4);
        assert_eq!(hash_hex(&png), crate::keys::to_hex(&icon.hash));
    }
    let palette = encoded(96, 96, Indexed, Eight, |e| {
        e.set_palette((0..7u8).flat_map(|i| [i * 30, 0, 255 - i * 30]).collect::<Vec<u8>>());
        e.set_trns(vec![0u8, 128, 255, 255, 255, 255, 255]);
    });
    let icon = read_icon(&palette).expect("a palette with transparency");
    assert_eq!((icon.rgba[3], icon.rgba[7]), (0, 128), "tRNS became alpha");
    let adam7: usize = [(12, 12), (12, 12), (24, 12), (24, 24), (48, 24), (48, 48), (96, 48)]
        .iter()
        .map(|&(w, h): &(usize, usize)| h * (1 + w * 4))
        .sum();
    assert_eq!(adam7, 37_044);
    assert!(read_icon(&handmade_with(&vec![0u8; adam7], true)).is_ok());
    assert!(read_icon(&handmade_with(&vec![0u8; RAW], true)).is_err(), "the size of the other layout");
}

#[test]
fn the_wrong_size_is_refused_before_decoding() {
    for (w, h) in [(95, 96), (96, 97), (1, 1), (192, 192)] {
        let e = read_icon(&rgba(w, h)).expect_err("another size");
        assert!(e.contains(&format!("{w} x {h}")), "{e}");
    }
    let mut bomb = rgba(96, 96);
    bomb[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    bomb[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    assert_eq!(dimensions(&bomb), Ok((100_000, 100_000)));
    assert!(read_icon(&bomb).expect_err("huge").contains("100000 x 100000"));
    assert!(read_icon(&vec![0u8; MAX_ICON_BYTES + 1]).expect_err("too big").contains("bytes"));
}

#[test]
fn truncated_and_altered_icons_are_refused_without_a_panic() {
    let good = rgba(96, 96);
    for cut in 0..good.len() {
        assert!(read_icon(&good[..cut]).is_err(), "cut at {cut}");
    }
    for i in 0..good.len() {
        let mut bad = good.clone();
        bad[i] ^= 0x10;
        assert!(read_icon(&bad).is_err(), "byte {i}");
    }
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(read_icon(&trailing).expect_err("trailing").contains("after its end"));
}

#[test]
fn a_decompression_bomb_is_refused() {
    let mut raw = vec![0u8; RAW];
    raw.resize(RAW + 40_000_000, 0);
    let bomb = handmade(&raw);
    assert!(bomb.len() <= MAX_ICON_BYTES, "{}", bomb.len());
    assert!(read_icon(&bomb).expect_err("a bomb").contains("decompression bomb"));
    assert!(read_icon(&handmade(&vec![0u8; RAW - 1])).expect_err("short").contains("inflates to"));
    assert!(read_icon(&handmade(&vec![0u8; RAW])).is_ok());
}

#[test]
fn animation_and_unknown_critical_chunks_are_refused() {
    let good = rgba(96, 96);
    let iend = good.len() - 12;
    let with = |c: Vec<u8>, at: usize| [&good[..at], &c[..], &good[at..]].concat();
    let actl = chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]);
    assert!(read_icon(&with(actl, 33)).expect_err("animated").contains("animated"));
    assert!(read_icon(&with(chunk(b"ABCD", &[]), 33)).expect_err("critical").contains("critical"));
    assert!(read_icon(&with(chunk(b"teXt", b"a"), iend)).is_ok(), "an ancillary chunk after the data");
    assert!(read_icon(&with(chunk(b"IHDR", &good[16..29]), 33)).is_err(), "a second header");
    let many: Vec<u8> = (0..70).flat_map(|_| chunk(b"teXt", b"a")).collect();
    assert!(read_icon(&with(many, 33)).expect_err("chunks").contains("chunks"));
}
