mod decode;
mod flac;
mod head;
pub mod music;
mod music_score;
mod ogg;
pub mod sounds;
pub mod switches;
mod wav;

pub use decode::{Decoder, decode_file};
pub use head::{ReadRange, probe_head};

pub const FORMATS: [&str; 3] = ["ogg", "flac", "wav"];

pub const MAX_ABS_SAMPLE: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    OggVorbis,
    Flac,
    Wav,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::OggVorbis => "OGG Vorbis",
            Format::Flac => "FLAC",
            Format::Wav => "WAV",
        }
    }

    pub fn from_name(name: &str) -> Result<Self, AudioError> {
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        match ext.as_str() {
            "ogg" => Ok(Format::OggVorbis),
            "flac" => Ok(Format::Flac),
            "wav" => Ok(Format::Wav),
            _ => Err(AudioError::new(name, "not .ogg, .flac or .wav: an audio file is OGG Vorbis, FLAC or WAV")),
        }
    }

    fn magic(self) -> &'static [u8] {
        match self {
            Format::OggVorbis => b"OggS",
            Format::Flac => b"fLaC",
            Format::Wav => b"RIFF",
        }
    }
}

fn sniff(bytes: &[u8]) -> &'static str {
    [Format::OggVorbis, Format::Flac, Format::Wav]
        .into_iter()
        .find(|f| bytes.starts_with(f.magic()))
        .map_or("of no audio format", Format::name)
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}", ssx_core::text::located(.file, .message))]
pub struct AudioError {
    pub file: String,
    pub message: String,
}

impl AudioError {
    pub fn new(file: &str, message: impl Into<String>) -> Self {
        Self { file: file.to_string(), message: message.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub max_file_bytes: u64,
    pub channels: &'static [u16],
    pub min_rate: u32,
    pub max_rate: u32,
    pub max_seconds: f64,
    pub max_decoded_bytes: u64,
    pub target_rate: u32,
}

impl Limits {
    pub const fn sound() -> Self {
        Self {
            max_file_bytes: 64 << 20,
            channels: &[1, 2, 4, 6],
            min_rate: 8_000,
            max_rate: 192_000,
            max_seconds: 120.0,
            max_decoded_bytes: 64 << 20,
            target_rate: 48_000,
        }
    }

    pub const fn music() -> Self {
        Self {
            max_file_bytes: 256 << 20,
            channels: &[1, 2],
            min_rate: 8_000,
            max_rate: 192_000,
            max_seconds: 1800.0,
            max_decoded_bytes: 640 << 20,
            target_rate: 44_100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioInfo {
    pub format: Format,
    pub channels: u16,
    pub rate: u32,
    pub frames: u64,
    pub file_bytes: u64,
}

impl AudioInfo {
    pub fn seconds(&self) -> f64 {
        self.frames as f64 / f64::from(self.rate.max(1))
    }

    pub fn frames_at(&self, rate: u32) -> u64 {
        let n = u128::from(self.frames) * u128::from(rate);
        u64::try_from(n.div_ceil(u128::from(self.rate.max(1)))).unwrap_or(u64::MAX)
    }

    pub fn decoded_bytes(&self, rate: u32) -> u64 {
        self.frames_at(rate).saturating_mul(u64::from(self.channels)).saturating_mul(4)
    }

    pub fn check(&self, name: &str, limits: &Limits) -> Result<(), AudioError> {
        let mut bad = Vec::new();
        if self.file_bytes > limits.max_file_bytes {
            bad.push(format!("{} bytes (at most {})", self.file_bytes, limits.max_file_bytes));
        }
        if !limits.channels.contains(&self.channels) {
            bad.push(format!("{} channels (taken: {:?})", self.channels, limits.channels));
        }
        if self.rate < limits.min_rate || self.rate > limits.max_rate {
            bad.push(format!("{} Hz (from {} to {} Hz)", self.rate, limits.min_rate, limits.max_rate));
        }
        if self.frames == 0 {
            bad.push("no samples".into());
        } else if self.rate > 0 && self.seconds() > limits.max_seconds {
            bad.push(format!("{:.1} s (at most {} s)", self.seconds(), limits.max_seconds));
        }
        let (native, target) = (self.decoded_bytes(self.rate), self.decoded_bytes(limits.target_rate));
        let decoded = native.max(target);
        if decoded > limits.max_decoded_bytes {
            let at = if native >= target { self.rate } else { limits.target_rate };
            bad.push(format!("{decoded} bytes decoded at {at} Hz (at most {})", limits.max_decoded_bytes));
        }
        if bad.is_empty() { Ok(()) } else { Err(AudioError::new(name, bad.join("; "))) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub channels: u16,
    pub rate: u32,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
}

pub fn probe(name: &str, bytes: &[u8], limits: &Limits) -> Result<AudioInfo, AudioError> {
    let info = walk(name, bytes)?;
    info.check(name, limits)?;
    Ok(info)
}

pub fn walk(name: &str, bytes: &[u8]) -> Result<AudioInfo, AudioError> {
    let format = Format::from_name(name)?;
    if !bytes.starts_with(format.magic()) {
        let what = sniff(bytes);
        return Err(AudioError::new(name, format!("named {}, but its content is {what}", format.name())));
    }
    let file_bytes = bytes.len() as u64;
    let (channels, rate, frames) = match format {
        Format::Wav => wav::walk(bytes).map(|w| (w.channels, w.rate, w.frames)),
        Format::Flac => flac::walk(bytes).map(|f| (f.channels, f.rate, f.frames)),
        Format::OggVorbis => ogg::walk(bytes).map(|o| (o.channels, o.rate, o.frames)),
    }
    .map_err(|m| AudioError::new(name, m))?;
    Ok(AudioInfo { format, channels, rate, frames, file_bytes })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_records;
