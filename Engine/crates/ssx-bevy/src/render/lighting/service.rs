use std::collections::BTreeMap;

use ssx_assets::track_lighting::LightingPreset;
use ssx_formats::attrib::fnv1;

pub const DEFAULT_LIGHTING: u32 = fnv1(b"default_lighting");

pub type Presets = BTreeMap<u32, LightingPreset>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightingService {
    stack: Vec<(u32, u32)>,
    append: Vec<(u32, u32)>,
    next_handle: u32,
    overrides: BTreeMap<String, [f32; 4]>,
    pub condition: u32,
    effective: (u32, BTreeMap<String, [f32; 4]>),
    pub blend: f32,
    pub serial: u64,
}

impl LightingService {
    pub fn new(base: u32, presets: &Presets) -> Self {
        let mut s = LightingService { stack: vec![(0, 0)], next_handle: 1, ..Default::default() };
        s.set_base(0.0, base, presets);
        s.serial = 0;
        s
    }

    pub fn set_base(&mut self, blend: f32, key: u32, presets: &Presets) {
        if let Some(e) = self.stack.first_mut() {
            e.0 = key;
        }
        self.flush(blend, presets);
    }

    fn handle(&mut self) -> u32 {
        if self.next_handle == 0 {
            self.next_handle = 1;
        }
        self.next_handle += 1;
        self.next_handle - 1
    }

    pub fn add_condition(&mut self, blend: f32, key: u32, presets: &Presets) -> u32 {
        let h = self.handle();
        self.stack.push((key, h));
        self.flush(blend, presets);
        h
    }

    pub fn remove_condition(&mut self, blend: f32, handle: u32, presets: &Presets) {
        if let Some(i) = self.stack.iter().position(|e| e.1 == handle) {
            self.stack.remove(i);
            self.flush(blend, presets);
        }
    }

    pub fn add_appended(&mut self, blend: f32, key: u32, presets: &Presets) -> u32 {
        let h = self.handle();
        self.append.push((key, h));
        self.flush(blend, presets);
        h
    }

    pub fn remove_appended(&mut self, blend: f32, handle: u32, presets: &Presets) {
        if let Some(i) = self.append.iter().position(|e| e.1 == handle) {
            self.append.remove(i);
            self.flush(blend, presets);
        }
    }

    pub fn override_from_param(&mut self, target: &str, source: &str, presets: &Presets) {
        let Some(p) = presets.get(&self.condition) else { return };
        let value = p.float(source).map(|x| [x, 0.0, 0.0, 0.0]).or_else(|| p.vector(source));
        if let Some(v) = value {
            self.overrides.insert(target.to_string(), v);
        }
    }

    pub fn remove_override(&mut self, target: &str) {
        self.overrides.remove(target);
    }

    pub fn flush(&mut self, blend: f32, presets: &Presets) {
        let top = self.append.last().or(self.stack.last()).map_or(0, |e| e.0);
        self.condition = if presets.contains_key(&top) { top } else { DEFAULT_LIGHTING };
        let effective = (self.condition, self.overrides.clone());
        // TODO(guess): a flush with overrides counts as a new +0x54 key every time (0x8313E000's key not decoded); settle: 0x8313E000's key.
        if effective != self.effective || !effective.1.is_empty() {
            self.serial += 1;
        }
        self.effective = effective;
        self.blend = blend;
    }

    pub fn collection(&self, presets: &Presets) -> Option<LightingPreset> {
        let mut p = presets.get(&self.effective.0)?.clone();
        for (name, v) in &self.effective.1 {
            if let Some(x) = p.floats.get_mut(name) {
                *x = v[0];
            } else if let Some(x) = p.vectors.get_mut(name) {
                *x = *v;
            }
        }
        Some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn presets() -> Presets {
        let preset = |name: &str, key: f32| {
            let mut p = LightingPreset { name: name.into(), ..Default::default() };
            p.floats.insert("dynamicExposureKeyValue".into(), key);
            p.floats.insert("heliInteriorDynamicExposureKeyValue".into(), 0.25);
            (fnv1(name.as_bytes()), p)
        };
        [preset("day_lighting", 0.5), preset("whiteout_lighting", 0.7), preset("default_lighting", 0.1)].into()
    }

    #[test]
    fn the_last_condition_added_is_in_force_until_it_is_removed() {
        let (p, day, white) = (presets(), fnv1(b"day_lighting"), fnv1(b"whiteout_lighting"));
        let mut s = LightingService::new(day, &p);
        assert_eq!((s.condition, s.serial), (day, 0));
        let h = s.add_condition(10.0, white, &p);
        assert_eq!((h, s.condition, s.blend, s.serial), (1, white, 10.0, 1));
        s.remove_condition(3.0, 7, &p);
        assert_eq!((s.condition, s.blend, s.serial), (white, 10.0, 1));
        s.remove_condition(4.0, h, &p);
        assert_eq!((s.condition, s.blend, s.serial), (day, 4.0, 2));
        let h = s.add_condition(0.0, 0x1234, &p);
        assert_eq!((h, s.condition), (2, DEFAULT_LIGHTING));
    }

    #[test]
    fn overrides_take_effect_at_the_flush_and_copy_the_condition() {
        let (p, day) = (presets(), fnv1(b"day_lighting"));
        let mut s = LightingService::new(day, &p);
        s.override_from_param("dynamicExposureKeyValue", "heliInteriorDynamicExposureKeyValue", &p);
        s.override_from_param("fillIntensity", "noSuchParameter", &p);
        assert_eq!(s.serial, 0);
        assert_eq!(s.collection(&p).and_then(|c| c.float("dynamicExposureKeyValue")), Some(0.5));
        s.flush(2.0, &p);
        assert_eq!((s.serial, s.blend), (1, 2.0));
        assert_eq!(s.collection(&p).and_then(|c| c.float("dynamicExposureKeyValue")), Some(0.25));
        s.remove_override("dynamicExposureKeyValue");
        s.flush(1.0, &p);
        assert_eq!(s.serial, 2);
        assert_eq!(s.collection(&p).and_then(|c| c.float("dynamicExposureKeyValue")), Some(0.5));
        s.flush(5.0, &p);
        assert_eq!(s.serial, 2);
    }
}
