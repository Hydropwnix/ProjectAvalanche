use std::sync::Arc;

use symphonia::core::checksum::{Crc8Ccitt, Crc16Ansi, Md5};
use symphonia::core::io::Monitor;

use super::*;

fn wav16(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
    let data = samples.len() as u32 * 2;
    let align = channels * 2;
    let mut v = Vec::new();
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    v.extend_from_slice(&align.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

fn tone(frames: usize, channels: usize) -> Vec<i16> {
    (0..frames * channels).map(|i| (((i / channels) as f32 * 0.05).sin() * 12_000.0) as i16).collect()
}

fn flac16(channels: u16, rate: u32, samples: &[i16], stated_frames: Option<u64>, md5: Option<[u8; 16]>) -> Vec<u8> {
    let frames = samples.len() / usize::from(channels);
    let mut hash = Md5::default();
    for s in samples {
        hash.process_buf_bytes(&s.to_le_bytes());
    }
    let mut v = b"fLaC".to_vec();
    v.extend_from_slice(&[0x80, 0, 0, 34]);
    v.extend_from_slice(&256u16.to_be_bytes());
    v.extend_from_slice(&256u16.to_be_bytes());
    v.extend_from_slice(&[0; 6]);
    let total = stated_frames.unwrap_or(frames as u64);
    let packed = u64::from(rate) << 44 | u64::from(channels - 1) << 41 | 15u64 << 36 | total;
    v.extend_from_slice(&packed.to_be_bytes());
    v.extend_from_slice(&md5.unwrap_or_else(|| hash.md5()));
    for (n, block) in samples.chunks(256 * usize::from(channels)).enumerate() {
        let len = block.len() / usize::from(channels);
        let start = v.len();
        v.extend_from_slice(&[0xFF, 0xF8, 0x70, ((channels - 1) as u8) << 4 | 0x8]);
        assert!(n < 128, "one-byte frame numbers in the test encoder");
        v.push(n as u8);
        v.extend_from_slice(&((len - 1) as u16).to_be_bytes());
        let mut crc8 = Crc8Ccitt::new(0);
        crc8.process_buf_bytes(&v[start..]);
        v.push(crc8.crc());
        for c in 0..usize::from(channels) {
            v.push(0x02);
            for f in 0..len {
                v.extend_from_slice(&block[f * usize::from(channels) + c].to_be_bytes());
            }
        }
        let mut crc16 = Crc16Ansi::new(0);
        crc16.process_buf_bytes(&v[start..]);
        v.extend_from_slice(&crc16.crc().to_be_bytes());
    }
    v
}

fn bytes(v: Vec<u8>) -> Arc<[u8]> {
    v.into()
}

#[test]
fn wav_and_flac_decode_to_their_samples() {
    let s = tone(1000, 2);
    let pcm = decode_file("a.wav", bytes(wav16(2, 32_000, &s)), &Limits::sound()).unwrap();
    assert_eq!((pcm.channels, pcm.rate, pcm.frames()), (2, 32_000, 1000));
    assert!(pcm.samples.iter().zip(&s).all(|(a, b)| (a - f32::from(*b) / 32768.0).abs() < 1e-6));
    let pcm = decode_file("a.flac", bytes(flac16(2, 44_100, &s, None, None)), &Limits::sound()).unwrap();
    assert_eq!((pcm.channels, pcm.rate, pcm.frames()), (2, 44_100, 1000));
    assert!(pcm.samples.iter().zip(&s).all(|(a, b)| (a - f32::from(*b) / 32768.0).abs() < 1e-6));
}

fn refused(name: &str, file: Vec<u8>, limits: &Limits) -> String {
    match decode_file(name, bytes(file), limits) {
        Ok(p) => panic!("{name} decoded to {} frames", p.frames()),
        Err(e) => e.message,
    }
}

#[test]
fn the_containers_are_held_to_their_structure() {
    let w = wav16(1, 48_000, &tone(500, 1));
    let cut = w[..w.len() - 7].to_vec();
    assert!(refused("cut.wav", cut, &Limits::sound()).contains("states"));
    let mut odd = w.clone();
    odd[40..44].copy_from_slice(&999u32.to_le_bytes());
    assert!(refused("odd.wav", odd.clone(), &Limits::sound()).contains("not whole frames"));
    odd[40..44].copy_from_slice(&99_998u32.to_le_bytes());
    assert!(refused("long.wav", odd, &Limits::sound()).contains("past the file's end"));
    let f = flac16(1, 48_000, &tone(600, 1), None, None);
    let msg = refused("cut.flac", f[..f.len() - 100].to_vec(), &Limits::sound());
    assert!(msg.contains("ends after") || msg.contains("MD5") || msg.contains("damaged"), "{msg}");
    assert!(refused("x.ogg", w.clone(), &Limits::sound()).contains("named OGG Vorbis, but its content is WAV"));
    assert!(refused("x.mp3", w, &Limits::sound()).contains("not .ogg, .flac or .wav"));
}

#[test]
fn the_caps_hold_before_anything_is_decoded() {
    let mut many = wav16(1, 48_000, &[0; 255]);
    many[22..24].copy_from_slice(&255u16.to_le_bytes());
    many[28..32].copy_from_slice(&(48_000u32 * 510).to_le_bytes());
    many[32..34].copy_from_slice(&510u16.to_le_bytes());
    assert!(refused("many.wav", many, &Limits::sound()).contains("255 channels"));
    for (rate, what) in [(1u32, "1 Hz"), (1_000_000, "1000000 Hz")] {
        let file = wav16(1, rate, &tone(16, 1));
        assert!(refused("rate.wav", file, &Limits::sound()).contains(what), "{rate}");
    }
    let hours = flac16(1, 48_000, &tone(256, 1), Some(48_000 * 3600 * 3), None);
    let msg = refused("hours.flac", hours, &Limits::music());
    assert!(msg.contains("10800.0 s"), "{msg}");
    let big = flac16(6, 48_000, &tone(256, 6), Some(4_800_000), None);
    let msg = refused("big.flac", big, &Limits::sound());
    assert!(msg.contains("115200000 bytes decoded") && !msg.contains("channels"), "{msg}");
}

#[test]
fn streams_that_decode_to_garbage_are_refused() {
    let s = tone(1024, 1);
    let mut changed = s.clone();
    changed[700] = changed[700].wrapping_add(1000);
    let mut hash = Md5::default();
    for x in &s {
        hash.process_buf_bytes(&x.to_le_bytes());
    }
    let lying = flac16(1, 48_000, &changed, None, Some(hash.md5()));
    assert!(refused("md5.flac", lying, &Limits::sound()).contains("MD5"));
    let unset = flac16(1, 48_000, &s, None, Some([0; 16]));
    assert!(refused("nomd5.flac", unset, &Limits::sound()).contains("cannot be verified"));
    let mut float = wav16(1, 48_000, &[0; 8]);
    float[20..22].copy_from_slice(&3u16.to_le_bytes());
    float[28..32].copy_from_slice(&(48_000u32 * 4).to_le_bytes());
    float[32..34].copy_from_slice(&4u16.to_le_bytes());
    float[34..36].copy_from_slice(&32u16.to_le_bytes());
    float[44..48].copy_from_slice(&1.0e6f32.to_le_bytes());
    let msg = refused("loud.wav", float.clone(), &Limits::sound());
    assert!(msg.contains("decodes to 1000000"), "{msg}");
    float[44..48].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(refused("nan.wav", float, &Limits::sound()).contains("decodes to NaN"));
}

fn page(serial: u32, seq: u32, flags: u8, granule: u64, packets: &[&[u8]]) -> Vec<u8> {
    let mut lacing = Vec::new();
    for p in packets {
        lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
        lacing.push((p.len() % 255) as u8);
    }
    let mut v = b"OggS".to_vec();
    v.push(0);
    v.push(flags);
    v.extend_from_slice(&granule.to_le_bytes());
    v.extend_from_slice(&serial.to_le_bytes());
    v.extend_from_slice(&seq.to_le_bytes());
    v.extend_from_slice(&[0; 4]);
    v.push(lacing.len() as u8);
    v.extend_from_slice(&lacing);
    for p in packets {
        v.extend_from_slice(p);
    }
    let crc = ogg::page_crc(&v);
    v[22..26].copy_from_slice(&crc.to_le_bytes());
    v
}

fn vorbis_id(channels: u8, rate: u32) -> Vec<u8> {
    let mut p = vec![1];
    p.extend_from_slice(b"vorbis");
    p.extend_from_slice(&0u32.to_le_bytes());
    p.push(channels);
    p.extend_from_slice(&rate.to_le_bytes());
    p.extend_from_slice(&[0; 12]);
    p.push(0xB8);
    p.push(1);
    p
}

fn ogg(pages: &[Vec<u8>]) -> Vec<u8> {
    pages.concat()
}

#[test]
fn ogg_pages_are_walked_strictly() {
    let id = vorbis_id(2, 44_100);
    let (comment, setup): (&[u8], &[u8]) = (b"\x03vorbis....", b"\x05vorbis....");
    let audio: &[u8] = &[0, 1, 2, 3];
    let good = |granule: u64| {
        vec![
            page(7, 0, 0x02, 0, &[&id]),
            page(7, 1, 0, 0, &[comment, setup]),
            page(7, 2, 0, 1024, &[audio, audio]),
            page(7, 3, 0x04, granule, &[audio]),
        ]
    };
    let info = walk("a.ogg", &ogg(&good(2048))).unwrap();
    assert_eq!((info.channels, info.rate, info.frames), (2, 44_100, 2048));
    let msg = probe("h.ogg", &ogg(&good(44_100 * 3600 * 2)), &Limits::music()).unwrap_err().message;
    assert!(msg.contains("7200.0 s"), "{msg}");
    let mut p = good(2048);
    p[2][30] ^= 1;
    assert!(walk("crc.ogg", &ogg(&p)).unwrap_err().message.contains("CRC"));
    let mut p = good(2048);
    p[3] = page(8, 3, 0x04, 2048, &[audio]);
    assert!(walk("two.ogg", &ogg(&p)).unwrap_err().message.contains("second logical stream"));
    let p = good(2048);
    assert!(walk("noeos.ogg", &ogg(&p[..3])).unwrap_err().message.contains("EOS"));
    let mut p = good(2048);
    p[3] = page(7, 4, 0x04, 2048, &[audio]);
    assert!(walk("gap.ogg", &ogg(&p)).unwrap_err().message.contains("lost or repeated"));
    let mut p = good(512);
    assert!(walk("back.ogg", &ogg(&p)).unwrap_err().message.contains("goes back"));
    p = good(2048);
    p.push(b"junk".to_vec());
    assert!(walk("tail.ogg", &ogg(&p)).unwrap_err().message.contains("after the last page"));
    let p = vec![page(7, 0, 0x02, 0, &[&vorbis_id(255, 44_100)]), page(7, 1, 0x04, 0, &[comment, setup])];
    assert!(walk("headers.ogg", &ogg(&p)).unwrap_err().message.contains("no audio"));
    let mut p = good(2048);
    p[0] = page(7, 0, 0x02, 0, &[&vorbis_id(255, 44_100)]);
    let msg = probe("c255.ogg", &ogg(&p), &Limits::sound()).unwrap_err().message;
    assert!(msg.contains("255 channels"), "{msg}");
    let mut p = good(2048);
    p[0] = page(7, 0, 0x02, 0, &[&vorbis_id(2, 1_000_000)]);
    assert!(probe("mhz.ogg", &ogg(&p), &Limits::sound()).unwrap_err().message.contains("1000000 Hz"));
}
