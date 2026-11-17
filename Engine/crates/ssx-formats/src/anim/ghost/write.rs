use std::io::Write as _;

use super::{
    CONTAINER_MAGIC, DESCRIPTOR_SIZE, GMPY, GhostFile, MAGIC, POSE_BLOCK_SIZE, POSI, REPLAY_BUFFER_SIZE, SCOR, Stream,
    UserHeader, VERSION, crc32,
};
use crate::{Error, Result};

pub const TABLE_OFFSET: u32 = 0x58;
pub const USER_HEADER_OFFSET: u32 = 0x1C;
pub const USER_HEADER_SIZE: u32 = 0x3C;
pub const GAMERTAG_MAX: usize = 0x31;
pub const RECORDER_BUILD: u32 = 0x6DA1D;
pub const RECORDER_INTERVAL: f32 = f32::from_bits(0x3EAA_AAAB);
pub const DEFLATE_LEVEL: u32 = 9;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StreamConfig {
    pub tag: [u8; 4],
    pub capacity: u32,
    pub element_size: u32,
    pub alignment: u32,
    pub interval: f32,
}

pub const RECORDER_STREAMS: [StreamConfig; 3] = [
    StreamConfig { tag: POSI, capacity: 1080, element_size: 12, alignment: 4, interval: RECORDER_INTERVAL },
    StreamConfig {
        tag: GMPY,
        capacity: 1080,
        element_size: POSE_BLOCK_SIZE as u32,
        alignment: 16,
        interval: RECORDER_INTERVAL,
    },
    StreamConfig { tag: SCOR, capacity: 1080, element_size: 4, alignment: 4, interval: RECORDER_INTERVAL },
];

impl Stream {
    pub fn push(&mut self, time: f32, element: &[u8]) -> bool {
        if self.count >= self.capacity || element.len() != self.element_size as usize {
            return false;
        }
        self.times.push(time);
        self.data.extend_from_slice(element);
        self.count += 1;
        true
    }
}

impl GhostFile {
    pub fn empty(header: UserHeader, streams: &[StreamConfig]) -> Result<Self> {
        let mut cursor = u64::from(TABLE_OFFSET) + (streams.len() * DESCRIPTOR_SIZE) as u64;
        let mut out = Vec::with_capacity(streams.len());
        for c in streams {
            if c.alignment == 0 || !c.alignment.is_power_of_two() {
                return Err(Error::invalid(format!("GHST: alignment {} is not a power of two", c.alignment)));
            }
            let times = (cursor + 3) & !3;
            let align = u64::from(c.alignment);
            let data = (times + 4 * u64::from(c.capacity) + align - 1) & !(align - 1);
            let size = u64::from(c.capacity) * u64::from(c.element_size);
            cursor = data + size;
            let (Ok(times_offset), Ok(data_offset), Ok(data_size)) =
                (u32::try_from(times), u32::try_from(data), u32::try_from(size))
            else {
                return Err(Error::invalid("GHST: the layout passes 4 GB"));
            };
            out.push(Stream {
                tag: c.tag,
                capacity: c.capacity,
                element_size: c.element_size,
                alignment: c.alignment,
                interval: c.interval,
                times_offset,
                data_offset,
                data_size,
                count: 0,
                times: Vec::new(),
                data: Vec::new(),
            });
        }
        if cursor > REPLAY_BUFFER_SIZE as u64 {
            return Err(Error::invalid(format!("GHST: {cursor} bytes, the buffer holds {REPLAY_BUFFER_SIZE}")));
        }
        Ok(GhostFile { crc: 0, version: VERSION, unk_1a: 0, header, streams: out })
    }

    pub fn image_len(&self) -> usize {
        let table = TABLE_OFFSET as usize + self.streams.len() * DESCRIPTOR_SIZE;
        self.streams
            .iter()
            .map(|s| {
                (s.times_offset as usize + 4 * s.capacity as usize).max(s.data_offset as usize + s.data_size as usize)
            })
            .fold(table, usize::max)
    }

