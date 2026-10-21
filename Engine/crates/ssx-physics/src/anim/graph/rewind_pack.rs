pub const TRANSLATION_RANGE: f32 = 2.0;

pub fn pack_quaternion(q: [f32; 4]) -> u32 {
    let signs = q.iter().enumerate().fold(0u32, |acc, (i, c)| acc | (u32::from(*c < 0.0) << (10 + i)));
    let a = q.map(f32::abs);
    let len2 = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
    let inv = if len2 > 0.0 { 1.0 / len2.sqrt() } else { 0.0 };
    let len = len2 * inv;
    let n = |c: f32| (c * inv).min(1.0);
    let small = len < 0.001;
    let theta = if small { std::f32::consts::FRAC_PI_2 } else { n(a[1]).acos() };
    let tq = (theta * 325.312_7 + 1.525_878_9e-5).floor();
    let phi = if small { 0.0 } else { n(a[0]).asin() };
    let wq = (a[3] * 1023.0).min(1023.0) as u32;
    let k = (phi * 1.273_239_5 * tq + 1.525_878_9e-5).floor();
    let index = (tq * tq + k).floor() as u32;
    (index << 14) | signs | wq
}

pub fn unpack_quaternion(word: u32) -> [f32; 4] {
    let (index, wq) = (word >> 14, word & 1023);
    let w = wq as f32 * (1.0 / 1023.0);
    let len = (1.0 - w * w).max(0.0).sqrt();
    if len <= 0.001 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let tq = ((index as f32).sqrt() + 1.525_878_9e-5).floor();
    let k = (index as f32 - tq * tq).floor();
    let phi = if tq > 0.0 { k * (1.0 / tq) * std::f32::consts::FRAC_PI_4 } else { 0.0 };
    let x = phi.sin() * len;
    let y = (tq * 0.003_073_965).cos() * len;
    let z = (1.0 - (w * w + y * y + x * x)).max(0.0).sqrt();
    let sign = |bit: u32| if word >> bit & 1 != 0 { -1.0 } else { 1.0 };
    [x * sign(10), y * sign(11), z * sign(12), w * sign(13)]
}

pub fn pack_vector(v: [f32; 3], range: f32) -> u32 {
    let q = v.map(|c| ((c * (512.0 / range) + 512.0).max(0.0) as u32) & 1023);
    (q[0] << 20) | (q[1] << 10) | q[2]
}

pub fn unpack_vector(word: u32, range: f32) -> [f32; 3] {
    let k = range / 512.0;
    [word >> 20 & 1023, word >> 10 & 1023, word & 1023].map(|q| (q as f32 - 512.0) * k)
}

pub fn blend_quaternion(cur: u32, nb: u32, t: f32) -> u32 {
    let (b, a) = (unpack_quaternion(cur), unpack_quaternion(nb));
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let r: [f32; 4] =
        std::array::from_fn(|i| if dot > 0.0 { b[i] + t * (a[i] - b[i]) } else { b[i] - t * (a[i] + b[i]) });
    let l2: f32 = r.iter().map(|c| c * c).sum();
    let inv = 1.0 / l2.sqrt();
    pack_quaternion(r.map(|c| c * inv))
}

