use std::collections::BTreeMap;

use crate::id::{ContentId, Kind};
use crate::material::Problem;
use crate::texture::{self, TextureSettings};

pub const CUBE_N: usize = 32;

pub const LAYER_KEY: &str = "layer";
pub const BLENDS_KEY: &str = "colour_cubes";
pub const CUBE_KEY: &str = "cube";

#[derive(Clone, Debug, PartialEq)]
pub struct Blend {
    pub stack: u32,
    pub mode: u32,
    pub group: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostFxRecord {
    pub floats: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, Vec<f32>>,
    pub layer: Option<String>,
    pub colour_cubes: Vec<Blend>,
    pub cube: Option<ContentId>,
}

fn finite(x: f64) -> bool {
    x.is_finite() && x.abs() <= f64::from(f32::MAX)
}

fn number(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Float(x) if finite(*x) => Some(*x),
        toml::Value::Integer(i) if i.unsigned_abs() < (1 << 24) => Some(*i as f64),
        _ => None,
    }
}

fn short_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && s.chars().all(|c| c.is_ascii_graphic())
}

impl PostFxRecord {
    pub fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        for (k, v) in &self.floats {
            t.insert(k.clone(), toml::Value::Float(f64::from(*v)));
        }
        for (k, v) in &self.vectors {
            t.insert(k.clone(), toml::Value::Array(v.iter().map(|x| toml::Value::Float(f64::from(*x))).collect()));
        }
        if let Some(l) = &self.layer {
            t.insert(LAYER_KEY.into(), toml::Value::String(l.clone()));
        }
        if !self.colour_cubes.is_empty() {
            let list = self
                .colour_cubes
                .iter()
                .map(|b| {
                    let mut e = toml::Table::new();
                    e.insert("stack".into(), toml::Value::Integer(i64::from(b.stack)));
                    e.insert("mode".into(), toml::Value::Integer(i64::from(b.mode)));
                    e.insert("group".into(), toml::Value::String(b.group.clone()));
                    e.insert("weight".into(), toml::Value::Float(f64::from(b.weight)));
                    toml::Value::Table(e)
                })
                .collect();
            t.insert(BLENDS_KEY.into(), toml::Value::Array(list));
        }
        if let Some(c) = &self.cube {
            t.insert(CUBE_KEY.into(), toml::Value::String(c.to_string()));
        }
        t
    }

    pub fn from_table(t: &toml::Table) -> Result<PostFxRecord, Vec<String>> {
        let mut r = PostFxRecord::default();
        let mut problems = Vec::new();
        for (k, v) in t {
            if let Err(e) = r.set(k, v) {
                problems.push(e);
            }
        }
        if problems.is_empty() { Ok(r) } else { Err(problems) }
    }

    fn set(&mut self, key: &str, v: &toml::Value) -> Result<(), String> {
        match key {
            LAYER_KEY => {
                let s = v.as_str().filter(|s| short_name(s)).ok_or("layer: a postfx_layers name, 1 to 63 bytes")?;
                self.layer = Some(s.to_string());
            }
            CUBE_KEY => {
                let s = v.as_str().ok_or("cube: a postfx id of a .png strip")?;
                let id: ContentId = s.parse().map_err(|e| format!("cube: {e}"))?;
                if id.kind() != Kind::Postfx {
                    return Err(format!("cube: {id} is not a postfx id"));
                }
                self.cube = Some(id);
            }
            BLENDS_KEY => {
                let list = v.as_array().ok_or("colour_cubes: an array of { stack, mode, group, weight }")?;
                if list.len() > 64 {
                    return Err(format!("colour_cubes: {} entries (at most 64)", list.len()));
                }
                let mut out = Vec::with_capacity(list.len());
                for (i, e) in list.iter().enumerate() {
                    let t = e.as_table().ok_or_else(|| format!("colour_cubes[{i}]: a table"))?;
                    if let Some(k) = t.keys().find(|k| !matches!(k.as_str(), "stack" | "mode" | "group" | "weight")) {
                        return Err(format!("colour_cubes[{i}]: unknown key {}", ssx_core::text::quoted(k)));
                    }
                    let int = |k: &str, max: i64| {
                        t.get(k).and_then(toml::Value::as_integer).filter(|v| (0..=max).contains(v)).map(|v| v as u32)
                    };
                    let stack = int("stack", 255).ok_or_else(|| format!("colour_cubes[{i}]: stack 0 to 255"))?;
                    let mode = int("mode", 2).ok_or_else(|| format!("colour_cubes[{i}]: mode 0, 1 or 2"))?;
                    let group = t
                        .get("group")
                        .and_then(toml::Value::as_str)
                        .filter(|s| short_name(s))
                        .ok_or_else(|| format!("colour_cubes[{i}]: group, a name of 1 to 63 bytes"))?;
                    let weight = t
                        .get("weight")
                        .and_then(number)
                        .filter(|w| (0.0..=1.0).contains(w))
                        .ok_or_else(|| format!("colour_cubes[{i}]: weight from 0 to 1"))?;
                    out.push(Blend { stack, mode, group: group.to_string(), weight: weight as f32 });
                }
                self.colour_cubes = out;
            }
            _ if !crate::lighting::field_name(key) => {
                return Err(format!(
                    "{}: not a field name (an unnamed field as \"0x<8 hex digits>\")",
                    ssx_core::text::quoted(key)
                ));
            }
            _ => match v {
                toml::Value::Array(a) => {
                    let xs: Option<Vec<f64>> = a.iter().map(number).collect();
                    match xs.filter(|x| (2..=4).contains(&x.len())) {
                        Some(x) => {
                            self.vectors.insert(key.into(), x.into_iter().map(|v| v as f32).collect());
                        }
                        None => return Err(format!("{key}: a vector is 2 to 4 finite numbers")),
                    }
                }
                other => match number(other) {
                    Some(x) => {
                        self.floats.insert(key.into(), x as f32);
                    }
                    None => return Err(format!("{key}: a finite number or 2 to 4 of them")),
                },
            },
        }
        Ok(())
    }
}