    pub fn write(&self) -> Result<Vec<u8>> {
        let name = self.header.gamertag.as_bytes();
        if name.len() > GAMERTAG_MAX || name.contains(&0) {
            return Err(Error::invalid(format!(
                "GHST: gamertag of {} bytes (at most {GAMERTAG_MAX}, no NUL)",
                name.len()
            )));
        }
        let len = self.image_len();
        if len > REPLAY_BUFFER_SIZE {
            return Err(Error::invalid(format!("GHST: {len} bytes, the buffer holds {REPLAY_BUFFER_SIZE}")));
        }
        let table_end = TABLE_OFFSET as usize + self.streams.len() * DESCRIPTOR_SIZE;
        let mut out = vec![0u8; len];
        let put = |out: &mut [u8], at: usize, b: &[u8]| out[at..at + b.len()].copy_from_slice(b);
        put(&mut out, 4, MAGIC);
        put(&mut out, 8, &TABLE_OFFSET.to_be_bytes());
        put(&mut out, 0xC, &(self.streams.len() as u32).to_be_bytes());
        put(&mut out, 0x10, &USER_HEADER_OFFSET.to_be_bytes());
        put(&mut out, 0x14, &USER_HEADER_SIZE.to_be_bytes());
        put(&mut out, 0x18, &self.version.to_be_bytes());
        put(&mut out, 0x1A, &self.unk_1a.to_be_bytes());
        let u = USER_HEADER_OFFSET as usize;
        put(&mut out, u, name);
        put(&mut out, u + 0x32, &self.header.gear_flags.to_be_bytes());
        put(&mut out, u + 0x34, &self.header.score.to_be_bytes());
        put(&mut out, u + 0x38, &self.header.build.to_be_bytes());
        let mut regions: Vec<(usize, usize)> = Vec::new();
        for (i, s) in self.streams.iter().enumerate() {
            let label = String::from_utf8_lossy(&s.tag).into_owned();
            let used = s.count as usize * s.element_size as usize;
            if s.count > s.capacity
                || s.times.len() != s.count as usize
                || s.data.len() != used
                || u64::from(s.capacity) * u64::from(s.element_size) != u64::from(s.data_size)
                || s.alignment == 0
                || s.data_offset % s.alignment != 0
                || s.times_offset % 4 != 0
            {
                return Err(Error::invalid(format!(
                    "GHST {label}: {} samples of {} ({} times, {} bytes) do not fit its descriptor",
                    s.count,
                    s.capacity,
                    s.times.len(),
                    s.data.len()
                )));
            }
            let times = (s.times_offset as usize, s.times_offset as usize + 4 * s.capacity as usize);
            let data = (s.data_offset as usize, s.data_offset as usize + s.data_size as usize);
            for r in [times, data] {
                let overlaps = regions.iter().any(|&(a, b)| r.0 < b && a < r.1);
                if r.0 < table_end || overlaps {
                    return Err(Error::invalid(format!("GHST {label}: region {:#x}..{:#x} overlaps", r.0, r.1)));
                }
                regions.push(r);
            }
            let d = TABLE_OFFSET as usize + i * DESCRIPTOR_SIZE;
            put(&mut out, d, &s.tag);
            for (k, w) in [s.capacity, s.element_size, s.alignment, s.interval.to_bits(), s.times_offset]
                .into_iter()
                .chain([s.data_offset, s.data_size, s.count])
                .enumerate()
            {
                put(&mut out, d + 4 + 4 * k, &w.to_be_bytes());
            }
            for (k, t) in s.times.iter().enumerate() {
                put(&mut out, times.0 + 4 * k, &t.to_be_bytes());
            }
            put(&mut out, data.0, &s.data);
        }
        let crc = crc32(&out[4..]);
        put(&mut out, 0, &crc.to_be_bytes());
        Ok(out)
    }
}

pub fn compress(image: &[u8]) -> Result<Vec<u8>> {
    let size = u32::try_from(image.len()).map_err(|_| Error::invalid("GHST: image above 4 GB"))?;
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(DEFLATE_LEVEL));
    z.write_all(image).map_err(|e| Error::Decompress(format!("GCMP deflate: {e}")))?;
    let body = z.finish().map_err(|e| Error::Decompress(format!("GCMP deflate: {e}")))?;
    if body.len() > REPLAY_BUFFER_SIZE - 8 {
        return Ok(image.to_vec());
    }
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(CONTAINER_MAGIC);
    out.extend_from_slice(&body);
    Ok(out)
}

