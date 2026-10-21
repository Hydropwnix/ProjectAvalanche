use std::collections::HashMap;

use bevy::math::{Vec3, Vec4};
use ssx_formats::lynx::{Param, Value};
use ssx_formats::lynx_schema::{self, ParamDefault};

#[derive(Clone, Debug)]
pub(super) struct ParamDef {
    base: Vec<Vec4>,
    states: Vec<(u8, Vec4)>,
    pub(super) text: Option<String>,
}

#[derive(Clone, Copy)]
pub struct NameHasher(u64);

impl Default for NameHasher {
    fn default() -> Self {
        NameHasher(0xcbf2_9ce4_8422_2325)
    }
}

impl std::hash::Hasher for NameHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

type NameMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<NameHasher>>;

#[derive(Clone, Debug, Default)]
pub struct Params {
    pub(super) map: NameMap<String, ParamDef>,
    defaults: NameMap<&'static str, Vec4>,
}

fn lanes(v: &Value, previous: Vec4) -> Vec4 {
    match v {
        Value::Bool(b) => Vec4::new(*b as u8 as f32, 0.0, 0.0, 0.0),
        Value::Int(i) => Vec4::new(*i as f32, 0.0, 0.0, 0.0),
        Value::Float(f) => Vec4::new(*f, 0.0, 0.0, 0.0),
        Value::Vector(c) => {
            let mut out = previous;
            for (i, x) in c.iter().enumerate().take(4) {
                out[i] = *x;
            }
            out
        }
        Value::Reference(_) | Value::String(_) => Vec4::ZERO,
    }
}

pub(super) fn default_lanes(d: ParamDefault) -> Vec4 {
    match d {
        ParamDefault::Bool(b) => Vec4::new(b as u8 as f32, 0.0, 0.0, 0.0),
        ParamDefault::Int(i) => Vec4::new(i as f32, 0.0, 0.0, 0.0),
        ParamDefault::Float(f) => Vec4::new(f, 0.0, 0.0, 0.0),
        ParamDefault::Vector(v) => Vec4::from_array(v),
        ParamDefault::None => Vec4::ZERO,
    }
}

impl Params {
    pub(super) fn new(class: &str, base: &[Param], states: &[(u8, &[Param])]) -> Self {
        let schema = lynx_schema::class(class);
        let default_of =
            |name: &str| schema.and_then(|c| c.param(name)).map_or(Vec4::ZERO, |p| default_lanes(p.default));
        let mut defaults: NameMap<&'static str, Vec4> = NameMap::default();
        for row in schema.map_or(&[][..], |c| c.params) {
            defaults.entry(row.name).or_insert_with(|| default_lanes(row.default));
        }
        let mut map: NameMap<String, ParamDef> = NameMap::default();
        for p in base {
            let def = map.entry(p.name.clone()).or_insert_with(|| ParamDef {
                base: vec![default_of(&p.name)],
                states: Vec::new(),
                text: None,
            });
            let i = p.index as usize;
            if def.base.len() <= i {
                def.base.resize(i + 1, default_of(&p.name));
            }
            def.base[i] = lanes(&p.value, def.base[i]);
            if let Value::String(s) | Value::Reference(s) = &p.value {
                def.text = Some(s.clone());
            }
        }
        for (state, list) in states {
            for p in list.iter().filter(|p| p.index == 0) {
                let def = map.entry(p.name.clone()).or_insert_with(|| ParamDef {
                    base: vec![default_of(&p.name)],
                    states: Vec::new(),
                    text: None,
                });
                let v = lanes(&p.value, default_of(&p.name));
                def.states.push((*state, v));
            }
        }
        Params { map, defaults }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Weights([f32; 16]);

impl Weights {
    pub fn from_list(list: &[(u8, f32)]) -> Self {
        let mut w = [0.0; 16];
        for &(s, x) in list {
            if (s as usize) < 16 {
                w[s as usize] = x;
            }
        }
        Weights(w)
    }
}

#[derive(Clone, Copy)]
pub struct View<'a> {
    params: &'a Params,
    w: &'a Weights,
}

impl<'a> View<'a> {
    pub fn new(params: &'a Params, w: &'a Weights) -> Self {
        View { params, w }
    }

    pub fn v(&self, name: &str, i: usize) -> Vec4 {
        match self.params.map.get(name) {
            Some(def) => self.blended(def, i),
            None => self.params.defaults.get(name).copied().unwrap_or(Vec4::ZERO),
        }
    }
    fn blended(&self, def: &ParamDef, i: usize) -> Vec4 {
        let base = def.base[i.min(def.base.len() - 1)];
        if i != 0 || def.states.is_empty() {
            return base;
        }
        let (mut sum_w, mut acc) = (0.0, Vec4::ZERO);
        for &(s, v) in &def.states {
            let w = self.w.0[s as usize & 15];
            sum_w += w;
            acc += v * w;
        }
        base * (1.0 - sum_w) + acc
    }
    pub fn v3(&self, name: &str) -> Vec3 {
        self.v(name, 0).truncate()
    }
    pub fn f(&self, name: &str) -> f32 {
        self.v(name, 0).x
    }
    pub fn i(&self, name: &str) -> i32 {
        match self.params.map.get(name) {
            Some(def) if !def.states.is_empty() => (self.blended(def, 0).x + 0.5).trunc() as i32,
            Some(def) => self.blended(def, 0).x as i32,
            None => self.params.defaults.get(name).map_or(0.0, |v| v.x) as i32,
        }
    }
    pub fn b(&self, name: &str) -> bool {
        self.v(name, 0).x > 0.5
    }
    pub fn count(&self, name: &str) -> usize {
        self.params.map.get(name).map_or(1, |d| d.base.len())
    }
    pub fn text(&self, name: &str) -> Option<&'a str> {
        self.params.map.get(name).and_then(|d| d.text.as_deref())
    }
    pub fn names(&self) -> impl Iterator<Item = &'a str> {
        self.params.map.keys().map(String::as_str)
    }
    pub fn built_in(&self, name: &str) -> bool {
        self.params.defaults.contains_key(name)
    }
}
