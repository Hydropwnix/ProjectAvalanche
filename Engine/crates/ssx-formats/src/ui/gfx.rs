use std::io::Read;

pub mod avm1;
pub mod bits;
pub mod limits;
pub mod place;
pub mod scaleform;
pub mod shape;
pub mod text;
pub mod types;

use crate::{Error, Result};
use avm1::Action;
use bits::SwfReader;
pub use limits::{Budget, Limits};
use place::PlaceObject;
use scaleform::{ExporterInfo, ExternalImage, SubImage};
use shape::{DefineShape, MorphShape};
use text::{CsmTextSettings, EditText, Font, StaticText};
use types::{Rect, Rgba};

pub(crate) fn expect_end(r: &SwfReader, what: &str) -> Result<()> {
    if r.remaining() != 0 {
        return Err(Error::invalid(format!("{what}: {} bytes left at {:#x}", r.remaining(), r.pos())));
    }
    Ok(())
}

pub fn tag_name(code: u16) -> Option<&'static str> {
    Some(match code {
        0 => "End",
        1 => "ShowFrame",
        2 => "DefineShape",
        4 => "PlaceObject",
        5 => "RemoveObject",
        6 => "DefineBits",
        7 => "DefineButton",
        8 => "JPEGTables",
        9 => "SetBackgroundColor",
        10 => "DefineFont",
        11 => "DefineText",
        12 => "DoAction",
        13 => "DefineFontInfo",
        14 => "DefineSound",
        15 => "StartSound",
        17 => "DefineButtonSound",
        18 => "SoundStreamHead",
        19 => "SoundStreamBlock",
        20 => "DefineBitsLossless",
        21 => "DefineBitsJPEG2",
        22 => "DefineShape2",
        23 => "DefineButtonCxform",
        24 => "Protect",
        26 => "PlaceObject2",
        28 => "RemoveObject2",
        32 => "DefineShape3",
        33 => "DefineText2",
        34 => "DefineButton2",
        35 => "DefineBitsJPEG3",
        36 => "DefineBitsLossless2",
        37 => "DefineEditText",
        39 => "DefineSprite",
        43 => "FrameLabel",
        45 => "SoundStreamHead2",
        46 => "DefineMorphShape",
        48 => "DefineFont2",
        56 => "ExportAssets",
        57 => "ImportAssets",
        58 => "EnableDebugger",
        59 => "DoInitAction",
        60 => "DefineVideoStream",
        61 => "VideoFrame",
        62 => "DefineFontInfo2",
        64 => "EnableDebugger2",
        65 => "ScriptLimits",
        66 => "SetTabIndex",
        69 => "FileAttributes",
        70 => "PlaceObject3",
        71 => "ImportAssets2",
        72 => "DoABC (pre-release)",
        73 => "DefineFontAlignZones",
        74 => "CSMTextSettings",
        75 => "DefineFont3",
        76 => "SymbolClass",
        77 => "Metadata",
        78 => "DefineScalingGrid",
        82 => "DoABC",
        83 => "DefineShape4",
        84 => "DefineMorphShape2",
        86 => "DefineSceneAndFrameLabelData",
        87 => "DefineBinaryData",
        88 => "DefineFontName",
        89 => "StartSound2",
        90 => "DefineBitsJPEG4",
        91 => "DefineFont4",
        1000 => "ExporterInfo",
        1001 => "DefineExternalImage",
        1002 => "FontTextureInfo",
        1003 => "DefineExternalGradient",
        1004 => "DefineGradientMap",
        1005 => "DefineCompactedFont",
        1006 => "DefineExternalSound",
        1007 => "DefineExternalStreamSound",
        1008 => "DefineSubImage",
        1009 => "DefineExternalImage2",
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub id: u16,
    pub frame_count: u16,
    pub tags: Vec<TagRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    End,
    ShowFrame,
    Shape(DefineShape),
    SetBackgroundColor(Rgba),
    Text(StaticText),
    DoAction(Vec<Action>),
    PlaceObject(Box<PlaceObject>),
    RemoveObject2 { depth: u16 },
    EditText(EditText),
    Sprite(Sprite),
    MorphShape(MorphShape),
    Font(Font),
    ExportAssets(Vec<(u16, String)>),
    ImportAssets { url: String, unk_reserved: Option<[u8; 2]>, assets: Vec<(u16, String)> },
    DoInitAction { sprite_id: u16, actions: Vec<Action> },
    FileAttributes(u32),
    CsmTextSettings(CsmTextSettings),
    Metadata(String),
    ScalingGrid { id: u16, splitter: Rect },
    FrameLabel { name: String, anchor: Option<u8> },
    ExporterInfo(ExporterInfo),
    ExternalImage(ExternalImage),
    SubImage(SubImage),
    Unknown(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TagRecord {
    pub code: u16,
    pub offset: usize,
    pub long_header: bool,
    pub tag: Tag,
}

impl TagRecord {
    pub fn name(&self) -> &'static str {
        tag_name(self.code).unwrap_or("?")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Movie {
    pub signature: [u8; 3],
    pub version: u8,
    pub file_length: u32,
    pub frame_size: Rect,
    pub frame_rate: u16,
    pub frame_count: u16,
    pub tags: Vec<TagRecord>,
    pub unk_inflate_padding: usize,
}

pub fn decompress(file: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    decompress_with(file, &Limits::default())
}

pub fn decompress_with(file: &[u8], limits: &Limits) -> Result<(Vec<u8>, Vec<u8>)> {
    if file.len() < 8 {
        return Err(Error::UnexpectedEof { offset: 0, wanted: 8, available: file.len() });
    }
    if file.len() > limits.max_file_bytes {
        return Err(Error::invalid(format!("the file is {} bytes, the cap {}", file.len(), limits.max_file_bytes)));
    }
    let sig = &file[..3];
    let len = u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize;
    if len < 8 || len > limits.max_movie_bytes {
        return Err(Error::invalid(format!("the header states {len} bytes (8 to {})", limits.max_movie_bytes)));
    }
    let mut out = file[..8].to_vec();
    let padding = match sig {
        b"CFX" | b"CWS" => {
            let cap = len.saturating_add(limits.max_inflate_padding);
            let mut z = flate2::read::ZlibDecoder::new(&file[8..]).take(cap.saturating_sub(8) as u64 + 1);
            z.read_to_end(&mut out).map_err(|e| Error::Decompress(format!("movie zlib stream: {e}")))?;
            if out.len() > cap {
                return Err(Error::invalid(format!(
                    "the zlib stream inflates past {cap} bytes (the stated {len} + {} of padding)",
                    limits.max_inflate_padding
                )));
            }
            let total_in = z.into_inner().total_in() as usize;
            if total_in != file.len() - 8 {
                return Err(Error::invalid(format!("{} bytes after the zlib stream", file.len() - 8 - total_in)));
            }
            if out.len() < len {
                return Err(Error::invalid(format!("inflated to {} bytes, header says {len}", out.len())));
            }
            out.split_off(len)
        }
        b"GFX" | b"FWS" => {
            if file.len() != len {
                return Err(Error::invalid(format!("file is {} bytes, header says {len}", file.len())));
            }
            out = file.to_vec();
            Vec::new()
        }
        _ => return Err(Error::BadMagic { offset: 0, expected: b"CFX", found: sig.to_vec() }),
    };
    Ok((out, padding))
}

fn read_tags(data: &[u8], base: usize, swf_version: u8, nested: bool, budget: &Budget) -> Result<Vec<TagRecord>> {
    let mut r = SwfReader::new(data);
    let mut tags = Vec::new();
    loop {
        budget.tag()?;
        let offset = base + r.pos();
        let head = r.u16()?;
        let code = head >> 6;
        let short = usize::from(head & 0x3F);
        let long_header = short == 0x3F;
        let len = if long_header { r.u32()? as usize } else { short };
        let body_at = base + r.pos();
        let body = r.bytes(len)?;
        let tag = read_tag(code, body, body_at, swf_version, nested, budget).map_err(|e| {
            Error::invalid(format!("tag {code} ({}) at {offset:#x}: {e}", tag_name(code).unwrap_or("?")))
        })?;
        tags.push(TagRecord { code, offset, long_header, tag });
        if code == 0 {
            break;
        }
    }
    if !r.at_end() {
        return Err(Error::invalid(format!("{} bytes after the End tag at {:#x}", r.remaining(), base + r.pos())));
    }
    Ok(tags)
}

fn actions(code: &[u8], budget: &Budget) -> Result<Vec<Action>> {
    let a = avm1::decode_in(code, budget)?;
    avm1::validate(&a, code.len() as u32)?;
    Ok(a)
}

fn assets(r: &mut SwfReader) -> Result<Vec<(u16, String)>> {
    let n = r.u16()?;
    (0..n).map(|_| Ok((r.u16()?, r.string()?))).collect()
}

fn read_tag(code: u16, body: &[u8], body_at: usize, swf_version: u8, nested: bool, budget: &Budget) -> Result<Tag> {
    let mut r = SwfReader::new(body);
    let tag = match code {
        0 => Tag::End,
        1 => Tag::ShowFrame,
        2 | 22 | 32 | 83 => {
            let version = match code {
                2 => 1,
                22 => 2,
                32 => 3,
                _ => 4,
            };
            return Ok(Tag::Shape(DefineShape::read_in(body, version, budget)?));
        }
        9 => Tag::SetBackgroundColor(Rgba::read_rgb(&mut r)?),
        11 | 33 => return Ok(Tag::Text(StaticText::read_in(body, code == 33, budget)?)),
        12 => return Ok(Tag::DoAction(actions(body, budget)?)),
        26 | 70 => {
            return Ok(Tag::PlaceObject(Box::new(PlaceObject::read_in(
                body,
                if code == 26 { 2 } else { 3 },
                swf_version,
                budget,
            )?)));
        }
        28 => Tag::RemoveObject2 { depth: r.u16()? },
        37 => return Ok(Tag::EditText(EditText::read(body)?)),
        39 if !nested => {
            let id = r.u16()?;
            let frame_count = r.u16()?;
            budget.timeline(frame_count)?;
            let tags = read_tags(r.rest(), body_at + 4, swf_version, true, budget)?;
            Tag::Sprite(Sprite { id, frame_count, tags })
        }
        43 => {
            let name = r.string()?;
            let anchor = if r.at_end() { None } else { Some(r.u8()?) };
            Tag::FrameLabel { name, anchor }
        }
        46 => return Ok(Tag::MorphShape(MorphShape::read_in(body, budget)?)),
        48 | 75 => return Ok(Tag::Font(Font::read_in(body, if code == 48 { 2 } else { 3 }, budget)?)),
        56 => Tag::ExportAssets(assets(&mut r)?),
        57 | 71 => {
            let url = r.string()?;
            let unk_reserved = if code == 71 { Some([r.u8()?, r.u8()?]) } else { None };
            Tag::ImportAssets { url, unk_reserved, assets: assets(&mut r)? }
        }
        59 => {
            let sprite_id = r.u16()?;
            Tag::DoInitAction { sprite_id, actions: actions(r.rest(), budget)? }
        }
        69 => Tag::FileAttributes(r.u32()?),
        74 => return Ok(Tag::CsmTextSettings(CsmTextSettings::read(body)?)),
        77 => Tag::Metadata(r.string()?),
        78 => Tag::ScalingGrid { id: r.u16()?, splitter: Rect::read(&mut r)? },
        1000 => return Ok(Tag::ExporterInfo(ExporterInfo::read(body)?)),
        1008 => return Ok(Tag::SubImage(SubImage::read(body)?)),
        1009 => return Ok(Tag::ExternalImage(ExternalImage::read(body)?)),
        _ => return Ok(Tag::Unknown(body.to_vec())),
    };
    expect_end(&r, tag_name(code).unwrap_or("tag"))?;
    Ok(tag)
}

impl Movie {
    pub fn parse(file: &[u8]) -> Result<Movie> {
        Movie::parse_with(file, &Limits::default())
    }

    pub fn parse_with(file: &[u8], limits: &Limits) -> Result<Movie> {
        Movie::parse_in(file, &mut Budget::new(*limits))
    }

    pub fn parse_in(file: &[u8], budget: &mut Budget) -> Result<Movie> {
        let budget = &*budget;
        let (data, padding) = decompress_with(file, budget.limits())?;
        let unk_inflate_padding = padding.len();
        let mut signature = [0u8; 3];
        signature.copy_from_slice(&data[..3]);
        let version = data[3];
        let file_length = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let mut r = SwfReader::new(&data[8..]);
        let frame_size = Rect::read(&mut r)?;
        let frame_rate = r.u16()?;
        let frame_count = r.u16()?;
        budget.timeline(frame_count)?;
        let at = 8 + r.pos();
        let tags = read_tags(r.rest(), at, version, false, budget)?;
        Ok(Movie { signature, version, file_length, frame_size, frame_rate, frame_count, tags, unk_inflate_padding })
    }

    pub fn all_tags(&self) -> Vec<(usize, &TagRecord)> {
        fn walk<'a>(tags: &'a [TagRecord], depth: usize, out: &mut Vec<(usize, &'a TagRecord)>) {
            for t in tags {
                out.push((depth, t));
                if let Tag::Sprite(s) = &t.tag {
                    walk(&s.tags, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.tags, 0, &mut out);
        out
    }

    pub fn exporter_info(&self) -> Option<&ExporterInfo> {
        self.tags.iter().find_map(|t| match &t.tag {
            Tag::ExporterInfo(e) => Some(e),
            _ => None,
        })
    }
}
