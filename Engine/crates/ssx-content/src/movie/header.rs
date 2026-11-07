use std::io::{Read, Seek};

use super::ebml::{Header, Reader};
use super::{CHANNELS, Limits, MovieError, Result, av1, vorbis};

pub(super) const EBML: u32 = 0x1A45_DFA3;
pub(super) const SEGMENT: u32 = 0x1853_8067;
pub(super) const CLUSTER: u32 = 0x1F43_B675;
const INFO: u32 = 0x1549_A966;
const TRACKS: u32 = 0x1654_AE6B;
pub(super) const VOID: u32 = 0xEC;
pub(super) const CRC32: u32 = 0xBF;
pub(super) const SKIPPED: [(u32, &str); 4] =
    [(0x114D_9B74, "SeekHead"), (0x1C53_BB6B, "Cues"), (0x1254_C367, "Tags"), (0x1043_A770, "Chapters")];
const ATTACHMENTS: u32 = 0x1941_A469;

#[derive(Clone, Debug, PartialEq)]
pub struct MovieInfo {
    pub timestamp_scale: u64,
    pub duration_ns: u64,
    pub muxing_app: String,
    pub writing_app: String,
    pub video: VideoTrack,
    pub audio: Option<AudioTrack>,
}

impl MovieInfo {
    pub fn duration_seconds(&self) -> f64 {
        self.duration_ns as f64 / 1e9
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoTrack {
    pub number: u64,
    pub width: u32,
    pub height: u32,
    pub default_duration_ns: Option<u64>,
    pub colour: Colour,
    pub config: av1::Config,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Colour {
    pub matrix: u64,
    pub range: u64,
    pub transfer: u64,
    pub primaries: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrack {
    pub number: u64,
    pub channels: u8,
    pub rate: u32,
    pub codec_private: Vec<u8>,
    pub headers: vorbis::Headers,
    pub codec_delay_ns: u64,
}

fn fail<T>(h: &Header, message: impl Into<String>) -> Result<T> {
    Err(MovieError::new(h.start, message))
}

pub(super) fn read_head<R: Read + Seek>(r: &mut Reader<R>, limits: &Limits) -> Result<(MovieInfo, u64, Header)> {
    let len = r.len();
    let h = r.header(len)?;
    if h.id != EBML || h.start != 0 {
        return fail(&h, "the file does not begin with an EBML header");
    }
    ebml_header(r, &h, limits)?;
    let seg = r.header(len)?;
    if seg.id != SEGMENT {
        return fail(&seg, format!("element 0x{:X} where the Segment comes", seg.id));
    }
    if seg.end() != len {
        let message = format!("{} bytes after the Segment's end (byte {})", len - seg.end(), seg.end());
        return Err(MovieError::new(seg.end(), message));
    }
    let (mut info, mut tracks) = (None, None);
    loop {
        if r.pos() >= seg.end() {
            return Err(MovieError::new(r.pos(), "the Segment holds no Cluster: no pictures"));
        }
        let c = r.header(seg.end())?;
        match c.id {
            INFO if info.is_none() => info = Some(read_info(r, &c, limits)?),
            TRACKS if tracks.is_none() => tracks = Some(read_tracks(r, &c, limits)?),
            INFO | TRACKS => return fail(&c, "a second Info or Tracks element"),
            CLUSTER => {
                let (Some((scale, duration, muxing_app, writing_app)), Some((video, audio))) = (info, tracks) else {
                    return fail(&c, "a Cluster before the Info and Tracks elements");
                };
                let out =
                    MovieInfo { timestamp_scale: scale, duration_ns: duration, muxing_app, writing_app, video, audio };
                return Ok((out, seg.end(), c));
            }
            ATTACHMENTS => return fail(&c, "attachments (WebM has none; a movie carries only its video and audio)"),
            VOID | CRC32 => r.skip(&c)?,
            id if SKIPPED.iter().any(|(s, _)| *s == id) => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in the Segment, which the reader does not know")),
        }
    }
}

fn ebml_header<R: Read + Seek>(r: &mut Reader<R>, h: &Header, limits: &Limits) -> Result<()> {
    let mut doc_type = None;
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0x4286 | 0x42F7 => {
                let v = r.uint(&c, "EBMLVersion")?;
                if v != 1 {
                    return fail(&c, format!("EBML version {v} (1)"));
                }
            }
            0x42F2 => {
                let v = r.uint(&c, "EBMLMaxIDLength")?;
                if v != 4 {
                    return fail(&c, format!("ids of up to {v} bytes (WebM: 4)"));
                }
            }
            0x42F3 => {
                let v = r.uint(&c, "EBMLMaxSizeLength")?;
                if !(1..=8).contains(&v) {
                    return fail(&c, format!("sizes of up to {v} bytes (1 to 8)"));
                }
            }
            0x4282 => doc_type = Some(r.string(&c, limits.max_string, "DocType")?),
            0x4287 => {
                r.uint(&c, "DocTypeVersion")?;
            }
            0x4285 => {
                let v = r.uint(&c, "DocTypeReadVersion")?;
                if !(1..=4).contains(&v) {
                    return fail(&c, format!("DocTypeReadVersion {v} (1 to 4)"));
                }
            }
            0x4281 | VOID | CRC32 => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in the EBML header, which the reader does not know")),
        }
    }
    match doc_type.as_deref() {
        Some("webm") => Ok(()),
        Some(other) => fail(
            h,
            format!(
                "a {} document, not WebM (a movie is WebM: AV1 video, Vorbis audio)",
                ssx_core::text::quoted(other)
            ),
        ),
        None => fail(h, "the EBML header names no document type"),
    }
}

fn read_info<R: Read + Seek>(r: &mut Reader<R>, h: &Header, limits: &Limits) -> Result<(u64, u64, String, String)> {
    let (mut scale, mut duration, mut muxing, mut writing) = (1_000_000u64, None, String::new(), String::new());
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0x2A_D7B1 => {
                scale = r.uint(&c, "TimestampScale")?;
                if !(1_000..=1_000_000_000).contains(&scale) {
                    return fail(&c, format!("a timestamp scale of {scale} ns (from 1 µs to 1 s)"));
                }
            }
            0x4489 => duration = Some((c, r.float(&c, "Duration")?)),
            0x4D80 => muxing = r.string(&c, limits.max_string, "MuxingApp")?,
            0x5741 => writing = r.string(&c, limits.max_string, "WritingApp")?,
            0x73A4 | 0x7384 | 0x3C_B923 | 0x3C_83AB | 0x3E_B923 | 0x3E_83BB | 0x4444 | 0x6924 | 0x4461 | 0x7BA9
            | VOID | CRC32 => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in Info, which the reader does not know")),
        }
    }
    let Some((at, ticks)) = duration else {
        return fail(h, "the movie states no duration (written to a pipe? write it to a file)");
    };
    let ns = ticks * scale as f64;
    if !(ns > 0.0 && ns <= limits.max_duration_ns as f64) {
        let cap = limits.max_duration_ns as f64 / 1e9;
        return fail(&at, format!("a duration of {:.3} s (more than 0, at most {cap} s)", ns / 1e9));
    }
    Ok((scale, ns.round() as u64, muxing, writing))
}

