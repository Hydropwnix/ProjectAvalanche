use crate::vmx;

const RING_STEP: f32 = f32::from_bits(0x3B49_7495);
const POLAR_SCALE: f32 = f32::from_bits(0x43A2_A806);
const AZIMUTH_SCALE: f32 = f32::from_bits(0x3FA2_F983);
const INV_1023: f32 = f32::from_bits(0x3A80_2008);
const HALF_PI: f32 = f32::from_bits(0x3FC9_0FDB);
const QUARTER_PI: f32 = f32::from_bits(0x3F49_0FDB);
const RING_EPS: f32 = f32::from_bits(0x3780_0000);
const MIN_AXIS: f32 = f32::from_bits(0x3A83_126F);
const CODE_CENTRE: f32 = 512.0;

pub fn pack_half(x: f32) -> u16 {
    let w = x.to_bits();
    let sign = ((w >> 16) & 0x8000) as u16;
    let e = ((w >> 23) & 0xFF) as i32 - 0x70;
    if e <= 0 {
        sign
    } else if e >= 31 {
        sign | 0x7C00
    } else {
        sign | ((e as u16) << 10) | ((w >> 13) & 0x3FF) as u16
    }
}

pub fn unpack_half(h: u16) -> f32 {
    let h = u32::from(h);
    let sign = (h << 16) & 0x8000_0000;
    let e = (h >> 10) & 0x1F;
    if e == 0 { f32::from_bits(sign) } else { f32::from_bits(((e + 0x70) << 23) | ((h << 13) & 0x007F_E000) | sign) }
}

pub fn pack_vector(v: [f32; 3], range: [f32; 3]) -> u32 {
    let code = |i: usize| {
        let scale = vmx::mul(vmx::recip(range[i]), CODE_CENTRE);
        vmx::to_u32(vmx::madd(v[i], scale, CODE_CENTRE)) & 1023
    };
    (code(0) << 20) | (code(1) << 10) | code(2)
}

pub fn unpack_vector(w: u32, range: [f32; 3]) -> [f32; 3] {
    let codes = [(w >> 20) & 1023, (w >> 10) & 1023, w & 1023];
    let step = vmx::recip(CODE_CENTRE);
    std::array::from_fn(|i| vmx::mul(vmx::sub(vmx::from_u32(codes[i]), CODE_CENTRE), vmx::mul(step, range[i])))
}

pub fn vector_length(v: [f32; 3]) -> f32 {
    let dot = vmx::dot3(v, v);
    if vmx::eq(dot, 0.0) { 0.0 } else { vmx::mul(dot, vmx::rsqrt(dot)) }
}

pub fn pack_quaternion(q: [f32; 4]) -> u32 {
    let signs = (0..4).fold(0u32, |acc, i| acc | (u32::from(vmx::gt(0.0, q[i])) << (10 + i)));
    let a = q.map(|c| f32::from_bits(c.to_bits() & 0x7FFF_FFFF));
    let xyz = [a[0], a[1], a[2]];
    let dot = vmx::dot3(xyz, xyz);
    let w1023 = vmx::mul(a[3], 1023.0);
    let inv = vmx::rsqrt(dot);
    let length = if vmx::eq(dot, 0.0) { 0.0 } else { vmx::mul(dot, inv) };
    let axis = xyz.map(|c| {
        let n = vmx::mul(c, inv);
        if vmx::in_bounds(n, 1.0) { n } else { 1.0f32.copysign(n) }
    });
    let short = vmx::gt(MIN_AXIS, length);
    let polar = if short { HALF_PI } else { vmx::acos(axis[1]) };
    let r = vmx::floor(vmx::madd(polar, POLAR_SCALE, RING_EPS));
    let azimuth = if short { 0.0 } else { vmx::asin(axis[0]) };
    let k = vmx::floor(vmx::madd(vmx::mul(azimuth, AZIMUTH_SCALE), r, RING_EPS));
    let index = vmx::to_u32(vmx::floor(vmx::madd(r, r, k)));
    let w_code = if vmx::gt(w1023, 1023.0) { 1023.0 } else { w1023 };
    (index << 14) | signs | vmx::to_u32(w_code)
}