pub fn blend_vector(cur: u32, nb: u32, t: f32, range: f32) -> u32 {
    let (b, a) = (unpack_vector(cur, range), unpack_vector(nb, range));
    pack_vector(std::array::from_fn(|i| b[i] + t * (a[i] - b[i])), range)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_1_SQRT_2;

    #[test]
    fn the_words_are_the_games() {
        let n = |q: [f32; 4]| {
            let l = q.iter().map(|c| c * c).sum::<f32>().sqrt();
            q.map(|c| c / l)
        };
        let cases: [([f32; 4], u32, [f32; 4]); 6] = [
            ([0.0, 0.0, 0.0, 1.0], 0xFF00_43FF, [0.0, 0.0, 0.0, 1.0]),
            ([0.5, -0.5, 0.5, -0.5], 0x5E15_A9FF, [0.49849, -0.50185, 0.500142, -0.499511]),
            ([0.1, 0.7, -0.2, 0.678_233], 0x09C8_52B5, [0.097924, 0.701116, -0.199877, 0.677419]),
            ([-0.9, 0.1, 0.3, 0.291_547_6], 0xDDFE_C52B, [-0.902145, 0.102692, 0.300271, 0.292278]),
            ([0.0003, -0.0002, 0.0001, 0.999_999_94], 0xFF00_4BFE, [0.0, 0.0, 0.044205, 0.999022]),
            ([FRAC_1_SQRT_2, 0.0, 0.0, FRAC_1_SQRT_2], 0xFFFF_82D3, [0.707468, 0.0, 0.001119, 0.706745]),
        ];
        for (q, word, back) in cases {
            assert_eq!(pack_quaternion(n(q)), word, "{q:?}");
            let u = unpack_quaternion(word);
            assert!(u.iter().zip(back).all(|(a, b)| (a - b).abs() < 4e-5), "{u:?} vs {back:?}");
        }
        let random: [([f32; 4], u32); 24] = [
            ([-0.719_503_3, 0.221_944_29, 0.584_855_4, -0.301_662_03], 0xB867_2534),
            ([-0.742_343_96, -0.036_603_55, 0.267_009, 0.613_426_3], 0xF008_CE73),
            ([-0.617_829_1, -0.656_950_36, 0.358_707_04, 0.240_899_79], 0x46E5_4CF6),
            ([0.831_41, 0.532_555_6, -0.055_172_086, -0.148_653_8], 0x678D_B098),
            ([0.876_391_5, -0.098_463_65, -0.267_614_54, -0.388_104_86], 0xDE02_798D),
            ([0.244_030_65, 0.371_471_14, -0.881_594_96, -0.158_898_09], 0x90DE_F0A2),
            ([0.203_494_74, 0.515_940_55, 0.817_283, 0.156_344_67], 0x6BB9_C09F),
            ([-0.746_997_4, 0.292_877_44, -0.592_536_15, -0.071_544_014], 0xA7D0_B449),
            ([-0.877_012_5, 0.001_250_030_2, -0.463_301_2, 0.127_277_3], 0xFEB1_1482),
            ([0.903_089_9, -0.013_300_34, 0.230_491_89, -0.362_112_16], 0xFADD_A972),
            ([-0.352_576_64, 0.810_878_6, -0.113_427_48, 0.453_100_26], 0x12F0_15CF),
            ([0.073_061_12, -0.711_413_1, 0.398_609_85, -0.574_163_44], 0x1B94_EA4B),
            ([0.910_139_86, -0.245_655_39, 0.333_612_9, 0.001_142_275_2], 0xB52D_8801),
            ([0.630_376_4, -0.398_539_84, 0.661_110_46, -0.082_003_31], 0x8B1E_6853),
            ([0.781_313_24, 0.386_043_94, 0.087_385_66, -0.482_579_98], 0x8077_E1ED),
            ([0.402_070_6, 0.245_154_47, 0.859_801_1, -0.197_435_07], 0xB31D_A0C9),
            ([0.668_201_15, 0.611_800_4, 0.340_837_24, 0.251_072_73], 0x5145_C100),
            ([0.145_000_59, -0.476_423_62, -0.807_686_3, -0.315_655_08], 0x704A_F942),
            ([0.272_616_18, 0.697_295_8, 0.450_853_17, 0.485_994_2], 0x2B26_01F1),
            ([0.592_524_05, 0.532_442_3, 0.139_766_87, 0.588_120_46], 0x4B36_8259),
            ([0.644_375, -0.119_027_03, -0.252_057_94, 0.712_095_6], 0xCAD4_9AD8),
            ([0.139_967_04, 0.555_413_5, 0.499_866_66, -0.649_660_2], 0x3A32_6298),
            ([0.782_437_4, 0.570_977_2, 0.056_893_222, 0.241_950_21], 0x5BCC_40F7),
            ([0.130_185_62, -0.906_981_2, -0.114_946_39, 0.383_697_87], 0x03A4_D988),
        ];
        for (q, word) in random {
            assert_eq!(pack_quaternion(q), word, "{q:?}");
        }
        let vectors: [([f32; 3], u32, [f32; 3]); 3] = [
            ([0.0, 0.9, -0.05], 0x200B_99F3, [0.0, 0.898_437_5, -0.050_781_25]),
            ([1.999, -2.0, 0.0039], 0x3FF0_0200, [1.996_093_8, -2.0, 0.0]),
            ([-0.3, 2.5, -2.5], 0x1B32_0000, [-0.300_781_25, -1.5, -2.0]),
        ];
        for (v, word, back) in vectors {
            assert_eq!(pack_vector(v, TRANSLATION_RANGE), word, "{v:?}");
            assert_eq!(unpack_vector(word, TRANSLATION_RANGE), back);
        }
    }

    #[test]
    fn the_playback_blends_towards_the_neighbour() {
        let (a, b) = (pack_quaternion([0.0, 0.0, 0.0, 1.0]), pack_quaternion([0.0, 0.382_683_4, 0.0, 0.923_879_5]));
        assert_eq!(blend_quaternion(a, b, 0.0), pack_quaternion(unpack_quaternion(a)));
        let half = unpack_quaternion(blend_quaternion(a, b, 0.5));
        assert!((half[1] - 0.195_090_3).abs() < 2e-3 && half[3] > 0.97, "{half:?}");
        let flipped = pack_quaternion([0.0, -0.382_683_4, 0.0, -0.923_879_5]);
        let h2 = unpack_quaternion(blend_quaternion(a, flipped, 0.5));
        assert!(half.iter().zip(h2).all(|(x, y)| (x - y).abs() < 2e-3), "{half:?} {h2:?}");
        let (u, v) = (pack_vector([0.0, 1.0, 0.0], 2.0), pack_vector([1.0, 1.0, -1.0], 2.0));
        assert_eq!(unpack_vector(blend_vector(u, v, 0.5, 2.0), 2.0), [0.5, 1.0, -0.5]);
    }
}
