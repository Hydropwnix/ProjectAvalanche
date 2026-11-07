use std::collections::BTreeSet;

use serde::Deserialize;

use super::{FloatTrack, Interpolation, Keys, MAX_FLOAT_TRACKS, MAX_NAME_BYTES, check_keys};
use crate::manifest::line_col;
use crate::material::Problem;

#[derive(Clone, Debug, PartialEq)]
pub struct AnimToml {
    pub gltf: Option<String>,
    pub animation: Option<String>,
    pub additive: bool,
    pub floats: Vec<FloatTrack>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawFloat {
    pub(crate) dof: Option<toml::Spanned<String>>,
    pub(crate) interpolation: Option<toml::Spanned<String>>,
    pub(crate) times: Option<toml::Spanned<Vec<f64>>>,
    pub(crate) values: Option<toml::Spanned<Vec<f64>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawAnimTable {
    pub(crate) gltf: Option<toml::Spanned<String>>,
    pub(crate) animation: Option<toml::Spanned<String>>,
    pub(crate) additive: Option<toml::Spanned<bool>>,
    pub(crate) float: Option<toml::Spanned<Vec<toml::Spanned<RawFloat>>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawAnim {
    pub(crate) anim: Option<toml::Spanned<RawAnimTable>>,
}

fn name_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= MAX_NAME_BYTES && !s.chars().any(char::is_control)
}

fn floats(list: &[f64]) -> Result<Vec<f32>, usize> {
    let out: Vec<f32> = list.iter().map(|&x| x as f32).collect();
    match out.iter().position(|x| !x.is_finite()) {
        Some(i) => Err(i),
        None => Ok(out),
    }
}

pub fn parse_toml(bytes: &[u8]) -> Result<AnimToml, Vec<Problem>> {
    crate::package::parse_record_at(bytes)
        .map_err(|(line, column, m)| vec![(line, column, ssx_core::text::message(&m).into_string())])?;
    let text = std::str::from_utf8(bytes).map_err(|e| vec![(0, 0, format!("not UTF-8: {e}"))])?;
    let raw: RawAnim = ::toml::from_str(text).map_err(|e| {
        let (line, column) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        vec![(line, column, ssx_core::text::message(e.message()).into_string())]
    })?;
    let mut problems = Vec::new();
    let mut at = |span: std::ops::Range<usize>, m: String| {
        let (line, column) = line_col(text, span.start);
        problems.push((line, column, m));
    };
    let Some(table) = raw.anim else {
        return Err(vec![(0, 0, "an anim.toml has an [anim] table".into())]);
    };
    let span = table.span();
    let a = table.into_inner();
    let gltf = a.gltf.as_ref().map(|s| {
        let lower = s.get_ref().to_ascii_lowercase();
        if !(lower.ends_with(".glb") || lower.ends_with(".gltf")) {
            at(s.span(), "gltf: a package path to a .glb or .gltf file".into());
        } else if let Err(e) = crate::path::check_package_path(s.get_ref(), false) {
            at(s.span(), format!("gltf: {e}"));
        }
        s.get_ref().clone()
    });
    let animation = a.animation.as_ref().map(|s| {
        if !name_ok(s.get_ref()) {
            at(s.span(), format!("animation: 1 to {MAX_NAME_BYTES} bytes, no control characters"));
        }
        if gltf.is_none() {
            at(s.span(), "animation: names an animation of the glTF file, but the clip names none (`gltf`)".into());
        }
        s.get_ref().clone()
    });
    let additive = a.additive.as_ref().is_some_and(|b| *b.get_ref());
    let mut list = Vec::new();
    if let Some(fl) = &a.float {
        if fl.get_ref().len() > MAX_FLOAT_TRACKS {
            at(fl.span(), format!("{} float tracks; at most {MAX_FLOAT_TRACKS}", fl.get_ref().len()));
        } else {
            let mut seen = BTreeSet::new();
            for (i, f) in fl.get_ref().iter().enumerate() {
                let fspan = f.span();
                let f = f.get_ref();
                let what = format!("[[anim.float]] {}", i + 1);
                let dof = match &f.dof {
                    Some(d) if name_ok(d.get_ref()) && seen.insert(d.get_ref().clone()) => d.get_ref().clone(),
                    Some(d) if name_ok(d.get_ref()) => {
                        at(
                            d.span(),
                            format!("{what}: dof {} has a track already", ssx_core::text::quoted(d.get_ref())),
                        );
                        continue;
                    }
                    Some(d) => {
                        at(d.span(), format!("{what}: dof: 1 to {MAX_NAME_BYTES} bytes, no control characters"));
                        continue;
                    }
                    None => {
                        at(
                            fspan,
                            format!(
                                "{what}: no dof (the float DOF it drives as `<layout>.<name>`, e.g. \"TrajectoryVelocityRig.AITrajectory.height\")"
                            ),
                        );
                        continue;
                    }
                };
                let interpolation = match &f.interpolation {
                    None => Interpolation::Linear,
                    Some(s) => match Interpolation::parse(s.get_ref()) {
                        Some(i) => i,
                        None => {
                            at(s.span(), format!("{what}: interpolation: step, linear or cubic"));
                            continue;
                        }
                    },
                };
                let (Some(times), Some(values)) = (&f.times, &f.values) else {
                    at(fspan, format!("{what}: a float track has times and values"));
                    continue;
                };
                let t = match floats(times.get_ref()) {
                    Ok(t) => t,
                    Err(i) => {
                        at(times.span(), format!("{what}: times[{i}] is not a finite 32-bit number"));
                        continue;
                    }
                };
                let v = match floats(values.get_ref()) {
                    Ok(v) => v,
                    Err(i) => {
                        at(values.span(), format!("{what}: values[{i}] is not a finite 32-bit number"));
                        continue;
                    }
                };
                let keys = Keys { interpolation, times: t, values: v };
                match check_keys(&keys, None, &what) {
                    Ok(()) => list.push(FloatTrack { dof, keys }),
                    Err(m) => at(times.span(), m),
                }
            }
        }
    }
    if gltf.is_none() && a.float.as_ref().is_none_or(|f| f.get_ref().is_empty()) {
        at(span, "the clip has no track: name a glTF animation (`gltf`) or give float tracks ([[anim.float]])".into());
    }
    if problems.is_empty() {
        Ok(AnimToml { gltf, animation, additive, floats: list })
    } else {
        problems.sort();
        Err(problems)
    }
}
