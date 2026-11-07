use super::Limits;
use super::bits::Msb;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub record: [u8; 4],
    pub sequence: SequenceHeader,
    pub sequence_payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SequenceHeader {
    pub profile: u8,
    pub level: u8,
    pub max_width: u32,
    pub max_height: u32,
    pub bit_depth: u8,
    pub mono: bool,
    pub color: ColorConfig,
    pub film_grain: bool,
    pub timing_fps: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorConfig {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
    pub chroma_position: u8,
}

pub const OBU_SEQUENCE_HEADER: u8 = 1;
pub const OBU_TEMPORAL_DELIMITER: u8 = 2;
pub const OBU_FRAME_HEADER: u8 = 3;
pub const OBU_TILE_GROUP: u8 = 4;
pub const OBU_METADATA: u8 = 5;
pub const OBU_FRAME: u8 = 6;
pub const OBU_REDUNDANT_FRAME_HEADER: u8 = 7;
pub const OBU_TILE_LIST: u8 = 8;
pub const OBU_PADDING: u8 = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Obu {
    pub kind: u8,
    pub spatial_id: u8,
    pub start: usize,
    pub payload: (usize, usize),
}

pub fn obus(data: &[u8], max: usize) -> Result<Vec<Obu>, String> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        if out.len() >= max {
            return Err(format!("more than {max} OBUs in one unit"));
        }
        let start = pos;
        let h = data[pos];
        if h & 0x81 != 0 {
            return Err(format!("OBU {}: the forbidden or reserved header bit is set", out.len()));
        }
        let kind = (h >> 3) & 0xF;
        let (ext, has_size) = (h & 0x04 != 0, h & 0x02 != 0);
        pos += 1;
        let mut spatial_id = 0;
        if ext {
            let e = *data.get(pos).ok_or("an OBU's extension header is cut short")?;
            if e & 0x07 != 0 {
                return Err("an OBU extension's reserved bits are set".into());
            }
            spatial_id = (e >> 3) & 0x3;
            pos += 1;
        }
        let size = if has_size {
            let (v, n) = leb128(&data[pos..])?;
            pos += n;
            usize::try_from(v).map_err(|_| "an OBU size larger than memory")?
        } else {
            data.len() - pos
        };
        let end = pos.checked_add(size).filter(|&e| e <= data.len()).ok_or_else(|| {
            format!("OBU {} of {size} bytes reaches past its unit's end ({} bytes left)", out.len(), data.len() - pos)
        })?;
        if !(1..=8).contains(&kind) && kind != OBU_PADDING {
            return Err(format!("OBU {} of the reserved type {kind}", out.len()));
        }
        out.push(Obu { kind, spatial_id, start, payload: (pos, end) });
        pos = end;
    }
    Ok(out)
}

fn leb128(b: &[u8]) -> Result<(u64, usize), String> {
    let mut v = 0u64;
    for i in 0..8 {
        let byte = *b.get(i).ok_or("an OBU size is cut short")?;
        v |= u64::from(byte & 0x7F) << (7 * i);
        if byte & 0x80 == 0 {
            if v > u64::from(u32::MAX) {
                return Err("an OBU size of 2³² bytes or more".into());
            }
            return Ok((v, i + 1));
        }
    }
    Err("an OBU size of more than 8 bytes".into())
}

impl Config {
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<Config, String> {
        let record: [u8; 4] =
            bytes.get(..4).and_then(|r| r.try_into().ok()).ok_or("a configuration record of fewer than 4 bytes")?;
        if record[0] != 0x81 {
            return Err(format!("configuration record marker / version 0x{:02x} (0x81)", record[0]));
        }
        let mut sequence = None;
        for o in obus(&bytes[4..], 16)? {
            let payload = &bytes[4 + o.payload.0..4 + o.payload.1];
            match o.kind {
                OBU_SEQUENCE_HEADER if sequence.is_none() => sequence = Some(payload.to_vec()),
                OBU_SEQUENCE_HEADER => return Err("two sequence headers in the configuration".into()),
                OBU_METADATA => {}
                k => {
                    return Err(format!(
                        "an OBU of type {k} in the configuration (only the sequence header and metadata)"
                    ));
                }
            }
        }
        let sequence_payload = sequence.ok_or("the configuration holds no sequence header")?;
        let sequence = SequenceHeader::parse(&sequence_payload, limits)?;
        let want = (sequence.profile << 5) | sequence.level;
        let flags = record[2] & 0x7C;
        let want_flags = if sequence.mono { 0x10 | 0x0C } else { 0x0C };
        if record[1] != want || flags != want_flags || record[2] & 0x40 != 0 {
            return Err("the configuration record does not match its sequence header".into());
        }
        Ok(Config { record, sequence, sequence_payload })
    }
}

