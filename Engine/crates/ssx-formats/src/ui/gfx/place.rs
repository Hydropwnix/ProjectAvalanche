use super::avm1::{self, Action};
use super::bits::SwfReader;
use super::limits::Budget;
use super::types::{ColorTransform, Matrix, Rgba};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct PlaceObject {
    pub version: u8,
    pub flags: u8,
    pub flags3: u8,
    pub depth: u16,
    pub class_name: Option<String>,
    pub character_id: Option<u16>,
    pub matrix: Option<Matrix>,
    pub color_transform: Option<ColorTransform>,
    pub ratio: Option<u16>,
    pub name: Option<String>,
    pub clip_depth: Option<u16>,
    pub filters: Vec<Filter>,
    pub blend_mode: Option<u8>,
    pub cache_as_bitmap: Option<u8>,
    pub visible: Option<u8>,
    pub background: Option<Rgba>,
    pub clip_actions: Option<ClipActions>,
}

impl PlaceObject {
    pub const MOVE: u8 = 0x01;
    pub const HAS_CHARACTER: u8 = 0x02;

    pub fn read(data: &[u8], version: u8, swf_version: u8) -> Result<Self> {
        Self::read_in(data, version, swf_version, &Budget::unlimited())
    }

    pub fn read_in(data: &[u8], version: u8, swf_version: u8, budget: &Budget) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let flags = r.u8()?;
        let flags3 = if version >= 3 { r.u8()? } else { 0 };
        let depth = r.u16()?;
        let class_name =
            if flags3 & 0x08 != 0 || (flags3 & 0x10 != 0 && flags & 0x02 != 0) { Some(r.string()?) } else { None };
        let character_id = if flags & 0x02 != 0 { Some(r.u16()?) } else { None };
        let matrix = if flags & 0x04 != 0 { Some(Matrix::read(&mut r)?) } else { None };
        let color_transform = if flags & 0x08 != 0 { Some(ColorTransform::read(&mut r, true)?) } else { None };
        let ratio = if flags & 0x10 != 0 { Some(r.u16()?) } else { None };
        let name = if flags & 0x20 != 0 { Some(r.string()?) } else { None };
        let clip_depth = if flags & 0x40 != 0 { Some(r.u16()?) } else { None };
        let mut filters = Vec::new();
        if flags3 & 0x01 != 0 {
            let n = r.u8()?;
            for _ in 0..n {
                filters.push(Filter::read(&mut r)?);
            }
        }
        let blend_mode = if flags3 & 0x02 != 0 { Some(r.u8()?) } else { None };
        let cache_as_bitmap = if flags3 & 0x04 != 0 { Some(r.u8()?) } else { None };
        let visible = if flags3 & 0x20 != 0 { Some(r.u8()?) } else { None };
        let background = if flags3 & 0x40 != 0 { Some(Rgba::read_rgba(&mut r)?) } else { None };
        let clip_actions = if flags & 0x80 != 0 { Some(ClipActions::read(&mut r, swf_version, budget)?) } else { None };
        super::expect_end(&r, "PlaceObject")?;
        Ok(PlaceObject {
            version,
            flags,
            flags3,
            depth,
            class_name,
            character_id,
            matrix,
            color_transform,
            ratio,
            name,
            clip_depth,
            filters,
            blend_mode,
            cache_as_bitmap,
            visible,
            background,
            clip_actions,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterBlur {
    pub blur_x: f32,
    pub blur_y: f32,
    pub angle: f32,
    pub distance: f32,
    pub strength: f32,
    pub inner: bool,
    pub knockout: bool,
    pub composite_source: bool,
    pub on_top: bool,
    pub passes: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    DropShadow { color: Rgba, blur: FilterBlur },
    Blur { blur_x: f32, blur_y: f32, passes: u8, unk_reserved: u8 },
    Glow { color: Rgba, blur: FilterBlur },
    Bevel { shadow: Rgba, highlight: Rgba, blur: FilterBlur },
    GradientGlow { colors: Vec<Rgba>, ratios: Vec<u8>, blur: FilterBlur },
    Convolution { width: u8, height: u8, divisor: f32, bias: f32, matrix: Vec<f32>, default: Rgba, flags: u8 },
    ColorMatrix([f32; 20]),
    GradientBevel { colors: Vec<Rgba>, ratios: Vec<u8>, blur: FilterBlur },
}

fn fixed(r: &mut SwfReader) -> Result<f32> {
    Ok(r.i32()? as f32 / 65536.0)
}

fn fixed8(r: &mut SwfReader) -> Result<f32> {
    Ok(r.i16()? as f32 / 256.0)
}

impl Filter {
    fn read(r: &mut SwfReader) -> Result<Self> {
        let id = r.u8()?;
        Ok(match id {
            0 | 2 => {
                let color = Rgba::read_rgba(r)?;
                let (blur_x, blur_y) = (fixed(r)?, fixed(r)?);
                let (angle, distance) = if id == 0 { (fixed(r)?, fixed(r)?) } else { (0.0, 0.0) };
                let strength = fixed8(r)?;
                let bits = r.u8()?;
                let blur = FilterBlur {
                    blur_x,
                    blur_y,
                    angle,
                    distance,
                    strength,
                    inner: bits & 0x80 != 0,
                    knockout: bits & 0x40 != 0,
                    composite_source: bits & 0x20 != 0,
                    on_top: false,
                    passes: bits & 0x1F,
                };
                if id == 0 { Filter::DropShadow { color, blur } } else { Filter::Glow { color, blur } }
            }
            1 => {
                let (blur_x, blur_y) = (fixed(r)?, fixed(r)?);
                let bits = r.u8()?;
                Filter::Blur { blur_x, blur_y, passes: bits >> 3, unk_reserved: bits & 7 }
            }
            3 => {
                let shadow = Rgba::read_rgba(r)?;
                let highlight = Rgba::read_rgba(r)?;
                Filter::Bevel { shadow, highlight, blur: Self::bevel_tail(r)? }
            }
            4 | 7 => {
                let n = usize::from(r.u8()?);
                let colors = (0..n).map(|_| Rgba::read_rgba(r)).collect::<Result<Vec<_>>>()?;
                let ratios = r.bytes(n)?.to_vec();
                let blur = Self::bevel_tail(r)?;
                if id == 4 {
                    Filter::GradientGlow { colors, ratios, blur }
                } else {
                    Filter::GradientBevel { colors, ratios, blur }
                }
            }
            5 => {
                let (width, height) = (r.u8()?, r.u8()?);
                let (divisor, bias) = (r.f32()?, r.f32()?);
                let matrix = (0..usize::from(width) * usize::from(height)).map(|_| r.f32()).collect::<Result<_>>()?;
                Filter::Convolution {
                    width,
                    height,
                    divisor,
                    bias,
                    matrix,
                    default: Rgba::read_rgba(r)?,
                    flags: r.u8()?,
                }
            }
            6 => {
                let mut m = [0f32; 20];
                for v in &mut m {
                    *v = r.f32()?;
                }
                Filter::ColorMatrix(m)
            }
            _ => return Err(Error::invalid(format!("filter id {id}"))),
        })
    }

    fn bevel_tail(r: &mut SwfReader) -> Result<FilterBlur> {
        let (blur_x, blur_y, angle, distance) = (fixed(r)?, fixed(r)?, fixed(r)?, fixed(r)?);
        let strength = fixed8(r)?;
        let bits = r.u8()?;
        Ok(FilterBlur {
            blur_x,
            blur_y,
            angle,
            distance,
            strength,
            inner: bits & 0x80 != 0,
            knockout: bits & 0x40 != 0,
            composite_source: bits & 0x20 != 0,
            on_top: bits & 0x10 != 0,
            passes: bits & 0x0F,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipActions {
    pub all_events: u32,
    pub records: Vec<ClipActionRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipActionRecord {
    pub events: u32,
    pub key_code: Option<u8>,
    pub actions: Vec<Action>,
}

impl ClipActions {
    fn read(r: &mut SwfReader, swf_version: u8, budget: &Budget) -> Result<Self> {
        let reserved = r.u16()?;
        if reserved != 0 {
            return Err(Error::invalid(format!("clip actions reserved word {reserved:#x}")));
        }
        let wide = swf_version >= 6;
        let all_events = if wide { r.u32()? } else { u32::from(r.u16()?) };
        let mut records = Vec::new();
        loop {
            let events = if wide { r.u32()? } else { u32::from(r.u16()?) };
            if events == 0 {
                break;
            }
            let size = r.u32()? as usize;
            let body = r.bytes(size)?;
            let (key_code, code) = if wide && events & 0x0002_0000 != 0 {
                (body.first().copied(), &body[1.min(body.len())..])
            } else {
                (None, body)
            };
            let actions = avm1::decode_in(code, budget)?;
            avm1::validate(&actions, code.len() as u32)?;
            records.push(ClipActionRecord { events, key_code, actions });
        }
        Ok(ClipActions { all_events, records })
    }
}