pub fn parse_toml(bytes: &[u8]) -> Result<PostFxRecord, Vec<Problem>> {
    let table = crate::package::parse_record_at(bytes).map_err(|p| vec![p])?;
    let lines = crate::lighting::key_lines(bytes);
    let mut r = PostFxRecord::default();
    let mut problems = Vec::new();
    for (k, v) in &table {
        if let Err(e) = r.set(k, v) {
            let (line, col) = lines.get(k).copied().unwrap_or((0, 0));
            problems.push((line, col, e));
        }
    }
    if problems.is_empty() {
        Ok(r)
    } else {
        problems.sort();
        Err(problems)
    }
}

pub fn cube_from_strip(name: &str, bytes: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    let limits = texture::Limits { max_side: 1024, max_decoded_bytes: 8 << 20, ..texture::Limits::default() };
    if !name.to_ascii_lowercase().ends_with(".png") {
        return Err(format!("{name}: a colour cube is a .png strip"));
    }
    let t = texture::decode_file(name, bytes, &TextureSettings::default(), &limits).map_err(|e| e.to_string())?;
    let l = t.levels.first().ok_or_else(|| format!("{name}: no image"))?;
    let (w, h) = (CUBE_N * CUBE_N, CUBE_N);
    if (l.width as usize, l.height as usize) != (w, h) {
        return Err(format!("{name}: {}×{}; a colour cube's strip is {w}×{h} (32 slices of 32×32)", l.width, l.height));
    }
    let img = &l.images[0];
    let mut out = Vec::with_capacity(CUBE_N.pow(3));
    for b in 0..CUBE_N {
        for g in 0..CUBE_N {
            for r in 0..CUBE_N {
                let at = (g * w + b * CUBE_N + r) * 4;
                out.push([img[at], img[at + 1], img[at + 2]]);
            }
        }
    }
    Ok(out)
}

pub fn strip_of(texels: &[[u8; 3]]) -> Vec<u8> {
    let w = CUBE_N * CUBE_N;
    let mut rgba = vec![0u8; w * CUBE_N * 4];
    for (i, t) in texels.iter().enumerate().take(CUBE_N.pow(3)) {
        let (r, g, b) = (i % CUBE_N, i / CUBE_N % CUBE_N, i / (CUBE_N * CUBE_N));
        let at = (g * w + b * CUBE_N + r) * 4;
        rgba[at..at + 4].copy_from_slice(&[t[0], t[1], t[2], 255]);
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_postfx_record_reads_round_trips_and_refuses_with_lines() {
        let text = br#"bloom_blur_x = 2.5
vignette_colour = [0.1, 0.2, 0.3, 1.0]
0x8e581409 = 1.0
layer = "track_layer"
cube = "warm:postfx/warm_cube"
colour_cubes = [{ stack = 0, mode = 0, group = "day_sunny_colorcube", weight = 1.0 }]
"#;
        let r = parse_toml(text).unwrap();
        assert_eq!(r.colour_cubes[0].group, "day_sunny_colorcube");
        assert_eq!(PostFxRecord::from_table(&r.to_table()).unwrap(), r);
        let bad = br#"bloom_blur_x = "x"
cube = "warm:texture/c"
colour_cubes = [{ stack = 0, mode = 3, group = "g", weight = 1.0 }]
vignette_colour = [1.0]
"#;
        let p = parse_toml(bad).unwrap_err();
        assert_eq!(p.iter().map(|(l, _, _)| *l).collect::<Vec<_>>(), [1, 2, 3, 4], "{p:?}");
    }

    #[test]
    fn a_cube_strip_round_trips_and_wrong_sizes_are_refused() {
        let texels: Vec<[u8; 3]> =
            (0..CUBE_N.pow(3)).map(|i| [(i % 251) as u8, (i % 13) as u8, (i / 1024) as u8]).collect();
        let rgba = strip_of(&texels);
        let mut png = Vec::new();
        let mut e = ::png::Encoder::new(&mut png, 1024, 32);
        e.set_color(::png::ColorType::Rgba);
        e.set_depth(::png::BitDepth::Eight);
        e.write_header().unwrap().write_image_data(&rgba).unwrap();
        assert_eq!(cube_from_strip("c.png", &png).unwrap(), texels);
        let mut small = Vec::new();
        let mut e = ::png::Encoder::new(&mut small, 32, 32);
        e.set_color(::png::ColorType::Rgba);
        e.write_header().unwrap().write_image_data(&vec![0; 32 * 32 * 4]).unwrap();
        assert!(cube_from_strip("c.png", &small).unwrap_err().contains("1024×32"));
        assert!(cube_from_strip("c.jpg", &png).is_err());
    }
}