impl SequenceHeader {
    pub fn parse(payload: &[u8], limits: &Limits) -> Result<SequenceHeader, String> {
        let mut b = Msb::new(payload);
        let profile = b.f(3)? as u8;
        let still = b.flag()?;
        let reduced = b.flag()?;
        if profile != 0 {
            return Err(format!("profile {profile} (a movie is the Main profile, 0: 8-bit 4:2:0)"));
        }
        if still || reduced {
            return Err("a still picture's sequence header (a movie has moving pictures)".into());
        }
        let mut timing_fps = None;
        let mut delay_bits = None;
        if b.flag()? {
            let units = b.f(32)?;
            let scale = b.f(32)?;
            if units == 0 || scale == 0 {
                return Err("timing info with a zero tick or time scale".into());
            }
            if b.flag()? {
                let ticks = b.uvlc()?;
                if ticks == u32::MAX {
                    return Err("timing info of 2³² ticks a picture".into());
                }
                timing_fps = Some(f64::from(scale) / (f64::from(units) * (f64::from(ticks) + 1.0)));
            }
            if b.flag()? {
                delay_bits = Some(b.f(5)? + 1);
                b.f(32)?;
                b.f(10)?;
            }
        }
        let initial_delay = b.flag()?;
        let points = b.f(5)? + 1;
        if points != 1 {
            return Err(format!("{points} operating points (scalable streams are refused; a movie has one)"));
        }
        b.f(12)?;
        let level = b.f(5)? as u8;
        if level > 7 {
            b.f(1)?;
        }
        if let Some(n) = delay_bits
            && b.flag()?
        {
            b.f(n)?;
            b.f(n)?;
            b.f(1)?;
        }
        if initial_delay && b.flag()? {
            b.f(4)?;
        }
        let wbits = b.f(4)? + 1;
        let hbits = b.f(4)? + 1;
        let max_width = b.f(wbits)?.saturating_add(1);
        let max_height = b.f(hbits)?.saturating_add(1);
        if max_width > limits.max_width || max_height > limits.max_height {
            return Err(format!(
                "pictures of up to {max_width} × {max_height} (at most {} × {})",
                limits.max_width, limits.max_height
            ));
        }
        if b.flag()? {
            b.f(4)?;
            b.f(3)?;
        }
        b.f(7)?;
        let order_hint = b.flag()?;
        if order_hint {
            b.f(2)?;
        }
        let force_screen = if b.flag()? { 2 } else { b.f(1)? };
        if force_screen > 0 && !b.flag()? {
            b.f(1)?;
        }
        if order_hint {
            b.f(3)?;
        }
        b.f(3)?;
        let (bit_depth, mono, color) = color_config(&mut b)?;
        let film_grain = b.flag()?;
        if b.f(1)? != 1 || (!b.bits_read().is_multiple_of(8) && b.f(8 - (b.bits_read() % 8) as u32)? != 0) {
            return Err("the sequence header's trailing bits are not 1 then zeros".into());
        }
        if payload[b.bits_read() / 8..].iter().any(|&x| x != 0) {
            return Err("bytes after the sequence header's trailing bits".into());
        }
        if bit_depth != 8 {
            return Err(format!("{bit_depth}-bit pictures (a movie is 8-bit)"));
        }
        if film_grain {
            return Err("film grain (the port does not synthesise AV1 film grain; encode without it)".into());
        }
        let min_fps = 1e9 / limits.min_frame_ns as f64;
        if let Some(fps) = timing_fps
            && fps > min_fps + 1e-6
        {
            return Err(format!("{fps:.0} pictures a second in its timing info (at most {min_fps:.0})"));
        }
        Ok(SequenceHeader { profile, level, max_width, max_height, bit_depth, mono, color, film_grain, timing_fps })
    }
}

fn color_config(b: &mut Msb) -> Result<(u8, bool, ColorConfig), String> {
    let bit_depth = if b.flag()? { 10 } else { 8 };
    let mono = b.flag()?;
    let (primaries, transfer, matrix) =
        if b.flag()? { (b.f(8)? as u8, b.f(8)? as u8, b.f(8)? as u8) } else { (2, 2, 2) };
    if mono {
        let full_range = b.flag()?;
        return Ok((bit_depth, true, ColorConfig { primaries, transfer, matrix, full_range, chroma_position: 0 }));
    }
    if primaries == 1 && transfer == 13 && matrix == 0 {
        return Err("RGB-coded pictures (sRGB with the identity matrix: 4:4:4, not the Main profile's 4:2:0)".into());
    }
    if ![1, 2, 4, 5, 6, 7, 9].contains(&matrix) {
        return Err(format!(
            "matrix coefficients {matrix} (BT.709, BT.601, BT.2020 NCL, FCC, SMPTE 240 or unspecified)"
        ));
    }
    let full_range = b.flag()?;
    let chroma_position = b.f(2)? as u8;
    b.f(1)?;
    Ok((bit_depth, false, ColorConfig { primaries, transfer, matrix, full_range, chroma_position }))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnitInfo {
    pub obus: usize,
    pub frames: usize,
}

pub fn check_temporal_unit(data: &[u8], config: &Config, limits: &Limits) -> Result<UnitInfo, String> {
    let list = obus(data, limits.max_obus)?;
    let mut frames = 0;
    for o in &list {
        let payload = &data[o.payload.0..o.payload.1];
        match o.kind {
            OBU_SEQUENCE_HEADER if payload != config.sequence_payload.as_slice() => {
                return Err("a sequence header other than the track's (a movie keeps one)".into());
            }
            OBU_TILE_LIST => return Err("a tile list (large-scale tile decoding is not a movie's)".into()),
            OBU_FRAME | OBU_FRAME_HEADER => frames += 1,
            _ => {}
        }
        if o.spatial_id != 0 {
            return Err(format!("an OBU of spatial layer {} (a movie has one layer)", o.spatial_id));
        }
    }
    if frames == 0 {
        return Err("a temporal unit without a frame".into());
    }
    Ok(UnitInfo { obus: list.len(), frames })
}