pub fn unpack_quaternion(w: u32) -> [f32; 4] {
    let n = vmx::from_u32(w >> 14);
    let a = vmx::mul(vmx::from_u32(w & 1023), INV_1023);
    let root = |x: f32| if vmx::eq(x, 0.0) { 0.0 } else { vmx::mul(x, vmx::rsqrt(x)) };
    let s = root(vmx::sub(1.0, vmx::mul(a, a)));
    let r = vmx::floor(vmx::add(root(n), RING_EPS));
    let k = vmx::floor(vmx::sub(n, vmx::mul(r, r)));
    let phi = if vmx::gt(r, 0.0) { vmx::mul(vmx::mul(k, vmx::recip(r)), QUARTER_PI) } else { 0.0 };
    let x = vmx::mul(vmx::sin(phi), s);
    let y = vmx::mul(vmx::cos(vmx::mul(r, RING_STEP)), s);
    let t = vmx::madd(a, a, vmx::madd(y, y, vmx::mul(x, x)));
    let z = root(vmx::max(vmx::sub(1.0, t), 0.0));
    if vmx::ge(MIN_AXIS, s) {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let signed = |v: f32, bit: u32| f32::from_bits(v.to_bits() | (((w >> bit) & 1) << 31));
    [signed(x, 10), signed(y, 11), signed(z, 12), signed(a, 13)]
}

pub fn blend_rotation(cur: [f32; 4], nb: [f32; 4], t: f32) -> [f32; 4] {
    let towards = vmx::gt(vmx::dot4(cur, nb), 0.0);
    let r: [f32; 4] = std::array::from_fn(|i| {
        if towards {
            vmx::madd(vmx::sub(nb[i], cur[i]), t, cur[i])
        } else {
            vmx::nmsub(vmx::add(nb[i], cur[i]), t, cur[i])
        }
    });
    let inv = vmx::rsqrt(vmx::dot4(r, r));
    r.map(|c| vmx::mul(c, inv))
}

pub fn blend_lerp(cur: [f32; 3], nb: [f32; 3], t: f32) -> [f32; 3] {
    std::array::from_fn(|i| vmx::madd(vmx::sub(nb[i], cur[i]), t, cur[i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_1_SQRT_2;

    #[test]
    fn halves() {
        assert_eq!(pack_half(1.0), 0x3C00);
        assert_eq!(pack_half(-2.0), 0xC000);
        assert_eq!(pack_half(f32::from_bits(0x3F80_1FFF)), 0x3C00, "truncated, not rounded");
        assert_eq!(pack_half(2f32.powi(-14)), 0x0400);
        assert_eq!(pack_half(2f32.powi(-15)), 0x0000);
        assert_eq!(pack_half(-1e-6), 0x8000);
        assert_eq!(pack_half(65_504.0), 0x7BFF);
        assert_eq!(pack_half(65_536.0), 0x7C00);
        assert_eq!(pack_half(f32::NEG_INFINITY), 0xFC00);
        assert_eq!(pack_half(f32::NAN), 0x7C00);
        assert_eq!(pack_half(f32::from_bits(0xFFC0_0000)), 0xFC00);
        assert_eq!(unpack_half(0x3C00), 1.0);
        assert_eq!(unpack_half(0x2FA0), 0.119_140_625);
        assert_eq!(unpack_half(0x7C00), 65_536.0);
        assert_eq!(unpack_half(0x7FFF), 131_008.0);
        assert_eq!(unpack_half(0x0001).to_bits(), 0);
        assert_eq!(unpack_half(0x8200).to_bits(), 0x8000_0000);
        for h in 0..=u16::MAX {
            let e = (h >> 10) & 0x1F;
            if (e != 0 || h & 0x3FF == 0) && (e != 31 || h & 0x3FF == 0) {
                assert_eq!(pack_half(unpack_half(h)), h, "{h:#06x}");
            }
        }
    }

    #[test]
    fn vector_codes() {
        let w = (512 << 20) | (768 << 10) | 256;
        assert_eq!(unpack_vector(w, [2.0; 3]), [0.0, 1.0, -1.0]);
        assert_eq!(pack_vector([0.0; 3], [2.0; 3]), 0x2008_0200);
        assert_eq!(pack_vector([-2.0, 0.0, 2.0], [2.0; 3]), 0x0008_0000);
        assert_eq!(pack_vector([-0.3, 2.5, -2.5], [2.0; 3]), 0x1B32_0000);
        assert_eq!(pack_vector([1.999, -2.0, 0.0039], [2.0; 3]), 0x3FF0_0200);
        for c in 0..1024u32 {
            let w = (c << 20) | ((1023 - c) << 10) | c;
            assert_eq!(pack_vector(unpack_vector(w, [2.0; 3]), [2.0; 3]), w);
        }
    }

    #[test]
    fn quaternion_codes() {
        assert_eq!(unpack_quaternion(1023), [0.0, 0.0, 0.0, 1.0]);
        let a = (std::f32::consts::FRAC_1_SQRT_2 * 1023.0) as u32;
        let q = unpack_quaternion(a);
        assert!((q[1] - (1.0 - (a as f32 / 1023.0).powi(2)).sqrt()).abs() < 1e-6 && q[0] == 0.0 && q[2] == 0.0);
        let neg = unpack_quaternion(a | (1 << 11) | (1 << 13));
        assert_eq!(neg, [q[0], -q[1], q[2], -q[3]]);
        let q = unpack_quaternion(((511 * 511) << 14) | a);
        assert!(q[1].abs() < 0.01 && (q[2] - q[3]).abs() < 0.01);
        assert_eq!(pack_quaternion([0.0, 0.0, 0.0, 1.0]), 0xFF00_43FF);
        assert_eq!(pack_quaternion([0.0, 0.0, 0.0, -1.0]), 0xFF00_63FF);
        assert_eq!(pack_quaternion([-0.0, 0.0, 0.0, 1.0]), 0xFF00_43FF, "−0 sets no sign");
        assert_eq!(pack_quaternion([0.0, 0.0, 0.0, 1.5]) & 1023, 1023);
    }

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
            assert_eq!(pack_vector(v, [2.0; 3]), word, "{v:?}");
            assert_eq!(unpack_vector(word, [2.0; 3]), back);
        }
    }

    #[test]
    fn the_bits_of_the_games_code() {
        let f = f32::from_bits;
        type Trig = (fn(f32) -> f32, [(u32, u32); 3]);
        let trig: [Trig; 4] = [
            (vmx::asin, [(0x3EB8_1030, 0x3EBC_472A), (0x3E22_610C, 0x3E23_1146), (0x3F35_3027, 0x3F49_4CFC)]),
            (vmx::acos, [(0xBF5E_CBA3, 0x4028_1A2A), (0xBF3C_1678, 0x4019_58D6), (0x0000_0000, 0x3FC9_0FDB)]),
            (vmx::sin, [(0xBFB5_665C, 0xBF7C_FC53), (0x3F4F_2B0B, 0x3F39_4901), (0x3E25_DD51, 0x3E25_23E3)]),
            (vmx::cos, [(0xC053_E5AA, 0xBF7C_5706), (0x404F_3670, 0xBF7E_D193), (0xBFE9_CA7A, 0xBE81_7E7C)]),
        ];
        for (func, cases) in trig {
            for (x, y) in cases {
                assert_eq!(func(f(x)).to_bits(), y, "{x:#010x}");
            }
        }
        let v = |w: [u32; 3]| w.map(f);
        assert_eq!(pack_vector(v([0x3F83_3824, 0xBE4B_D763, 0xBF8F_F7D2]), [f(0x3F8D_58F6); 3]), 0x3DB6_8C00);
        assert_eq!(pack_vector(v([0xBF84_CA71, 0x3EB6_5DC5, 0xBF89_85AA]), [1.5; 3]), 0x09D9_E491);
        assert_eq!(pack_vector(v([0xBF33_CC93, 0x3F59_A8F9, 0x3F96_2233]), [f(0x3F93_2111); 3]), 0x0C7D_E80A);
        let bits = |a: [f32; 3]| a.map(f32::to_bits);
        assert_eq!(bits(unpack_vector(0x1057_9A2E, [f(0x3F8D_58F6); 3])), [0xBF0A_9639, 0xBD65_B090, 0x3DCB_2FE2]);
        assert_eq!(bits(unpack_vector(0xA4AC_B5E0, [f(0x3F93_2111); 3])), [0x3E2A_1E3C, 0x3F2C_FDE1, 0xBD93_2111]);
        assert_eq!(pack_quaternion([1.0, 0.0, 0.0, 0.0]), 0xFFFF_8000);
        assert_eq!(pack_quaternion([0x3F20_624D, 0x3F1C_2935, 0xBE53_5573, 0xBEE0_D070].map(f)), 0x4665_F1C1);
        assert_eq!(pack_quaternion([0x3F40_525B, 0x3F18_7732, 0x3E90_A73A, 0xBD07_4140].map(f)), 0x59FA_6021);
        let q = |w: u32| unpack_quaternion(w).map(f32::to_bits);
        assert_eq!(q(0x4032_CEB4), [0xBEDB_F520, 0xBF05_1D6D, 0x3E97_70B9, 0x3F2D_2B4B]);
        assert_eq!(q(0x2311_4450), [0xBF33_B6A7, 0x3F55_5771, 0x0000_0000, 0x3DA0_280A]);
        assert_eq!(q(0xFF00_63FF), [0, 0, 0, 0x3F80_0000]);
    }
}