fn read_tracks<R: Read + Seek>(
    r: &mut Reader<R>,
    h: &Header,
    limits: &Limits,
) -> Result<(VideoTrack, Option<AudioTrack>)> {
    let (mut video, mut audio) = (None, None);
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0xAE => match read_entry(r, &c, limits)? {
                Entry::Video(v) if video.is_none() => video = Some(v),
                Entry::Video(_) => return fail(&c, "a second video track (a movie has one)"),
                Entry::Audio(a) if audio.is_none() => audio = Some(a),
                Entry::Audio(_) => return fail(&c, "a second audio track (a movie has at most one)"),
            },
            VOID | CRC32 => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in Tracks, which the reader does not know")),
        }
    }
    let video = video.ok_or_else(|| MovieError::new(h.start, "no video track"))?;
    if audio.as_ref().is_some_and(|a: &AudioTrack| a.number == video.number) {
        return fail(h, "the audio and video tracks share a number");
    }
    Ok((video, audio))
}

enum Entry {
    Video(VideoTrack),
    Audio(AudioTrack),
}

#[derive(Default)]
struct RawEntry {
    number: u64,
    kind: u64,
    codec: String,
    private: Option<Vec<u8>>,
    default_duration: Option<u64>,
    codec_delay: u64,
    video: Option<(u64, u64, Colour)>,
    audio: Option<(f64, u64)>,
}

