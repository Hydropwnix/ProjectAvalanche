use super::bits::SwfReader;
use super::types::{Matrix, Rect, Rgba};
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub spread: u8,
    pub interpolation: u8,
    pub records: Vec<(u8, Rgba)>,
    pub focal: Option<i16>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FillStyle {
    Solid(Rgba),
    Gradient { kind: u8, matrix: Matrix, gradient: Gradient },
    Bitmap { kind: u8, bitmap_id: u16, matrix: Matrix },
}

impl FillStyle {
    fn read(r: &mut SwfReader, version: u8) -> Result<Self> {
        let kind = r.u8()?;
        Ok(match kind {
            0x00 => FillStyle::Solid(Rgba::read(r, version >= 3)?),
            0x10 | 0x12 | 0x13 => {
                let matrix = Matrix::read(r)?;
                let gradient = Gradient::read(r, version, kind == 0x13)?;
                FillStyle::Gradient { kind, matrix, gradient }
            }
            0x40..=0x43 => FillStyle::Bitmap { kind, bitmap_id: r.u16()?, matrix: Matrix::read(r)? },
            _ => return Err(Error::invalid(format!("fill style type {kind:#x}"))),
        })
    }
}

impl Gradient {
    fn read(r: &mut SwfReader, version: u8, focal: bool) -> Result<Self> {
        let spread = r.ub(2)? as u8;
        let interpolation = r.ub(2)? as u8;
        let n = r.ub(4)?;
        let mut records = Vec::with_capacity(n as usize);
        for _ in 0..n {
            records.push((r.u8()?, Rgba::read(r, version >= 3)?));
        }
        let focal = if focal { Some(r.i16()?) } else { None };
        Ok(Gradient { spread, interpolation, records, focal })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineStyle {
    pub width: u16,
    pub color: Rgba,
    pub start_cap: u8,
    pub join: u8,
    pub no_h_scale: bool,
    pub no_v_scale: bool,
    pub pixel_hinting: bool,
    pub unk_reserved: u8,
    pub no_close: bool,
    pub end_cap: u8,
    pub miter_limit: Option<u16>,
    pub fill: Option<FillStyle>,
}

impl LineStyle {
    fn read(r: &mut SwfReader, version: u8) -> Result<Self> {
        let width = r.u16()?;
        if version < 4 {
            return Ok(LineStyle {
                width,
                color: Rgba::read(r, version >= 3)?,
                start_cap: 0,
                join: 0,
                no_h_scale: false,
                no_v_scale: false,
                pixel_hinting: false,
                unk_reserved: 0,
                no_close: false,
                end_cap: 0,
                miter_limit: None,
                fill: None,
            });
        }
        let start_cap = r.ub(2)? as u8;
        let join = r.ub(2)? as u8;
        let has_fill = r.flag()?;
        let no_h_scale = r.flag()?;
        let no_v_scale = r.flag()?;
        let pixel_hinting = r.flag()?;
        let unk_reserved = r.ub(5)? as u8;
        let no_close = r.flag()?;
        let end_cap = r.ub(2)? as u8;
        let miter_limit = if join == 2 { Some(r.u16()?) } else { None };
        let (color, fill) =
            if has_fill { (Rgba::default(), Some(FillStyle::read(r, 4)?)) } else { (Rgba::read_rgba(r)?, None) };
        Ok(LineStyle {
            width,
            color,
            start_cap,
            join,
            no_h_scale,
            no_v_scale,
            pixel_hinting,
            unk_reserved,
            no_close,
            end_cap,
            miter_limit,
            fill,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Styles {
    pub fills: Vec<FillStyle>,
    pub lines: Vec<LineStyle>,
}

fn style_count(r: &mut SwfReader, extended: bool) -> Result<usize> {
    let n = r.u8()?;
    Ok(if n == 0xFF && extended { usize::from(r.u16()?) } else { usize::from(n) })
}

impl Styles {
    fn read(r: &mut SwfReader, version: u8) -> Result<Self> {
        let nf = style_count(r, version >= 2)?;
        let fills = (0..nf).map(|_| FillStyle::read(r, version)).collect::<Result<_>>()?;
        let nl = style_count(r, version >= 2)?;
        let lines = (0..nl).map(|_| LineStyle::read(r, version)).collect::<Result<_>>()?;
        Ok(Styles { fills, lines })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShapeRecord {
    StyleChange {
        move_to: Option<(i32, i32)>,
        fill0: Option<u32>,
        fill1: Option<u32>,
        line: Option<u32>,
        new_styles: Option<Styles>,
    },
    Straight {
        dx: i32,
        dy: i32,
    },
    Curved {
        control_dx: i32,
        control_dy: i32,
        anchor_dx: i32,
        anchor_dy: i32,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    pub styles: Styles,
    pub records: Vec<ShapeRecord>,
}

fn read_records(
    r: &mut SwfReader,
    version: u8,
    mut fills: usize,
    mut lines: usize,
    style_version: u8,
) -> Result<Vec<ShapeRecord>> {
    let mut fill_bits = r.ub(4)?;
    let mut line_bits = r.ub(4)?;
    let mut out = Vec::new();
    let check = |v: u32, n: usize, what: &str| {
        if v as usize > n { Err(Error::invalid(format!("{what} style {v} of {n}"))) } else { Ok(v) }
    };
    loop {
        if r.ub(1)? == 0 {
            let flags = r.ub(5)?;
            if flags == 0 {
                break;
            }
            let move_to = if flags & 1 != 0 {
                let n = r.ub(5)?;
                Some((r.sb(n)?, r.sb(n)?))
            } else {
                None
            };
            let fill0 = if flags & 2 != 0 { Some(r.ub(fill_bits)?) } else { None };
            let fill1 = if flags & 4 != 0 { Some(r.ub(fill_bits)?) } else { None };
            let line = if flags & 8 != 0 { Some(r.ub(line_bits)?) } else { None };
            let new_styles = if flags & 0x10 != 0 {
                if version < 2 {
                    return Err(Error::invalid("new styles in a version-1 shape"));
                }
                let s = Styles::read(r, style_version)?;
                fills = s.fills.len();
                lines = s.lines.len();
                fill_bits = r.ub(4)?;
                line_bits = r.ub(4)?;
                Some(s)
            } else {
                None
            };
            for f in [fill0, fill1].into_iter().flatten() {
                check(f, fills, "fill")?;
            }
            if let Some(l) = line {
                check(l, lines, "line")?;
            }
            out.push(ShapeRecord::StyleChange { move_to, fill0, fill1, line, new_styles });
        } else if r.flag()? {
            let n = r.ub(4)? + 2;
            let (dx, dy) = if r.flag()? {
                (r.sb(n)?, r.sb(n)?)
            } else if r.flag()? {
                (0, r.sb(n)?)
            } else {
                (r.sb(n)?, 0)
            };
            out.push(ShapeRecord::Straight { dx, dy });
        } else {
            let n = r.ub(4)? + 2;
            out.push(ShapeRecord::Curved {
                control_dx: r.sb(n)?,
                control_dy: r.sb(n)?,
                anchor_dx: r.sb(n)?,
                anchor_dy: r.sb(n)?,
            });
        }
    }
    r.align();
    Ok(out)
}

impl Shape {
    pub fn read_with_style(r: &mut SwfReader, version: u8) -> Result<Self> {
        let styles = Styles::read(r, version)?;
        let records = read_records(r, version, styles.fills.len(), styles.lines.len(), version)?;
        Ok(Shape { styles, records })
    }

    pub fn read_glyph(r: &mut SwfReader) -> Result<Self> {
        Ok(Shape { styles: Styles::default(), records: read_records(r, 1, 1, 0, 1)? })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefineShape {
    pub version: u8,
    pub id: u16,
    pub bounds: Rect,
    pub edge_bounds: Option<Rect>,
    pub unk_reserved: u8,
    pub uses_fill_winding_rule: bool,
    pub uses_non_scaling_strokes: bool,
    pub uses_scaling_strokes: bool,
    pub shape: Shape,
}

impl DefineShape {
    pub fn read(data: &[u8], version: u8) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let id = r.u16()?;
        let bounds = Rect::read(&mut r)?;
        let (mut edge_bounds, mut unk_reserved, mut winding, mut non_scaling, mut scaling) =
            (None, 0, false, false, false);
        if version == 4 {
            edge_bounds = Some(Rect::read(&mut r)?);
            unk_reserved = r.ub(5)? as u8;
            winding = r.flag()?;
            non_scaling = r.flag()?;
            scaling = r.flag()?;
        }
        let shape = Shape::read_with_style(&mut r, version)?;
        super::expect_end(&r, "DefineShape")?;
        Ok(DefineShape {
            version,
            id,
            bounds,
            edge_bounds,
            unk_reserved,
            uses_fill_winding_rule: winding,
            uses_non_scaling_strokes: non_scaling,
            uses_scaling_strokes: scaling,
            shape,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MorphFill {
    pub start: FillStyle,
    pub end: FillStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MorphLine {
    pub start_width: u16,
    pub end_width: u16,
    pub start_color: Rgba,
    pub end_color: Rgba,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MorphShape {
    pub id: u16,
    pub start_bounds: Rect,
    pub end_bounds: Rect,
    pub fills: Vec<MorphFill>,
    pub lines: Vec<MorphLine>,
    pub start_edges: Vec<ShapeRecord>,
    pub end_edges: Vec<ShapeRecord>,
}

fn morph_gradient(r: &mut SwfReader) -> Result<(Gradient, Gradient)> {
    let n = r.u8()?;
    if n == 0 || n > 15 {
        return Err(Error::invalid(format!("morph gradient with {n} records")));
    }
    let mut s = Gradient { spread: 0, interpolation: 0, records: Vec::new(), focal: None };
    let mut e = s.clone();
    for _ in 0..n {
        s.records.push((r.u8()?, Rgba::read_rgba(r)?));
        e.records.push((r.u8()?, Rgba::read_rgba(r)?));
    }
    Ok((s, e))
}

impl MorphShape {
    pub fn read(data: &[u8]) -> Result<Self> {
        let mut r = SwfReader::new(data);
        let id = r.u16()?;
        let start_bounds = Rect::read(&mut r)?;
        let end_bounds = Rect::read(&mut r)?;
        let offset = r.u32()? as usize;
        let end_at = r.pos() + offset;
        let nf = style_count(&mut r, true)?;
        let mut fills = Vec::with_capacity(nf);
        for _ in 0..nf {
            let kind = r.u8()?;
            fills.push(match kind {
                0x00 => MorphFill {
                    start: FillStyle::Solid(Rgba::read_rgba(&mut r)?),
                    end: FillStyle::Solid(Rgba::read_rgba(&mut r)?),
                },
                0x10 | 0x12 => {
                    let (m0, m1) = (Matrix::read(&mut r)?, Matrix::read(&mut r)?);
                    let (g0, g1) = morph_gradient(&mut r)?;
                    MorphFill {
                        start: FillStyle::Gradient { kind, matrix: m0, gradient: g0 },
                        end: FillStyle::Gradient { kind, matrix: m1, gradient: g1 },
                    }
                }
                0x40..=0x43 => {
                    let bitmap_id = r.u16()?;
                    let (m0, m1) = (Matrix::read(&mut r)?, Matrix::read(&mut r)?);
                    MorphFill {
                        start: FillStyle::Bitmap { kind, bitmap_id, matrix: m0 },
                        end: FillStyle::Bitmap { kind, bitmap_id, matrix: m1 },
                    }
                }
                _ => return Err(Error::invalid(format!("morph fill type {kind:#x}"))),
            });
        }
        let nl = style_count(&mut r, true)?;
        let mut lines = Vec::with_capacity(nl);
        for _ in 0..nl {
            lines.push(MorphLine {
                start_width: r.u16()?,
                end_width: r.u16()?,
                start_color: Rgba::read_rgba(&mut r)?,
                end_color: Rgba::read_rgba(&mut r)?,
            });
        }
        let start_edges = read_records(&mut r, 1, nf, nl, 3)?;
        if r.pos() != end_at {
            return Err(Error::invalid(format!("morph end edges at {:#x}, offset says {end_at:#x}", r.pos())));
        }
        let end_edges = read_records(&mut r, 1, nf, nl, 3)?;
        super::expect_end(&r, "DefineMorphShape")?;
        Ok(MorphShape { id, start_bounds, end_bounds, fills, lines, start_edges, end_edges })
    }
}
