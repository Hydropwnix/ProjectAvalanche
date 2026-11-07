use std::io::Write;

pub const RED_BLUE_BC1: [u8; 8] = [0x00, 0xf8, 0x1f, 0x00, 0x04, 0, 0, 0];

pub fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("hex")).collect()
}

pub fn png_file(w: u32, h: u32, colour: png::ColorType, depth: png::BitDepth, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, w, h);
    e.set_color(colour);
    e.set_depth(depth);
    let mut writer = e.write_header().expect("header");
    writer.write_image_data(data).expect("data");
    writer.finish().expect("finish");
    out
}

pub fn palette_png(palette: &[[u8; 3]], indices: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, indices.len() as u32, 1);
    e.set_color(png::ColorType::Indexed);
    e.set_depth(png::BitDepth::Eight);
    e.set_palette(palette.concat());
    let mut writer = e.write_header().expect("header");
    writer.write_image_data(indices).expect("data");
    writer.finish().expect("finish");
    out
}

pub use crate::test_dir::png_header_only;

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).expect("zlib");
    e.finish().expect("zlib")
}

fn dds_header(w: u32, h: u32, mips: u32, pixel_format: [u32; 8], caps2: u32) -> Vec<u8> {
    let mut out = b"DDS ".to_vec();
    let mut header = [0u32; 31];
    header[0] = 124;
    header[1] = 0x1 | 0x2 | 0x4 | 0x1000 | 0x2_0000;
    (header[2], header[3], header[6]) = (h, w, mips);
    header[18..26].copy_from_slice(&pixel_format);
    (header[26], header[27]) = (0x1000, caps2);
    header.iter().for_each(|v| out.extend(v.to_le_bytes()));
    out
}

pub fn dds_legacy_fourcc(w: u32, h: u32, mips: u32, fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = dds_header(w, h, mips, [32, 0x4, u32::from_le_bytes(*fourcc), 0, 0, 0, 0, 0], 0);
    out.extend_from_slice(data);
    out
}

pub fn dds_legacy_rgb(w: u32, h: u32, bits: u32, masks: [u32; 4], data: &[u8]) -> Vec<u8> {
    let pf = [32, 0x40 | 0x1, 0, bits, masks[0], masks[1], masks[2], masks[3]];
    let mut out = dds_header(w, h, 1, pf, 0);
    out.extend_from_slice(data);
    out
}

pub fn dds_dx10(w: u32, h: u32, mips: u32, dxgi: u32, misc: u32, array: u32, data: &[u8]) -> Vec<u8> {
    let mut out = dds_header(w, h, mips, [32, 0x4, u32::from_le_bytes(*b"DX10"), 0, 0, 0, 0, 0], 0);
    [dxgi, 3, misc, array, 0].iter().for_each(|v| out.extend(v.to_le_bytes()));
    out.extend_from_slice(data);
    out
}

