use anyhow::{Result, bail};

use crate::tuning::{AssetView, Bundle};

pub const SURFACE_COUNT: usize = 12;
pub const WEIGHT_COUNT: usize = 14;
pub const SURFACE_NAMES: [&str; SURFACE_COUNT] = [
    "Windswept",
    "Crusty",
    "Wet",
    "Groomed",
    "Crud",
    "Powder",
    "Rock",
    "Ice",
    "Wood",
    "Metal",
    "Concrete",
    "MetalHollow",
];
pub const POWDER: usize = 5;
pub const ROCK: usize = 6;
pub const ICE: usize = 7;
pub const AMBIENT_OCCLUSION: usize = 12;
pub const SNOW_COVERAGE: usize = 13;

pub type Weights = [f32; WEIGHT_COUNT];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceProperties {
    pub adhesion_accel: f32,
    pub adhesion_accel_ice_axe_low: f32,
    pub adhesion_accel_ice_axe_high: f32,
    pub adhesion_dist_above: f32,
    pub adhesion_dist_below: f32,
    pub dig_lean: f32,
    pub dig_lean_ice_axe_low: f32,
    pub dig_lean_ice_axe_high: f32,
    pub friction: f32,
    pub friction_brake: f32,
    pub friction_uber_bail: f32,
    pub friction_ice_axe_low: f32,
    pub friction_ice_axe_high: f32,
    pub friction_brake_ice_axe_low: f32,
    pub friction_brake_ice_axe_high: f32,
    pub snow_depth: f32,
    pub riding_depth: f32,
    pub board_chatter_height: f32,
    pub surface_rumble_value: f32,
}

pub const MIN_ADHESION_DIST_ABOVE: f32 = 0.9;
pub const MIN_ADHESION_DIST_BELOW: f32 = 0.12;

impl SurfaceProperties {
    fn from_asset(a: &AssetView<'_>) -> Result<Self> {
        Ok(Self {
            adhesion_accel: a.f32("AdhesionAccel")?,
            adhesion_accel_ice_axe_low: a.f32("AdhesionAccelIceAxeLow")?,
            adhesion_accel_ice_axe_high: a.f32("AdhesionAccelIceAxeHigh")?,
            adhesion_dist_above: a.f32("AdhesionDistAbove")?,
            adhesion_dist_below: a.f32("AdhesionDistBelow")?,
            dig_lean: a.f32("DigLean")?,
            dig_lean_ice_axe_low: a.f32("DigLeanIceAxeLow")?,
            dig_lean_ice_axe_high: a.f32("DigLeanIceAxeHigh")?,
            friction: a.f32("Friction")?,
            friction_brake: a.f32("FrictionBrake")?,
            friction_uber_bail: a.f32("FrictionUberBail")?,
            friction_ice_axe_low: a.f32("FrictionIceAxeLow")?,
            friction_ice_axe_high: a.f32("FrictionIceAxeHigh")?,
            friction_brake_ice_axe_low: a.f32("FrictionBrakeIceAxeLow")?,
            friction_brake_ice_axe_high: a.f32("FrictionBrakeIceAxeHigh")?,
            snow_depth: a.f32("SnowDepth")?,
            riding_depth: a.f32("RidingDepth")?,
            board_chatter_height: 0.0,
            surface_rumble_value: a.f32("SurfaceRumbleValue")?,
        })
    }

    fn fields_mut(&mut self) -> [&mut f32; 19] {
        [
            &mut self.adhesion_accel,
            &mut self.adhesion_accel_ice_axe_low,
            &mut self.adhesion_accel_ice_axe_high,
            &mut self.adhesion_dist_above,
            &mut self.adhesion_dist_below,
            &mut self.dig_lean,
            &mut self.dig_lean_ice_axe_low,
            &mut self.dig_lean_ice_axe_high,
            &mut self.friction,
            &mut self.friction_brake,
            &mut self.friction_uber_bail,
            &mut self.friction_ice_axe_low,
            &mut self.friction_ice_axe_high,
            &mut self.friction_brake_ice_axe_low,
            &mut self.friction_brake_ice_axe_high,
            &mut self.snow_depth,
            &mut self.riding_depth,
            &mut self.board_chatter_height,
            &mut self.surface_rumble_value,
        ]
    }

