use crate::{Error, Result};

pub const PACKET_SIZE: usize = 2048;
pub const PACKET_HEADER_BITS: usize = 32;
pub const FRAME_LENGTH_BITS: usize = 15;
pub const SAMPLES_PER_FRAME: u32 = 512;
pub const NO_MORE_FRAMES: u32 = 0x7FFF;

#[derive(Clone, Copy, Debug)]
pub struct SubBlock<'a> {
    pub rate_index: u8,
    pub packets: &'a [u8],
}

pub fn split_block(payload: &[u8], streams: usize) -> Result<(Vec<SubBlock<'_>>, usize)> {
    let mut out = Vec::with_capacity(streams);
    let mut pos = 0;
    for s in 0..streams {
        let word = payload
            .get(pos..pos + 4)
            .map(|b| u32::from_be_bytes(b.try_into().expect("4 bytes")))
            .ok_or_else(|| Error::invalid(format!("EA-XMA substream {s} header past block end")))?;
        let size = (word >> 2) as usize;
        if size < 4 || pos + size > payload.len() {
            return Err(Error::invalid(format!("EA-XMA substream {s}: size {size} at {pos} of {}", payload.len())));
        }
        out.push(SubBlock { rate_index: (word & 3) as u8, packets: &payload[pos + 4..pos + size] });
        pos += size;
    }
    Ok((out, payload.len() - pos))
}

pub fn rate_index_for(sample_rate: u32) -> u8 {
    match sample_rate {
        0..=24000 => 0,
        24001..=32000 => 1,
        32001..=44100 => 2,
        _ => 3,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PacketHeader {
    pub frame_count: u8,
    pub first_frame_bits: u16,
    pub metadata: u8,
    pub skip_count: u8,
}

impl PacketHeader {
    pub fn parse(p: &[u8]) -> Self {
        Self {
            frame_count: p[0] >> 2,
            first_frame_bits: (((p[0] as u16) & 3) << 13) | ((p[1] as u16) << 5) | ((p[2] as u16) >> 3),
            metadata: p[2] & 7,
            skip_count: p[3],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FrameWalk {
    pub packets: usize,
    pub frames: u32,
    pub header_frames: u32,
    pub offset_mismatches: usize,
    pub non_xma2_packets: usize,
    pub tail_bits: usize,
}

fn bit(data: &[u8], payload_bit: usize) -> Option<u32> {
    let per = (PACKET_SIZE - 4) * 8;
    let (pkt, off) = (payload_bit / per, payload_bit % per);
    let byte = pkt * PACKET_SIZE + 4 + off / 8;
    data.get(byte).map(|b| ((b >> (7 - off % 8)) & 1) as u32)
}

fn read_bits(data: &[u8], pos: usize, n: usize) -> Option<u32> {
    (0..n).try_fold(0u32, |acc, i| Some((acc << 1) | bit(data, pos + i)?))
}

pub fn walk_frames(data: &[u8]) -> Result<FrameWalk> {
    let per = (PACKET_SIZE - 4) * 8;
    let packets = data.len().div_ceil(PACKET_SIZE);
    let payload_bits: usize = (0..packets).map(|k| (data.len() - k * PACKET_SIZE).min(PACKET_SIZE) * 8 - 32).sum();
    let mut w = FrameWalk { packets, ..Default::default() };
    let headers: Vec<PacketHeader> = (0..packets).map(|k| PacketHeader::parse(&data[k * PACKET_SIZE..])).collect();
    for h in &headers {
        w.header_frames += h.frame_count as u32;
        w.non_xma2_packets += (h.frame_count > 0 && h.metadata != 1) as usize;
    }
    let mut pos = headers.first().map_or(0, |h| h.first_frame_bits as usize);
    let mut seen_in_packet = vec![false; packets];
    while pos + FRAME_LENGTH_BITS <= payload_bits {
        let len = read_bits(data, pos, FRAME_LENGTH_BITS).ok_or_else(|| Error::invalid("XMA read past end"))?;
        let pkt = pos / per;
        if len == NO_MORE_FRAMES || len == 0 {
            let next = pkt + 1;
            if next >= packets {
                break;
            }
            pos = next * per + headers[next].first_frame_bits as usize;
            continue;
        }
        if !seen_in_packet[pkt] {
            seen_in_packet[pkt] = true;
            if pos % per != headers[pkt].first_frame_bits as usize {
                w.offset_mismatches += 1;
            }
        }
        if pos + len as usize > payload_bits {
            return Err(Error::invalid(format!("XMA frame at bit {pos} (len {len}) runs past the data")));
        }
        w.frames += 1;
        pos += len as usize;
    }
    w.tail_bits = payload_bits.saturating_sub(pos);
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(buf: &mut [u8], pos: &mut usize, v: u32, n: usize) {
        for i in (0..n).rev() {
            if (v >> i) & 1 != 0 {
                buf[*pos / 8] |= 0x80 >> (*pos % 8);
            }
            *pos += 1;
        }
    }

    #[test]
    fn packet_headers() {
        let h = PacketHeader::parse(&[0x1C, 0x00, 0x01, 0x00]);
        assert_eq!((h.frame_count, h.first_frame_bits, h.metadata, h.skip_count), (7, 0, 1, 0));
        let h = PacketHeader::parse(&[0x03, 0xFF, 0xF8, 0x00]);
        assert_eq!((h.frame_count, h.first_frame_bits, h.metadata), (0, 0x7FFF, 0));
        assert_eq!([8000, 22050, 32000, 36000, 44100, 48000].map(rate_index_for), [0, 0, 1, 2, 2, 3]);
    }

    #[test]
    fn frame_walk_and_split() {
        let mut pkt = vec![0u8; 40];
        pkt[..4].copy_from_slice(&[2 << 2, 0, 1, 0]);
        let mut pos = 32;
        put(&mut pkt, &mut pos, 100, 15);
        pos = 32 + 100;
        put(&mut pkt, &mut pos, 50, 15);
        pos = 32 + 150;
        put(&mut pkt, &mut pos, NO_MORE_FRAMES, 15);
        let w = walk_frames(&pkt).unwrap();
        assert_eq!((w.packets, w.frames, w.header_frames, w.offset_mismatches, w.non_xma2_packets), (1, 2, 2, 0, 0));
        let mut payload = (((4 + pkt.len()) as u32) << 2 | 3).to_be_bytes().to_vec();
        payload.extend(&pkt);
        payload.extend([0, 0]);
        let (subs, rest) = split_block(&payload, 1).unwrap();
        assert_eq!((subs[0].rate_index, subs[0].packets.len(), rest), (3, 40, 2));
    }
}
