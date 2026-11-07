pub mod av1;
mod bits;
pub mod card;
mod demux;
pub mod ebml;
mod header;
pub mod vorbis;
pub mod write;

#[cfg(test)]
mod tests;

pub use demux::{Demuxer, Packet, Track, Walked, walk};
pub use header::{AudioTrack, Colour, MovieInfo, VideoTrack};

use std::fmt;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub min_side: u32,
    pub min_frame_ns: u64,
    pub max_duration_ns: u64,
    pub max_overrun_ns: u64,
    pub max_video_private: u64,
    pub max_audio_private: u64,
    pub max_video_block: u64,
    pub max_audio_block: u64,
    pub max_lace_frames: usize,
    pub max_string: u64,
    pub min_rate: u32,
    pub max_rate: u32,
    pub max_vorbis_tables: u64,
    pub max_obus: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_file_bytes: crate::limits::MAX_FILE_BYTES,
            max_width: 1920,
            max_height: 1080,
            min_side: 16,
            min_frame_ns: 1_000_000_000 / 60,
            max_duration_ns: 1_800 * 1_000_000_000,
            max_overrun_ns: 1_000_000_000,
            max_video_private: 4 * 1024,
            max_audio_private: 256 * 1024,
            max_video_block: 8 * 1024 * 1024,
            max_audio_block: 1024 * 1024,
            max_lace_frames: 64,
            max_string: 4 * 1024,
            min_rate: 8_000,
            max_rate: 192_000,
            max_vorbis_tables: 8 * 1024 * 1024,
            max_obus: 1_024,
        }
    }
}

pub const CHANNELS: [u8; 3] = [1, 2, 6];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovieError {
    pub at: u64,
    pub message: String,
}

impl MovieError {
    pub fn new(at: u64, message: impl Into<String>) -> Self {
        Self { at, message: message.into() }
    }
}

impl fmt::Display for MovieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: {}", self.at, self.message)
    }
}

impl std::error::Error for MovieError {}

pub type Result<T> = std::result::Result<T, MovieError>;

pub fn sniff(head: &[u8]) -> Option<&'static str> {
    let at = |o: usize, m: &[u8]| head.get(o..o + m.len()) == Some(m);
    if at(0, &[0x1A, 0x45, 0xDF, 0xA3]) {
        return None;
    }
    Some(if at(0, b"MVhd") {
        "an EA movie (VP6, the disc's own format)"
    } else if at(0, b"OggS") {
        "an OGG stream"
    } else if at(0, b"RIFF") {
        "a RIFF file (AVI or WAV)"
    } else if at(4, b"ftyp") {
        "an MP4 / QuickTime file"
    } else if at(0, b"FLV") {
        "a Flash video"
    } else if at(0, b"GIF8") || at(0, b"\x89PNG") {
        "a picture"
    } else {
        "not a WebM file (no EBML header)"
    })
}