    fn fields(mut self) -> [f32; 19] {
        self.fields_mut().map(|f| *f)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardChatter {
    pub amplitude: f32,
    pub frequency: f32,
    pub speed_falloff: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ChatterInputs {
    pub distance_carved: f32,
    pub grind_blend: f32,
    pub player_speed: f32,
}

impl BoardChatter {
    pub fn height(&self, c: &ChatterInputs) -> f32 {
        let r = (c.player_speed / self.speed_falloff).clamp(0.0, 1.0);
        let s = (self.frequency * c.distance_carved).sin();
        (self.amplitude + self.amplitude * s) * (1.0 - c.grind_blend) * r
    }

    fn from_asset(a: &AssetView<'_>) -> Result<Self> {
        for (field, name) in [
            ("DistanceCarved", "FLT_carving_state_distance_carved"),
            ("GrindBlendValue", "FLT_carving_state_GrindBlendValue"),
            ("SpeedValue", "FLT_PlayerSpeed"),
        ] {
            if a.asset(field)?.name() != name {
                bail!("{}.{field} is {:?}, expected {name}", a.name(), a.asset(field)?.name());
            }
        }
        Ok(Self {
            amplitude: a.f32("Amplitude")?,
            frequency: a.f32("Frequency")?,
            speed_falloff: a.f32("SpeedMultiplierFalloffPoint")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SurfaceTable {
    pub records: [SurfaceProperties; SURFACE_COUNT],
    pub chatter: [BoardChatter; SURFACE_COUNT],
}

impl SurfaceTable {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let spq = bundle.view("SnowboarderPhysicsQueriesSceneOp", "SnowboarderPhysicsQueriesSceneOpAsset")?;
        let items = spq.asset("SurfacePropertiesListAsset")?.asset_list("AssetListItems")?;
        if items.len() != SURFACE_COUNT {
            bail!("list_surface_properties has {} items, expected {SURFACE_COUNT}", items.len());
        }
        let mut records = [SurfaceProperties::default(); SURFACE_COUNT];
        let mut chatter = [BoardChatter::default(); SURFACE_COUNT];
        for (i, item) in items.iter().enumerate() {
            let expected = format!("SP_{}", SURFACE_NAMES[i]);
            if item.name() != expected || item.class() != "SurfacePropertiesAsset" {
                bail!("list_surface_properties[{i}] is {} {:?}, expected {expected}", item.class(), item.name());
            }
            records[i] = SurfaceProperties::from_asset(item)?;
            chatter[i] = BoardChatter::from_asset(&item.asset("BoardChatterProperties")?)?;
        }
        Ok(Self { records, chatter })
    }

    pub fn blend(&self, w: &Weights, c: &ChatterInputs) -> SurfaceProperties {
        let mut out = [0.0f32; 19];
        for (wi, rec) in w.iter().zip(&self.records) {
            for (o, f) in out.iter_mut().zip(rec.fields()) {
                *o += wi * f;
            }
        }
        let mut o = SurfaceProperties::default();
        for (dst, v) in o.fields_mut().into_iter().zip(out) {
            *dst = v;
        }
        o.adhesion_dist_above = o.adhesion_dist_above.max(MIN_ADHESION_DIST_ABOVE);
        o.adhesion_dist_below = o.adhesion_dist_below.max(MIN_ADHESION_DIST_BELOW);
        o.board_chatter_height = w.iter().zip(&self.chatter).map(|(wi, ch)| wi * ch.height(c)).sum();
        o
    }
}

pub fn no_contact_weights() -> Weights {
    let mut w = [0.0; WEIGHT_COUNT];
    w[POWDER] = 1.0;
    w[SNOW_COVERAGE] = 1.0;
    w
}

pub fn dominant_surface(w: &Weights) -> (usize, f32) {
    w[..SURFACE_COUNT].iter().copied().enumerate().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap_or((POWDER, 0.0))
}

pub fn pack_terrain_nibbles(attrs: &[f32; 8]) -> u32 {
    let q = ssx_assets::terrain_vertex::quantized_attributes(attrs);
    q.iter().enumerate().fold(0u32, |acc, (i, &x)| acc | (((x * 15.0).round() as u32) & 0xF) << (28 - 4 * i))
}

pub fn unpack_terrain_nibbles(packed: u32) -> [f32; 8] {
    std::array::from_fn(|i| ((packed >> (28 - 4 * i)) & 0xF) as f32 * (1.0 / 15.0))
}

pub fn decompose_terrain_weights(q: &[f32; 8]) -> Weights {
    let q5 = q[5];
    let mut m = [0.0; WEIGHT_COUNT];
    for k in 0..5 {
        m[k] = q[k] * q5;
    }
    m[POWDER] = (1.0 - (q[0] + q[1] + q[2] + q[3] + q[4]).clamp(0.0, 1.0)) * q5;
    m[ROCK] = (1.0 - q[6]) * (1.0 - q5);
    m[ICE] = q[6] * (1.0 - q5);
    m[AMBIENT_OCCLUSION] = q[7];
    m[SNOW_COVERAGE] = q5;
    m
}

pub fn terrain_contact_weights(nibbles: [u32; 3], bary: [f32; 3]) -> Weights {
    let mut out = [0.0; WEIGHT_COUNT];
    for (packed, b) in nibbles.into_iter().zip(bary) {
        let m = decompose_terrain_weights(&unpack_terrain_nibbles(packed));
        for (o, mk) in out.iter_mut().zip(m) {
            *o += b * mk;
        }
    }
    out
}

pub const OBJECT_MATERIAL_IDS: [u8; SURFACE_COUNT] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 10, 20, 30, 40, 50, 60];
pub const TERRAIN_SURFACE_ID: u32 = 0xFFFF;

pub fn object_contact_weights(surface_id: u32, normal_y: f32) -> Weights {
    let mut w = [0.0; WEIGHT_COUNT];
    match OBJECT_MATERIAL_IDS.iter().position(|&m| u32::from(m) == surface_id & 0xFE) {
        Some(i) => {
            let mut rest = 1.0;
            if surface_id & 1 != 0 {
                let cos60 = std::f32::consts::FRAC_PI_3.cos();
                let s = (normal_y / cos60).clamp(0.0, 1.0);
                w[POWDER] = s;
                w[SNOW_COVERAGE] = s;
                rest = 1.0 - s;
            }
            w[i] = rest;
        }
        None => {
            w[POWDER] = 1.0;
            w[SNOW_COVERAGE] = 1.0;
        }
    }
    w[AMBIENT_OCCLUSION] = ((surface_id >> 8) & 0xFF) as f32 * 0.003_921_569;
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibbles_round_trip_and_decompose() {
        let a = [0.0, 0.0, 0.0, 0.5, 0.0, 0.5, 0.0, 1.0];
        let q = unpack_terrain_nibbles(pack_terrain_nibbles(&a));
        assert_eq!(q, ssx_assets::terrain_vertex::quantized_attributes(&a));
        let m = decompose_terrain_weights(&q);
        assert!((m[3] - 7.0 / 15.0).abs() < 1e-6);
        assert!((m[POWDER] - 8.0 / 15.0).abs() < 1e-6);
        assert_eq!((m[ROCK], m[ICE], m[SNOW_COVERAGE], m[AMBIENT_OCCLUSION]), (0.0, 0.0, 1.0, 1.0));
        let m = decompose_terrain_weights(&unpack_terrain_nibbles(pack_terrain_nibbles(&[
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0,
        ])));
        assert_eq!((m[ROCK], m[ICE], m[POWDER]), (0.0, 1.0, 0.0));
    }

    #[test]
    fn barycentric_blend_of_vertex_weights() {
        let rock = pack_terrain_nibbles(&[0.0; 8]);
        let powder = pack_terrain_nibbles(&[0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let w = terrain_contact_weights([rock, powder, powder], [0.25, 0.5, 0.25]);
        assert!((w[ROCK] - 0.25).abs() < 1e-6 && (w[POWDER] - 0.75).abs() < 1e-6);
        assert!((w[SNOW_COVERAGE] - 0.75).abs() < 1e-6);
    }

    #[test]
    fn object_weights() {
        let w = object_contact_weights(10, 1.0);
        assert_eq!((w[ROCK], w[POWDER]), (1.0, 0.0));
        let w = object_contact_weights(31, 0.25);
        assert!((w[8] - 0.5).abs() < 1e-6 && (w[POWDER] - 0.5).abs() < 1e-6 && (w[SNOW_COVERAGE] - 0.5).abs() < 1e-6);
        let w = object_contact_weights(0, 1.0);
        assert_eq!(w, no_contact_weights());
    }

    #[test]
    fn dominant_surface_ignores_ao_and_coverage() {
        let mut w = [0.0; WEIGHT_COUNT];
        w[ROCK] = 0.6;
        w[POWDER] = 0.4;
        w[AMBIENT_OCCLUSION] = 1.0;
        w[SNOW_COVERAGE] = 0.9;
        assert_eq!(dominant_surface(&w), (ROCK, 0.6));
        assert_eq!(dominant_surface(&no_contact_weights()).0, POWDER);
    }
}