fn read_entry<R: Read + Seek>(r: &mut Reader<R>, h: &Header, limits: &Limits) -> Result<Entry> {
    let mut e = RawEntry::default();
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0xD7 => e.number = r.uint(&c, "TrackNumber")?,
            0x83 => e.kind = r.uint(&c, "TrackType")?,
            0x86 => e.codec = r.string(&c, limits.max_string, "CodecID")?,
            0x63A2 => {
                let cap = limits.max_video_private.max(limits.max_audio_private);
                e.private = Some(r.binary(&c, cap, "CodecPrivate")?);
            }
            0x23_E383 => e.default_duration = Some(r.uint(&c, "DefaultDuration")?),
            0x56AA => e.codec_delay = r.uint(&c, "CodecDelay")?,
            0x23_314F => {
                if r.float(&c, "TrackTimestampScale")? != 1.0 {
                    return fail(&c, "a track timestamp scale other than 1");
                }
            }
            0x55EE => {
                if r.uint(&c, "MaxBlockAdditionID")? != 0 {
                    return fail(&c, "block additions (a movie's blocks carry only its pictures and sound)");
                }
            }
            0xE0 => e.video = Some(read_video(r, &c)?),
            0xE1 => e.audio = Some(read_audio(r, &c)?),
            0x6D80 => return fail(&c, "content encodings (compressed or encrypted blocks are refused)"),
            0x41E4 | 0x7446 | 0xE2 | 0x6624 => {
                return fail(&c, format!("element 0x{:X} (block additions, attachments, track operations)", c.id));
            }
            0x73C5 | 0xB9 | 0x88 | 0x55AA | 0x55AB | 0x55AC | 0x55AD | 0x55AE | 0x55AF | 0x9C | 0x6DE7 | 0x6DF8
            | 0x23_4E7A | 0x536E | 0x22_B59C | 0x22_B59D | 0x25_8688 | 0xAA | 0x6FAB | 0x56BB | VOID | CRC32 => {
                r.skip(&c)?
            }
            id => return fail(&c, format!("element 0x{id:X} in a TrackEntry, which the reader does not know")),
        }
    }
    if e.number == 0 {
        return fail(h, "a track without a number");
    }
    let private = |cap: u64| match &e.private {
        Some(p) if p.len() as u64 <= cap => Ok(p.clone()),
        Some(p) => fail(h, format!("{}: codec private data of {} bytes (at most {cap})", e.codec, p.len())),
        None => fail(h, format!("{}: no codec private data", e.codec)),
    };
    match (e.kind, e.codec.as_str()) {
        (1, "V_AV1") => {
            let Some((w, ht, colour)) = e.video else { return fail(h, "a video track without its Video element") };
            let (w, ht) = (
                fit(h, w, limits.max_width, limits.min_side, "width")?,
                fit(h, ht, limits.max_height, limits.min_side, "height")?,
            );
            if let Some(d) = e.default_duration
                && d < limits.min_frame_ns
            {
                let fps = 1e9 / d.max(1) as f64;
                return fail(
                    h,
                    format!("{fps:.0} pictures a second (at most {:.0})", 1e9 / limits.min_frame_ns as f64),
                );
            }
            let bytes = private(limits.max_video_private)?;
            let config =
                av1::Config::parse(&bytes, limits).map_err(|m| MovieError::new(h.start, format!("AV1: {m}")))?;
            if (config.sequence.max_width, config.sequence.max_height) != (w, ht) {
                let s = &config.sequence;
                return fail(
                    h,
                    format!("the track is {w} × {ht}, its AV1 sequence header {} × {}", s.max_width, s.max_height),
                );
            }
            let video = VideoTrack {
                number: e.number,
                width: w,
                height: ht,
                default_duration_ns: e.default_duration,
                colour,
                config,
            };
            Ok(Entry::Video(video))
        }
        (2, "A_VORBIS") => {
            let Some((rate, channels)) = e.audio else { return fail(h, "an audio track without its Audio element") };
            let bytes = private(limits.max_audio_private)?;
            let headers =
                vorbis::Headers::parse(&bytes, limits).map_err(|m| MovieError::new(h.start, format!("Vorbis: {m}")))?;
            if f64::from(headers.rate) != rate || u64::from(headers.channels) != channels {
                return fail(
                    h,
                    format!(
                        "the track states {channels} channels at {rate} Hz, its Vorbis header {} at {}",
                        headers.channels, headers.rate
                    ),
                );
            }
            if !CHANNELS.contains(&headers.channels) || !(limits.min_rate..=limits.max_rate).contains(&headers.rate) {
                return fail(
                    h,
                    format!(
                        "{} channels at {} Hz (1, 2 or 6 channels; {} to {} Hz)",
                        headers.channels, headers.rate, limits.min_rate, limits.max_rate
                    ),
                );
            }
            let max_delay = 1_000_000_000u64 * (1 << headers.blocksize_exp[1]) / u64::from(headers.rate);
            if e.codec_delay > max_delay {
                return fail(
                    h,
                    format!("a codec delay of {} ns (at most one long block, {max_delay} ns)", e.codec_delay),
                );
            }
            let audio = AudioTrack {
                number: e.number,
                channels: headers.channels,
                rate: headers.rate,
                codec_private: bytes,
                headers,
                codec_delay_ns: e.codec_delay,
            };
            Ok(Entry::Audio(audio))
        }
        (1 | 2, codec) => fail(
            h,
            format!(
                "a {} track of codec {} (a movie is AV1 video, V_AV1, with Vorbis audio, A_VORBIS)",
                if e.kind == 1 { "video" } else { "audio" },
                ssx_core::text::quoted(codec)
            ),
        ),
        (kind, codec) => fail(
            h,
            format!("a track of type {kind} ({}): a movie has only video and audio", ssx_core::text::quoted(codec)),
        ),
    }
}