pub fn read_any(data: &[u8]) -> Result<GhostFile> {
    if data.len() > REPLAY_BUFFER_SIZE {
        return Err(Error::invalid(format!(
            "ghost: {} bytes, the replay buffer holds {REPLAY_BUFFER_SIZE}",
            data.len()
        )));
    }
    match data.get(4..8) {
        Some(m) if m == CONTAINER_MAGIC => GhostFile::from_container(data),
        Some(m) if m == MAGIC => GhostFile::parse(data),
        _ => Err(Error::invalid("ghost: neither GCMP nor GHST at +4")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> UserHeader {
        UserHeader { gamertag: "Probe".into(), gear_flags: 2, score: 77_532, build: RECORDER_BUILD }
    }

    #[test]
    fn the_recorder_layout_is_the_disc_files() {
        let g = GhostFile::empty(header(), &RECORDER_STREAMS).unwrap();
        let offsets: Vec<(u32, u32, u32)> =
            g.streams.iter().map(|s| (s.times_offset, s.data_offset, s.data_size)).collect();
        assert_eq!(offsets, [(0xC4, 0x11A4, 12_960), (0x4444, 0x5530, 259_200), (0x449B0, 0x45A90, 4320)]);
        assert_eq!(g.image_len(), 0x46B70);
    }

    #[test]
    fn a_recording_round_trips_through_the_writer_and_the_reader() {
        let mut g = GhostFile::empty(header(), &RECORDER_STREAMS).unwrap();
        for k in 0..5u32 {
            let t = k as f32 * RECORDER_INTERVAL;
            assert!(g.streams[0].push(t, &[k as u8; 12]));
            assert!(g.streams[1].push(t, &[(k + 1) as u8; POSE_BLOCK_SIZE]));
            assert!(g.streams[2].push(t, &(k as f32 * 10.0).to_be_bytes()));
        }
        let image = g.write().unwrap();
        assert_eq!(image.len(), 0x46B70);
        let back = GhostFile::parse(&image).unwrap();
        assert_eq!(back.header, g.header);
        assert_eq!(back.streams.iter().map(|s| s.count).collect::<Vec<_>>(), [5, 5, 5]);
        assert_eq!(back.write().unwrap(), image);
        let gcmp = compress(&image).unwrap();
        assert_eq!(&gcmp[4..8], CONTAINER_MAGIC);
        assert_eq!(&gcmp[8..10], [0x78, 0xDA]);
        assert_eq!(read_any(&gcmp).unwrap(), back);
        assert_eq!(read_any(&image).unwrap(), back);
        assert_eq!(back.scores().unwrap(), [0.0, 10.0, 20.0, 30.0, 40.0]);
    }

    #[test]
    fn a_full_stream_takes_no_more() {
        let mut g = GhostFile::empty(header(), &RECORDER_STREAMS).unwrap();
        for k in 0..1080 {
            assert!(g.streams[2].push(k as f32, &[0; 4]));
        }
        assert!(!g.streams[2].push(1080.0, &[0; 4]));
        assert!(!g.streams[0].push(0.0, &[0; 4]), "a wrong element size is refused");
    }

    #[test]
    fn damaged_ghosts_are_refused() {
        let mut g = GhostFile::empty(header(), &RECORDER_STREAMS).unwrap();
        g.streams[0].push(0.0, &[1; 12]);
        let image = g.write().unwrap();
        let mut bad = image.clone();
        bad[0x2000] ^= 1;
        assert!(GhostFile::parse(&bad).is_err());
        let gcmp = compress(&image).unwrap();
        assert!(read_any(&gcmp[..gcmp.len() - 9]).is_err());
        let mut big = gcmp.clone();
        big[..4].copy_from_slice(&(REPLAY_BUFFER_SIZE as u32 + 1).to_be_bytes());
        assert!(read_any(&big).is_err());
        let mut short = gcmp.clone();
        short[..4].copy_from_slice(&1000u32.to_be_bytes());
        assert!(read_any(&short).is_err(), "an inflate past the stated size");
        assert!(read_any(&[0u8; 16]).is_err());
        assert!(read_any(&vec![0u8; REPLAY_BUFFER_SIZE + 1]).is_err());
        g.header.gamertag = "x".repeat(GAMERTAG_MAX + 1);
        assert!(g.write().is_err());
    }
}