pub fn ktx2_file_with(format: u32, w: u32, h: u32, levels: u32, scheme: u32, data: &[(Vec<u8>, u64)]) -> Vec<u8> {
    let n = (levels as usize).max(data.len()).max(1);
    let dfd_at = 80 + 24 * n;
    let dfd: Vec<u8> = [28u32, 0, 2 | 24 << 16, 0, 0, 0, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut out = b"\xABKTX 20\xBB\r\n\x1A\n".to_vec();
    for v in [format, 1, w, h, 0, 0, 1, levels, scheme, dfd_at as u32, dfd.len() as u32, 0, 0] {
        out.extend(v.to_le_bytes());
    }
    out.extend([0u8; 16]);
    let mut at = (dfd_at + dfd.len()) as u64;
    for i in 0..n {
        let (bytes, stated) = data.get(i).map_or((&[][..], 0), |(b, s)| (&b[..], *s));
        for v in [at, bytes.len() as u64, stated] {
            out.extend(v.to_le_bytes());
        }
        at += bytes.len() as u64;
    }
    out.extend(&dfd);
    data.iter().for_each(|(b, _)| out.extend_from_slice(b));
    out
}

pub fn ktx2_file(format: u32, w: u32, h: u32, levels: u32, scheme: u32, data: &[Vec<u8>]) -> Vec<u8> {
    let with: Vec<(Vec<u8>, u64)> = data.iter().map(|d| (d.clone(), d.len() as u64)).collect();
    ktx2_file_with(format, w, h, levels, scheme, &with)
}
pub const JPEG_BASE: &str = concat!(
    "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303030304030304050805050404050a070706080c0a0c",
    "0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f171816141812141514ffdb00430103040405040509050509140d0b0d",
    "1414141414141414141414141414141414141414141414141414141414141414141414141414141414141414141414141414ffc00011",
    "080008001003011100021101031101ffc4001500010100000000000000000000000000000008ffc40018100002030000000000000000",
    "00000000000001166391ffc400160101010100000000000000000000000000000708ffc4001711000301000000000000000000000000",
    "00001763a1ffda000c03010002110311003f008da7d62d2e28f961a39c35d13eb16847cb0386ba7fffd9",
);
pub const JPEG_PROG: &str = concat!(
    "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303030304030304050805050404050a070706080c0a0c",
    "0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f171816141812141514ffdb00430103040405040509050509140d0b0d",
    "1414141414141414141414141414141414141414141414141414141414141414141414141414141414141414141414141414ffc20011",
    "080008001003011100021101031101ffc4001500010100000000000000000000000000000007ffc40016010101010000000000000000",
    "0000000000000607ffda000c030100021003100000018d5c68e3ffc4001510010100000000000000000000000000000014ffda000801",
    "0100010502bd7bffc400161100030000000000000000000000000000001662ffda0008010301013f0170a1c28fffc400161100030000",
    "000000000000000000000000001662ffda0008010201013f0147911e4fffc4001510010100000000000000000000000000000031ffda",
    "0008010100063f02aaffc40014100100000000000000000000000000000010ffda0008010100013f212bffda000c0301000200030000",
    "0010dbffc4001611000300000000000000000000000000000011f0ffda0008010301013f10b65b3fffc4001611000300000000000000",
    "000000000000000011f0ffda0008010201013f10944a3fffc400161000030000000000000000000000000000007191ffda0008010100",
    "013f1045114fffd9",
);
pub const JPEG_GREY: &str = concat!(
    "ffd8ffe000104a46494600010100000100010000ffdb0043000302020302020303030304030304050805050404050a070706080c0a0c",
    "0c0b0a0b0b0d0e12100d0e110e0b0b1016101113141515150c0f171816141812141514ffc0000b080008000801011100ffc400140001",
    "00000000000000000000000000000001ffc400171000030100000000000000000000000000001965a3ffda0008010100003f0015848c",
    "cfffd9",
);

pub const FUZZ_PNG: &str = concat!(
    "89504e470d0a1a0a0000000d49484452000000010000003b1000000000b1f43d1400000002664b4744ffff14ab31cd0000000774494d",
    "4507ea0a170a3918ec0a75f90000002574455874646174653a63726589504e470d0a1a0a0000000d4948445200000029000000e31002",
    "000001b70684e3000000206348524d00007a26000080840000fa31303a35373a32342b30303a30305e1056220000009cba513c000000",
    "2874455874646174653a74696d657374616d7000323032382d31302d323354cfcfc5cac8c5cdc42b30303a30307858cf410000007b49",
    "44415438cbed94c109c0200c455fa00bb8822bb8823b7585ee644770051dc115da4b29051b30d05b7db7f0f9f94904e537b891753130",
    "455874646174653a743032362d030000000000000031302d323354ffffffffffffffffffffff2874455874646174653a74696d657374",
    "616d7000323032372d31302d323a32342b30305e3a303010562200ae42006082002566647e89504e4700000000",
);

pub const FUZZ_JPEG: &str = concat!(
    "ffd8ffee000e41646f626500640000000002ffdb004300030202020202030202020303030304060404040404080606050609080a0a09",
    "0809090a0c0f0c0a0b0e0b09090d110d0e0f101011100a0c12131210130f1010fbffdb00430103030304030408040408100b090b1010",
    "101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010ffc000140800",
    "f9001004011100021101031101041100ffc400160001010100000000000000000000000000000708ffc4002010000101080300000000",
    "00000000000000001801020304161765a2535492ffc40017010101010100000000000000000000000000090308ffc4001f1100010206",
    "030000000000000000000000000104020315175391165152ffda000e040100021103110400003f00b5d430b5521d373c936f904cf477",
    "eda167893413263f516aa43a6e791c8267a16859e24d04c9d48fffd9",
);

pub const FUZZ_DDS: &str = concat!(
    "444453207c0000000f104000020000001b000000180000000400000004000000494d4147454d414749434b00000000b3000000000000",
    "00000000000000000000000000000000000000000000200000000200000000000000180000000000ff000000ff000040000000000000",
    "00000000000000004000444453207c0000000f104000020000000300000018000000040000000f0004000000494d4147454d41474943",
    "4b00000000b3000000000000000000000000000000000000000000000000000020000000040000000000005d5d5d5d5d5d5d5d5d5d5d",
    "00000044585432005d5d5d5d5d5d5d00ff2400000000000400000044585431000000000000000000ff00000000000000081040000000",
    "0000000000000000000000005d5d5d5d00ff2400",
);
