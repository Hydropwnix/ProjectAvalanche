use super::bits::SwfReader;
use super::limits::Budget;
use super::shape::Shape;
use super::types::{Matrix, Rect, Rgba};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct EditText {
    pub id: u16,
    pub bounds: Rect,
    pub flags1: u8,
    pub flags2: u8,
    pub font_id: Option<u16>,
    pub font_class: Option<String>,
    pub font_height: Option<u16>,
    pub color: Option<Rgba>,
    pub max_length: Option<u16>,
    pub layout: Option<TextLayout>,
    pub variable_name: String,
    pub initial_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextLayout {
    pub align: u8,
    pub left_margin: u16,
    pub right_margin: u16,
    pub indent: u16,
    pub leading: i16,
}

impl EditText {
    pub const WORD_WRAP: u8 = 0x40;
    pub const MULTILINE: u8 = 0x20;
    pub const HTML: u8 = 0x02;
    pub const USE_OUTLINES: u8 = 0x01;

    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let id = r.u16()?;
        let bounds = Rect::read(&mut r)?;
        let flags1 = r.u8()?;
        let flags2 = r.u8()?;
        let font_id = if flags1 & 0x01 != 0 { Some(r.u16()?) } else { None };
        let font_class = if flags2 & 0x80 != 0 { Some(r.string()?) } else { None };
        let font_height = if flags1 & 0x01 != 0 || flags2 & 0x80 != 0 { Some(r.u16()?) } else { None };
        let color = if flags1 & 0x04 != 0 { Some(Rgba::read_rgba(&mut r)?) } else { None };
        let max_length = if flags1 & 0x02 != 0 { Some(r.u16()?) } else { None };
        let layout = if flags2 & 0x20 != 0 {
            Some(TextLayout {
                align: r.u8()?,
                left_margin: r.u16()?,
                right_margin: r.u16()?,
                indent: r.u16()?,
                leading: r.i16()?,
            })
        } else {
            None
        };
        let variable_name = r.string()?;
        let initial_text = if flags1 & 0x80 != 0 { Some(r.string()?) } else { None };
        super::expect_end(&r, "DefineEditText")?;
        Ok(EditText {
            id,
            bounds,
            flags1,
            flags2,
            font_id,
            font_class,
            font_height,
            color,
            max_length,
            layout,
            variable_name,
            initial_text,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextRecord {
    pub font_id: Option<u16>,
    pub color: Option<Rgba>,
    pub x_offset: Option<i16>,
    pub y_offset: Option<i16>,
    pub height: Option<u16>,
    pub glyphs: Vec<(u32, i32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticText {
    pub id: u16,
    pub bounds: Rect,
    pub matrix: Matrix,
    pub records: Vec<TextRecord>,
}

impl StaticText {
    pub fn read(data: &[u8], rgba: bool) -> Result<Self> {
        Self::read_in(data, rgba, &Budget::unlimited())
    }

    pub fn read_in(data: &[u8], rgba: bool, budget: &Budget) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let id = r.u16()?;
        let bounds = Rect::read(&mut r)?;
        let matrix = Matrix::read(&mut r)?;
        let glyph_bits = u32::from(r.u8()?);
        let advance_bits = u32::from(r.u8()?);
        let mut records = Vec::new();
        loop {
            let flags = r.u8()?;
            if flags == 0 {
                break;
            }
            if flags & 0xF0 != 0x80 {
                return Err(Error::invalid(format!("text record flags {flags:#x}")));
            }
            let font_id = if flags & 0x08 != 0 { Some(r.u16()?) } else { None };
            let color = if flags & 0x04 != 0 { Some(Rgba::read(&mut r, rgba)?) } else { None };
            let x_offset = if flags & 0x01 != 0 { Some(r.i16()?) } else { None };
            let y_offset = if flags & 0x02 != 0 { Some(r.i16()?) } else { None };
            let height = if flags & 0x08 != 0 { Some(r.u16()?) } else { None };
            let n = r.u8()?;
            budget.text_glyphs(u64::from(n))?;
            let mut glyphs = Vec::with_capacity(usize::from(n));
            for _ in 0..n {
                glyphs.push((r.ub(glyph_bits)?, r.sb(advance_bits)?));
            }
            r.align();
            records.push(TextRecord { font_id, color, x_offset, y_offset, height, glyphs });
        }
        super::expect_end(&r, "DefineText")?;
        Ok(StaticText { id, bounds, matrix, records })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub version: u8,
    pub id: u16,
    pub flags: u8,
    pub language: u8,
    pub name: String,
    pub glyphs: Vec<Shape>,
    pub codes: Vec<u16>,
    pub layout: Option<FontLayout>,
    pub unk_trailing_layout: Option<FontLayout>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FontLayout {
    pub ascent: u16,
    pub descent: u16,
    pub leading: i16,
    pub advances: Vec<i16>,
    pub bounds: Vec<Rect>,
    pub kerning: Vec<(u16, u16, i16)>,
}

impl Font {
    pub const HAS_LAYOUT: u8 = 0x80;
    pub const WIDE_OFFSETS: u8 = 0x08;
    pub const WIDE_CODES: u8 = 0x04;

    pub fn read(data: &[u8], version: u8) -> Result<Self> {
        Self::read_in(data, version, &Budget::unlimited())
    }

    pub fn read_in(data: &[u8], version: u8, budget: &Budget) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let id = r.u16()?;
        let flags = r.u8()?;
        let language = r.u8()?;
        let name_len = usize::from(r.u8()?);
        let raw = r.bytes(name_len)?;
        let name = String::from_utf8_lossy(raw.strip_suffix(&[0]).unwrap_or(raw)).into_owned();
        let n = usize::from(r.u16()?);
        let wide = flags & Self::WIDE_OFFSETS != 0;
        let table_at = r.pos();
        let mut offsets = Vec::with_capacity((n + 1).min(r.remaining() / 2));
        if n > 0 || !r.at_end() {
            for _ in 0..=n {
                offsets.push(if wide { r.u32()? as usize } else { usize::from(r.u16()?) });
            }
        }
        let mut glyphs = Vec::with_capacity(n.min(offsets.len()));
        for (i, &off) in offsets.iter().take(n).enumerate() {
            if table_at + off != r.pos() {
                return Err(Error::invalid(format!("font {id} glyph {i} at {:#x}, table says {off:#x}", r.pos())));
            }
            glyphs.push(Shape::read_glyph_in(&mut r, budget)?);
        }
        if let Some(&off) = offsets.get(n)
            && table_at + off != r.pos()
        {
            return Err(Error::invalid(format!("font {id} code table at {:#x}", r.pos())));
        }
        let codes = (0..n)
            .map(|_| if flags & Self::WIDE_CODES != 0 { r.u16() } else { r.u8().map(u16::from) })
            .collect::<Result<Vec<_>>>()?;
        let (mut layout, mut unk_trailing_layout) = (None, None);
        if flags & Self::HAS_LAYOUT != 0 {
            layout = Some(FontLayout::read(&mut r, n, flags)?);
        } else if n == 0 && !r.at_end() {
            unk_trailing_layout = Some(FontLayout::read(&mut r, 0, flags)?);
        }
        super::expect_end(&r, "DefineFont")?;
        Ok(Font { version, id, flags, language, name, glyphs, codes, layout, unk_trailing_layout })
    }
}

impl FontLayout {
    fn read(r: &mut SwfReader, n: usize, flags: u8) -> Result<Self> {
        let ascent = r.u16()?;
        let descent = r.u16()?;
        let leading = r.i16()?;
        let advances = (0..n).map(|_| r.i16()).collect::<Result<Vec<_>>>()?;
        let bounds = (0..n).map(|_| Rect::read(r)).collect::<Result<Vec<_>>>()?;
        let nk = r.u16()?;
        let mut kerning = Vec::with_capacity(usize::from(nk).min(r.remaining() / 4));
        for _ in 0..nk {
            let (a, b) = if flags & Font::WIDE_CODES != 0 {
                (r.u16()?, r.u16()?)
            } else {
                (u16::from(r.u8()?), u16::from(r.u8()?))
            };
            kerning.push((a, b, r.i16()?));
        }
        kerning.sort_by_key(|k| (k.0, k.1));
        Ok(FontLayout { ascent, descent, leading, advances, bounds, kerning })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CsmTextSettings {
    pub text_id: u16,
    pub use_flash_type: u8,
    pub grid_fit: u8,
    pub unk_reserved: u8,
    pub thickness: f32,
    pub sharpness: f32,
    pub unk_reserved2: u8,
}

impl CsmTextSettings {
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let text_id = r.u16()?;
        let use_flash_type = r.ub(2)? as u8;
        let grid_fit = r.ub(3)? as u8;
        let unk_reserved = r.ub(3)? as u8;
        let s = CsmTextSettings {
            text_id,
            use_flash_type,
            grid_fit,
            unk_reserved,
            thickness: r.f32()?,
            sharpness: r.f32()?,
            unk_reserved2: r.u8()?,
        };
        super::expect_end(&r, "CSMTextSettings")?;
        Ok(s)
    }
}
