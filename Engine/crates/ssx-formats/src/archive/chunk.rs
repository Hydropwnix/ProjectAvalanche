use std::io::Read;

use crate::lzx::LzxDecoder;
use crate::reader::Reader;
use crate::{Error, Result};

pub const MAGIC_LZX: &[u8; 8] = b"chunklzx";
pub const MAGIC_ZIP: &[u8; 8] = b"chunkzip";

const CODEC_DEFLATE: u32 = 1;
const CODEC_LZX: u32 = 3;
const CODEC_STORED: u32 = 4;

pub const LZX_WINDOW_BITS: u32 = 17;

const XMEM_FRAME_SIZE: usize = 32 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerKind {
    Lzx,
    Zip,
}

#[derive(Clone, Debug)]
pub struct ContainerHeader {
    pub kind: ContainerKind,
    pub version: u32,
    pub full_size: u32,
    pub block_size: u32,
    pub segment_count: u32,
    pub alignment: u32,
}

pub fn detect(data: &[u8]) -> Option<ContainerKind> {
    match data.get(..8)? {
        m if m == MAGIC_LZX => Some(ContainerKind::Lzx),
        m if m == MAGIC_ZIP => Some(ContainerKind::Zip),
        _ => None,
    }
}

pub fn read_header(data: &[u8]) -> Result<ContainerHeader> {
    let kind = detect(data).ok_or_else(|| Error::invalid("not a chunklzx/chunkzip container"))?;
    let mut r = Reader::be(data);
    r.skip(8)?;
    Ok(ContainerHeader {
        kind,
        version: r.u32()?,
        full_size: r.u32()?,
        block_size: r.u32()?,
        segment_count: r.u32()?,
        alignment: r.u32()?,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Segment<'a> {
    pub codec: u32,
    pub payload: &'a [u8],
    pub expected: usize,
}

pub fn segments(data: &[u8]) -> Result<(ContainerHeader, Vec<Segment<'_>>)> {
    let header = read_header(data)?;
    if header.version != 2 {
        return Err(Error::invalid(format!("unsupported container version {}", header.version)));
    }
    let align = header.alignment.max(1) as usize;
    let mut r = Reader::be(data);
    r.seek(0x1c)?;
    let mut left = header.full_size as usize;
    let mut segments = Vec::with_capacity(header.segment_count.min(1024) as usize);
    for _ in 0..header.segment_count {
        let header_pos = (r.pos() + 8).next_multiple_of(align) - 8;
        r.seek(header_pos)?;
        let stored_size = r.u32()? as usize;
        let codec = r.u32()?;
        let payload = r.bytes(stored_size)?;
        let expected = left.min(header.block_size as usize);
        left -= expected;
        segments.push(Segment { codec, payload, expected });
    }
    Ok((header, segments))
}

pub fn decompress_if_needed(data: &[u8]) -> Result<Vec<u8>> {
    if detect(data).is_some() { decompress(data) } else { Ok(data.to_vec()) }
}

pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    let (header, segments) = segments(data)?;
    let mut out = Vec::with_capacity(header.full_size as usize);
    let mut lzx: Option<LzxDecoder> = None;

    for (index, segment) in segments.iter().enumerate() {
        let before = out.len();
        match segment.codec {
            CODEC_LZX if segment.payload.is_empty() => {
                return Err(Error::EmptySegment { segment: index, expected: segment.expected });
            }
            CODEC_LZX => {
                let decoder = match &mut lzx {
                    Some(decoder) => {
                        decoder.reset();
                        decoder
                    }
                    None => lzx.insert(LzxDecoder::new(LZX_WINDOW_BITS)?),
                };
                decode_xmem_frames(decoder, segment.payload, segment.expected, &mut out)?;
            }
            CODEC_DEFLATE => {
                flate2::read::DeflateDecoder::new(segment.payload).read_to_end(&mut out)?;
            }
            CODEC_STORED => out.extend_from_slice(segment.payload),
            other => return Err(Error::Decompress(format!("unknown segment codec {other} in segment {index}"))),
        }
        if out.len() - before != segment.expected {
            return Err(Error::Decompress(format!(
                "segment {index} (codec {}) gave {} bytes, expected {}",
                segment.codec,
                out.len() - before,
                segment.expected
            )));
        }
    }

    if out.len() != header.full_size as usize {
        return Err(Error::Decompress(format!(
            "container decompressed to {} bytes, header says {}",
            out.len(),
            header.full_size
        )));
    }
    Ok(out)
}

pub fn decompress_xmem_lzx(payload: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
    decompress_xmem_lzx_with(payload, expected, LZX_WINDOW_BITS, out)
}

pub fn decompress_xmem_lzx_with(payload: &[u8], expected: usize, window_bits: u32, out: &mut Vec<u8>) -> Result<()> {
    let mut decoder = LzxDecoder::new(window_bits)?;
    decode_xmem_frames(&mut decoder, payload, expected, out)
}

fn decode_xmem_frames(lzx: &mut LzxDecoder, payload: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
    let mut r = Reader::be(payload);
    let mut produced = 0usize;
    while produced < expected {
        if r.remaining() == 0 {
            return Err(Error::Decompress(format!(
                "LZX stream ends after {produced} of {expected} bytes without an end frame"
            )));
        }
        let (frame_out, frame_in) = if r.peek(1)?[0] == 0xFF {
            r.skip(1)?;
            (r.u16()? as usize, r.u16()? as usize)
        } else {
            (XMEM_FRAME_SIZE.min(expected - produced), r.u16()? as usize)
        };
        if frame_in == 0 {
            break;
        }
        let at = r.pos();
        let frame = r.bytes(frame_in)?;
        lzx.decode_frame(frame, frame_out, out).map_err(|e| match e {
            Error::Decompress(msg) => Error::Decompress(format!("{msg} (frame at {at:#x})")),
            other => other,
        })?;
        produced += frame_out;
    }
    if produced != expected {
        return Err(Error::Decompress(format!("LZX stream gave {produced} bytes, expected {expected}")));
    }
    Ok(())
}