fn fit(h: &Header, v: u64, max: u32, min: u32, what: &str) -> Result<u32> {
    match u32::try_from(v) {
        Ok(v) if (min..=max).contains(&v) => Ok(v),
        _ => fail(h, format!("a {what} of {v} pixels (from {min} to {max})")),
    }
}

fn read_video<R: Read + Seek>(r: &mut Reader<R>, h: &Header) -> Result<(u64, u64, Colour)> {
    let (mut w, mut ht, mut colour) = (0, 0, Colour::default());
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0xB0 => w = r.uint(&c, "PixelWidth")?,
            0xBA => ht = r.uint(&c, "PixelHeight")?,
            0x9A => {
                if r.uint(&c, "FlagInterlaced")? == 1 {
                    return fail(&c, "interlaced pictures (a movie is progressive)");
                }
            }
            0x53B8 | 0x53C0 => {
                if r.uint(&c, "StereoMode / AlphaMode")? != 0 {
                    return fail(&c, "stereo or alpha pictures (a movie is one opaque picture)");
                }
            }
            0x54AA | 0x54BB | 0x54CC | 0x54DD => {
                if r.uint(&c, "PixelCrop")? != 0 {
                    return fail(&c, "a cropped picture (encode the movie at the size it is shown)");
                }
            }
            0x7670 => return fail(&c, "a projection (360° pictures are not played)"),
            0x55B0 => colour = read_colour(r, &c)?,
            0x9D | 0x54B0 | 0x54BA | 0x54B2 | 0x54B3 | 0x2E_B524 | 0x2F_B523 | 0x23_83E3 | VOID | CRC32 => {
                r.skip(&c)?
            }
            id => return fail(&c, format!("element 0x{id:X} in Video, which the reader does not know")),
        }
    }
    Ok((w, ht, colour))
}

fn read_colour<R: Read + Seek>(r: &mut Reader<R>, h: &Header) -> Result<Colour> {
    let mut out = Colour::default();
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0x55B1 => out.matrix = r.uint(&c, "MatrixCoefficients")?,
            0x55B9 => out.range = r.uint(&c, "Range")?,
            0x55BA => out.transfer = r.uint(&c, "TransferCharacteristics")?,
            0x55BB => out.primaries = r.uint(&c, "Primaries")?,
            0x55B2..=0x55B8 | 0x55BC | 0x55BD | 0x55D0 | VOID | CRC32 => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in Colour, which the reader does not know")),
        }
    }
    Ok(out)
}

fn read_audio<R: Read + Seek>(r: &mut Reader<R>, h: &Header) -> Result<(f64, u64)> {
    let (mut rate, mut channels, mut output) = (8_000.0, 1, None);
    while r.pos() < h.end() {
        let c = r.header(h.end())?;
        match c.id {
            0xB5 => rate = r.float(&c, "SamplingFrequency")?,
            0x78B5 => output = Some(r.float(&c, "OutputSamplingFrequency")?),
            0x9F => channels = r.uint(&c, "Channels")?,
            0x6264 | 0x52F1 | VOID | CRC32 => r.skip(&c)?,
            id => return fail(&c, format!("element 0x{id:X} in Audio, which the reader does not know")),
        }
    }
    if output.is_some_and(|o| o != rate) {
        return fail(h, "an output rate other than the rate (spectral band replication is not Vorbis)");
    }
    Ok((rate, channels))
}
