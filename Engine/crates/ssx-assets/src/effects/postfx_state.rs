use std::collections::BTreeMap;

use ssx_formats::attrib::fnv1;

use crate::postfx::{LevelPresets, PostFxPreset};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateField {
    pub key: u32,
    pub name: Option<&'static str>,
    pub offset: u16,
    pub lanes: usize,
}

const fn named(name: &'static str, offset: u16, lanes: usize) -> StateField {
    StateField { key: fnv1(name.as_bytes()), name: Some(name), offset, lanes }
}

const fn unnamed(key: u32, offset: u16, lanes: usize) -> StateField {
    StateField { key, name: None, offset, lanes }
}

pub const FIELD_COUNT: usize = 31;

pub const FIELDS: [StateField; FIELD_COUNT] = [
    unnamed(0x8E58_1409, 0x00, 1),
    named("bloom_blur_x", 0x04, 1),
    named("bloom_blur_y", 0x08, 1),
    named("colourcube_depth_far", 0x0C, 1),
    named("colourcube_depth_near", 0x10, 1),
    unnamed(0x2DD8_D98E, 0x14, 1),
    unnamed(0xF1C5_C1FC, 0x18, 1),
    unnamed(0x8A34_76D2, 0x1C, 1),
    named("motion_blur_intensity", 0x20, 1),
    named("motion_blur_origin", 0x24, 4),
    named("overlay_1_tintcolour", 0x34, 4),
    named("overlay_1_intensity", 0x44, 1),
    named("overlay_2_tintcolour", 0x48, 4),
    named("overlay_2_intensity", 0x58, 1),
    named("overlay_fadeoffbounds", 0x5C, 3),
    unnamed(0x5320_D81A, 0x68, 1),
    named("topogrid_scale", 0x6C, 1),
    named("topogrid_colour", 0x70, 4),
    named("topogrid_colour_grid", 0x80, 4),
    named("topogrid_dist_start", 0x90, 1),
    named("topogrid_dist_end", 0x94, 1),
    named("topogrid_dist_fadein", 0x98, 1),
    named("topogrid_dist_fadeout", 0x9C, 1),
    named("topogrid_pulse_rate", 0xA0, 1),
    named("topogrid_pulse_falloff", 0xA4, 1),
    unnamed(0xDE26_D7F4, 0xA8, 1),
    named("topogrid_min_intensities", 0xAC, 2),
    named("topogrid_max_intensities", 0xB4, 2),
    unnamed(0x9D89_7880, 0xBC, 1),
    named("vignette_colour", 0xC0, 4),
    named("vignette_fadeoffbounds", 0xD0, 2),
];

pub const COLOUR_CUBE_ROOM: usize = 16;

