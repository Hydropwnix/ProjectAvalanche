pub type Rgb = [f32; 3];

const fn rgb(hex: u32) -> Rgb {
    [(hex >> 16 & 255) as f32 / 255.0, (hex >> 8 & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

#[derive(Clone, Copy, Debug)]
pub struct Range {
    pub base: f32,
    pub rise: f32,
    pub spacing: f32,
    pub depth: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub seed: u32,
    pub sky: [Rgb; 3],
    pub horizon: f32,
    pub sun: (f32, f32, f32),
    pub glow: Rgb,
    pub snow: Rgb,
    pub shade: Rgb,
    pub rock: Rgb,
    pub haze: Rgb,
    pub ranges: &'static [Range],
}

const fn range(base: f32, rise: f32, spacing: f32, depth: f32) -> Range {
    Range { base, rise, spacing, depth }
}

pub const BACKDROP: Scene = Scene {
    seed: 2012,
    sky: [rgb(0x04_1A_2E), rgb(0x12_58_7C), rgb(0x9C_D8_E8)],
    horizon: 0.66,
    sun: (0.80, 0.42, 0.40),
    glow: rgb(0xF4_FC_FF),
    snow: rgb(0xF2_FA_FD),
    shade: rgb(0x3E_7C_A2),
    rock: rgb(0x0C_26_3A),
    haze: rgb(0x8E_CC_DF),
    ranges: &[
        range(0.68, 0.16, 0.34, 0.0),
        range(0.76, 0.24, 0.48, 0.38),
        range(0.90, 0.30, 0.66, 0.72),
        range(1.08, 0.30, 0.90, 1.0),
    ],
};

pub const HERO: Scene = Scene {
    seed: 77,
    sky: [rgb(0x08_34_56), rgb(0x2A_8C_B4), rgb(0xD4_F1_F7)],
    horizon: 0.62,
    sun: (0.72, 0.36, 0.55),
    glow: rgb(0xFF_FF_FA),
    snow: rgb(0xF6_FC_FE),
    shade: rgb(0x4C_8E_B4),
    rock: rgb(0x10_30_48),
    haze: rgb(0xB4_E2_EE),
    ranges: &[range(0.62, 0.22, 0.30, 0.0), range(0.78, 0.36, 0.46, 0.45), range(1.04, 0.50, 0.70, 1.0)],
};

const TILE_RANGES: [Range; 3] =
    [range(0.40, 0.14, 0.26, 0.0), range(0.50, 0.22, 0.40, 0.5), range(0.66, 0.30, 0.62, 1.0)];

pub const TILES: [Scene; 3] = [
    Scene { seed: 11, sun: (0.27, 0.20, 0.50), horizon: 0.44, ranges: &TILE_RANGES, ..HERO },
    Scene {
        seed: 23,
        sky: [rgb(0x05_26_3A), rgb(0x16_66_78), rgb(0x86_D4_D2)],
        sun: (0.22, 0.22, 0.42),
        horizon: 0.44,
        shade: rgb(0x2F_7A_86),
        rock: rgb(0x0A_2A_34),
        haze: rgb(0x7C_C8_C8),
        ranges: &TILE_RANGES,
        ..HERO
    },
    Scene {
        seed: 42,
        sky: [rgb(0x03_0E_22), rgb(0x0A_2C_58), rgb(0x3A_78_B4)],
        sun: (0.34, 0.16, 0.26),
        glow: rgb(0xCC_E6_FF),
        horizon: 0.44,
        snow: rgb(0xC2_DC_F4),
        shade: rgb(0x22_4E_84),
        rock: rgb(0x06_18_30),
        haze: rgb(0x34_68_A0),
        ranges: &TILE_RANGES,
    },
];