pub fn field_index(key: u32) -> Option<usize> {
    FIELDS.iter().position(|f| f.key == key)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CubeEntry {
    pub stack: u16,
    pub mode: u16,
    pub group: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostFxState {
    pub fields: [Option<[f32; 4]>; FIELD_COUNT],
    pub colour_cubes: Vec<CubeEntry>,
}

fn preset_value(p: &PostFxPreset, f: &StateField) -> Result<Option<[f32; 4]>, String> {
    let hex = format!("{:#010x}", f.key);
    let label = f.name.unwrap_or(&hex);
    let float = f.name.and_then(|n| p.floats.get(n)).or_else(|| p.floats.get(&hex)).copied();
    let vector = f.name.and_then(|n| p.vectors.get(n)).or_else(|| p.vectors.get(&hex));
    match (f.lanes, float, vector) {
        (1, Some(x), _) => Ok(Some([x, 0.0, 0.0, 0.0])),
        (n, None, Some(v)) if v.len() == n => Ok(Some(std::array::from_fn(|i| v.get(i).copied().unwrap_or(0.0)))),
        (_, None, None) => Ok(None),
        (1, None, Some(v)) => Err(format!("{label}: a float, given {} numbers", v.len())),
        (n, _, Some(v)) => Err(format!("{label}: {n} numbers, given {}", v.len())),
        (n, Some(_), None) => Err(format!("{label}: {n} numbers, given a float")),
    }
}

impl PostFxState {
    pub fn of(p: &PostFxPreset) -> PostFxState {
        let mut s = PostFxState::default();
        for (slot, f) in s.fields.iter_mut().zip(FIELDS.iter()) {
            // PORT(choice): a package's preset field in another shape than its class type (a record takes a number or
            // 2 to 4 of them for any field) is left out of the state; the console's vault holds every field at its
            // type's size (Float, Vector2/3/4), which the load copies.
            *slot = preset_value(p, f).ok().flatten();
        }
        // PORT(choice): the state keeps a package preset's first 16 colour-cube entries (a record takes up to 64); the
        // console's load copies all of them into the 16 entries' room at +0xD8 and on over the count at +0x198 (no
        // bound, 0x82FD29F0…0x82FD2A4C); the disc's presets have one.
        s.colour_cubes = p
            .colour_cubes
            .iter()
            .take(COLOUR_CUBE_ROOM)
            .map(|b| CubeEntry { stack: b.stack as u16, mode: b.mode as u16, group: b.group.clone(), weight: b.weight })
            .collect();
        s
    }

    pub fn refused(p: &PostFxPreset) -> Vec<String> {
        let mut out: Vec<String> = FIELDS.iter().filter_map(|f| preset_value(p, f).err()).collect();
        if p.colour_cubes.len() > COLOUR_CUBE_ROOM {
            out.push(format!("{} colour-cube entries (the state holds {COLOUR_CUBE_ROOM})", p.colour_cubes.len()));
        }
        out
    }

    pub fn get(&self, key: u32) -> Option<[f32; 4]> {
        field_index(key).and_then(|i| self.fields[i])
    }

    pub fn float(&self, key: u32) -> Option<f32> {
        self.get(key).map(|v| v[0])
    }

    pub fn field_mut(&mut self, key: u32) -> Option<(&mut [f32; 4], usize)> {
        let i = field_index(key)?;
        self.fields[i].as_mut().map(|v| (v, FIELDS[i].lanes))
    }

    pub fn blend(&mut self, src: &PostFxState, w: f32) {
        for ((acc, s), f) in self.fields.iter_mut().zip(&src.fields).zip(FIELDS.iter()) {
            let Some(s) = s else { continue };
            *acc = Some(match *acc {
                None => *s,
                Some(a) if f.lanes == 1 => [(s[0] - a[0]).mul_add(w, a[0]), 0.0, 0.0, 0.0],
                Some(a) => std::array::from_fn(|i| if i < f.lanes { a[i] + (s[i] - a[i]) * w } else { 0.0 }),
            });
        }
        for e in &src.colour_cubes {
            let same = |a: &CubeEntry| a.stack == e.stack && a.mode == e.mode && a.group == e.group;
            match self.colour_cubes.iter().position(same) {
                Some(i) => {
                    let a = &mut self.colour_cubes[i];
                    a.weight = (e.weight - a.weight).mul_add(w, a.weight);
                }
                None if self.colour_cubes.len() < COLOUR_CUBE_ROOM => {
                    self.colour_cubes.push(CubeEntry { weight: e.weight * w, ..e.clone() });
                }
                // PORT(choice): a 17th distinct colour-cube entry of a frame is dropped (packages' presets on several
                // layers can bring that many); the console's blend appends it past the 16 entries' room, over the
                // count at +0x198 (no bound, 0x82FD35CC); the disc's presets bring one each.
                None => {}
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostFxValues {
    pub fields: [[f32; 4]; FIELD_COUNT],
    pub colour_cubes: Vec<CubeEntry>,
}

impl PostFxValues {
    pub fn hold(&mut self, frame: &PostFxState) {
        for (v, f) in self.fields.iter_mut().zip(&frame.fields) {
            if let Some(f) = f {
                *v = *f;
            }
        }
        self.colour_cubes.clone_from(&frame.colour_cubes);
    }

    pub fn get(&self, key: u32) -> [f32; 4] {
        field_index(key).map_or([0.0; 4], |i| self.fields[i])
    }

    pub fn float(&self, key: u32) -> f32 {
        self.get(key)[0]
    }
}

pub const GOGGLES_PRIORITY: u32 = 5;

pub fn layer_priority(layer: &str) -> Option<u32> {
    let order =
        ["default", "track", "environment", "player", "frontend", "goggles", "overlay_layer2", "overlay_layer1"];
    match layer {
        "motion_blur" => Some(8),
        "fade" => Some(9),
        _ => order.iter().position(|l| *l == layer).and_then(|i| u32::try_from(i).ok()),
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layer<S> {
    pub key: u32,
    pub current: S,
    pub prev: S,
    pub prev_weight: f32,
    pub from: f32,
    pub to: f32,
    pub rem: f32,
    pub dur: f32,
    pub stopping: bool,
}

impl<S: Clone + Default> Layer<S> {
    pub fn weight(&self) -> f32 {
        if self.rem > 0.0 { (1.0 - self.rem / self.dur).mul_add(self.to - self.from, self.from) } else { self.to }
    }

    pub fn start(&mut self, key: u32, state: S, blend_in: f32, weight: f32) {
        let w = self.weight();
        if blend_in <= 0.0 || w <= 0.0 {
            (self.prev, self.prev_weight) = (S::default(), 0.0);
        } else {
            (self.prev, self.prev_weight) = (self.current.clone(), w);
        }
        (self.current, self.key, self.from, self.to, self.stopping) = (state, key, 0.0, weight, false);
        (self.rem, self.dur) = (blend_in, blend_in);
    }

    pub fn stop(&mut self, key: u32, blend_out: f32) {
        if self.key != key {
            return;
        }
        (self.prev, self.prev_weight) = (S::default(), 0.0);
        (self.from, self.to, self.rem, self.dur) = (self.weight(), 0.0, blend_out, blend_out);
        if blend_out <= 0.0 {
            (self.current, self.key, self.stopping) = (S::default(), 0, false);
        } else {
            self.stopping = true;
        }
    }

    pub fn set_weight(&mut self, key: u32, weight: f32) {
        if self.key != key {
            return;
        }
        (self.prev, self.prev_weight) = (S::default(), 0.0);
        (self.from, self.to, self.stopping, self.rem, self.dur) = (weight, weight, false, 0.0, 0.0);
    }

    pub fn reload(&mut self, key: u32, state: S) {
        if self.key == key {
            self.current = state;
        }
    }

    pub fn update(&mut self, dt: f32) {
        if self.rem.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
            return;
        }
        self.rem -= dt;
        if self.rem <= 0.0 {
            (self.rem, self.prev, self.prev_weight) = (0.0, S::default(), 0.0);
            if self.stopping {
                (self.current, self.key, self.from, self.to, self.dur, self.stopping) =
                    (S::default(), 0, 0.0, 0.0, 0.0, false);
            }
        }
    }

    pub fn blends(&self) -> Vec<(&S, f32)> {
        let mut out = Vec::new();
        let w = self.weight();
        if self.rem > 0.0 {
            let pw = self.rem / self.dur * self.prev_weight;
            if pw > 0.0 {
                out.push((&self.prev, pw));
            }
        }
        if w > 0.0 {
            out.push((&self.current, w));
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct PostFxScene {
    pub base: PostFxState,
    pub presets: BTreeMap<String, (Option<u32>, PostFxState)>,
    pub held: PostFxValues,
}

impl PostFxScene {
    pub fn of(presets: &LevelPresets) -> PostFxScene {
        let mut scene = PostFxScene { base: PostFxState::of(&presets.base), ..PostFxScene::default() };
        for (name, p) in &presets.named {
            let priority = p.layer.as_deref().and_then(layer_priority);
            scene.presets.insert(name.clone(), (priority, PostFxState::of(p)));
        }
        scene
    }

    pub fn frame(&mut self, running: &[(String, f32)], goggles: &[(&PostFxState, f32)]) -> &PostFxValues {
        let mut by_layer: BTreeMap<u32, Vec<(&PostFxState, f32)>> = BTreeMap::new();
        by_layer.insert(GOGGLES_PRIORITY, goggles.to_vec());
        let mut tracks: BTreeMap<u32, (&PostFxState, f32)> = BTreeMap::new();
        for (name, w) in running {
            if let Some((Some(priority), st)) = self.presets.get(name) {
                tracks.insert(*priority, (st, *w));
            }
        }
        for (priority, entry) in tracks {
            by_layer.entry(priority).or_default().push(entry);
        }
        let mut acc = PostFxState::default();
        acc.blend(&self.base, 1.0);
        for (st, w) in by_layer.into_values().flatten() {
            if w > 0.0 {
                acc.blend(st, w);
            }
        }
        self.held.hold(&acc);
        &self.held
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::postfx::ColourCubeBlend;

    fn key(name: &str) -> u32 {
        fnv1(name.as_bytes())
    }

    fn state(fields: &[(&str, &[f32])]) -> PostFxState {
        let mut s = PostFxState::default();
        for (name, v) in fields {
            let i = field_index(key(name)).unwrap();
            s.fields[i] = Some(std::array::from_fn(|j| v.get(j).copied().unwrap_or(0.0)));
        }
        s
    }

    fn cube(group: &str, weight: f32) -> CubeEntry {
        CubeEntry { stack: 1, mode: 0, group: group.into(), weight }
    }

    #[test]
    fn the_fields_follow_the_load() {
        let keys = [
            ("bloom_blur_x", 0xCA95_B073),
            ("colourcube_depth_far", 0xF408_EB22),
            ("motion_blur_intensity", 0x9823_741B),
            ("motion_blur_origin", 0x24FB_AE56),
            ("overlay_1_tintcolour", 0x4C2B_5DDD),
            ("overlay_fadeoffbounds", 0xB62D_5EC0),
            ("topogrid_min_intensities", 0x895A_AD5A),
            ("vignette_fadeoffbounds", 0x18EC_F294),
        ];
        for (name, k) in keys {
            assert_eq!(FIELDS[field_index(k).unwrap()].name, Some(name));
        }
        for pair in FIELDS.windows(2) {
            assert_eq!(pair[0].offset as usize + 4 * pair[0].lanes, pair[1].offset as usize, "{pair:?}");
        }
        assert_eq!(FIELDS[FIELD_COUNT - 1].offset + 8, 0xD8);
    }

    #[test]
    fn the_load_takes_the_fields_in_their_class_shape() {
        let mut p = PostFxPreset::default();
        p.floats.insert("bloom_blur_x".into(), 3.0);
        p.floats.insert(format!("{:#010x}", key("overlay_1_intensity")), 0.5);
        p.vectors.insert("topogrid_min_intensities".into(), vec![0.1, 0.5]);
        p.vectors.insert("topogrid_colour".into(), vec![1.0, 2.0, 3.0]);
        p.vectors.insert("topogrid_scale".into(), vec![1.0, 2.0]);
        p.colour_cubes = vec![ColourCubeBlend { stack: 1, mode: 0, group: "g".into(), weight: 1.0 }; 17];
        let s = PostFxState::of(&p);
        assert_eq!(s.float(key("bloom_blur_x")), Some(3.0));
        assert_eq!(s.float(key("overlay_1_intensity")), Some(0.5));
        assert_eq!(s.get(key("topogrid_min_intensities")), Some([0.1, 0.5, 0.0, 0.0]));
        assert_eq!((s.get(key("topogrid_colour")), s.get(key("topogrid_scale"))), (None, None));
        assert_eq!(s.colour_cubes.len(), COLOUR_CUBE_ROOM);
        let refused = PostFxState::refused(&p);
        assert_eq!(refused.len(), 3, "{refused:?}");
    }

    #[test]
    fn the_blend_copies_lerps_and_merges() {
        let mut acc = state(&[("overlay_1_intensity", &[0.0]), ("topogrid_colour", &[0.0, 1.0, 0.5])]);
        acc.colour_cubes.push(cube("a", 1.0));
        let mut src = state(&[
            ("overlay_1_intensity", &[0.8]),
            ("topogrid_colour", &[0.375, 0.47, 0.5]),
            ("topogrid_dist_end", &[300.0]),
        ]);
        src.colour_cubes = vec![cube("a", 0.5), cube("b", 0.5)];
        acc.blend(&src, 0.25);
        assert_eq!(acc.float(key("overlay_1_intensity")), Some((0.8f32 - 0.0).mul_add(0.25, 0.0)));
        assert_eq!(acc.get(key("topogrid_colour")), Some([0.375 * 0.25, 1.0 + (0.47 - 1.0) * 0.25, 0.5, 0.0]));
        assert_eq!(acc.float(key("topogrid_dist_end")), Some(300.0));
        assert_eq!(acc.colour_cubes, vec![cube("a", (0.5f32 - 1.0).mul_add(0.25, 1.0)), cube("b", 0.125)]);
        let mut full = PostFxState { colour_cubes: (0..16).map(|i| cube(&i.to_string(), 1.0)).collect(), ..acc };
        full.blend(&src, 1.0);
        assert_eq!(full.colour_cubes.len(), COLOUR_CUBE_ROOM);
        let mut held = PostFxValues::default();
        held.fields[0] = [9.0; 4];
        held.hold(&src);
        assert_eq!((held.fields[0], held.float(key("topogrid_dist_end"))), ([9.0; 4], 300.0));
    }

    #[test]
    fn the_layer_ramps_in_and_out() {
        let shown = state(&[("topogrid_dist_end", &[300.0])]);
        let mut l = Layer::<PostFxState>::default();
        l.start(7, shown, 1.0, 1.0);
        assert_eq!(l.weight(), 0.0);
        assert!(l.blends().is_empty());
        l.update(0.25);
        assert_eq!(l.weight(), 0.25);
        l.update(1.0);
        assert_eq!((l.weight(), l.rem), (1.0, 0.0));
        l.stop(8, 0.4);
        assert_eq!(l.key, 7, "another preset's stop changes nothing");
        l.stop(7, 0.4);
        l.update(0.2);
        assert_eq!(l.weight(), 0.5);
        l.update(0.3);
        assert_eq!((l.key, l.current.clone(), l.weight()), (0, PostFxState::default(), 0.0));
    }

    #[test]
    fn a_start_over_a_shown_preset_blends_the_old_one_out() {
        let (first, other) = (state(&[("topogrid_dist_end", &[300.0])]), state(&[("topogrid_scale", &[0.5])]));
        let mut l = Layer::<PostFxState>::default();
        l.start(1, first.clone(), 0.0, 1.0);
        l.start(2, other.clone(), 2.0, 1.0);
        l.update(0.5);
        let b = l.blends();
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].0, b[0].1), (&first, 0.75));
        assert_eq!((b[1].0, b[1].1), (&other, 0.25));
        l.reload(1, PostFxState::default());
        assert_eq!(l.current, other, "a reload of another key changes nothing");
    }

    #[test]
    fn layer_priorities_follow_the_vault() {
        assert_eq!(layer_priority("default"), Some(0));
        assert_eq!(layer_priority("goggles"), Some(GOGGLES_PRIORITY));
        assert_eq!(layer_priority("overlay_layer1"), Some(7));
        assert_eq!(layer_priority("fade"), Some(9));
        assert_eq!(layer_priority("nothing"), None);
    }

    #[test]
    fn running_presets_blend_by_layer() {
        let i1 = key("overlay_1_intensity");
        let base = state(&[("overlay_1_intensity", &[0.0]), ("overlay_2_intensity", &[0.0])]);
        let mut scene = PostFxScene { base, ..PostFxScene::default() };
        scene.presets.insert("a2".into(), (Some(6), state(&[("overlay_2_intensity", &[1.0])])));
        scene.presets.insert("b1".into(), (Some(7), state(&[("overlay_1_intensity", &[1.0])])));
        scene.presets.insert("c1".into(), (Some(7), PostFxState::default()));
        let v = scene.frame(&[("b1".into(), 0.5), ("a2".into(), 0.25)], &[]);
        assert_eq!((v.float(i1), v.float(key("overlay_2_intensity"))), (0.5, 0.25));
        let v = scene.frame(&[("b1".into(), 0.5), ("c1".into(), 1.0)], &[]);
        assert_eq!(v.float(i1), 0.0);
    }
}
