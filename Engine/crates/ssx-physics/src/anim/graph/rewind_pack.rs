use ssx_formats::ghost;

pub const TRANSLATION_RANGE: f32 = ghost::CHANNEL_RANGE;

pub fn blend_quaternion(cur: u32, nb: u32, t: f32) -> u32 {
    ghost::pack_quaternion(ghost::blend_rotation(ghost::unpack_quaternion(cur), ghost::unpack_quaternion(nb), t))
}

pub fn blend_vector(cur: u32, nb: u32, t: f32, range: f32) -> u32 {
    let unpack = |w: u32| ghost::unpack_vector(w, [range; 3]);
    ghost::pack_vector(ghost::blend_lerp(unpack(cur), unpack(nb), t), [range; 3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssx_formats::ghost::{pack_quaternion, pack_vector, unpack_quaternion, unpack_vector};

    #[test]
    fn the_playback_blends_towards_the_neighbour() {
        let (a, b) = (pack_quaternion([0.0, 0.0, 0.0, 1.0]), pack_quaternion([0.0, 0.382_683_4, 0.0, 0.923_879_5]));
        assert_eq!(blend_quaternion(a, b, 0.0), pack_quaternion(unpack_quaternion(a)));
        let half = unpack_quaternion(blend_quaternion(a, b, 0.5));
        assert!((half[1] - 0.195_090_3).abs() < 2e-3 && half[3] > 0.97, "{half:?}");
        let flipped = pack_quaternion([0.0, -0.382_683_4, 0.0, -0.923_879_5]);
        let h2 = unpack_quaternion(blend_quaternion(a, flipped, 0.5));
        assert!(half.iter().zip(h2).all(|(x, y)| (x - y).abs() < 2e-3), "{half:?} {h2:?}");
        let (u, v) = (pack_vector([0.0, 1.0, 0.0], [2.0; 3]), pack_vector([1.0, 1.0, -1.0], [2.0; 3]));
        assert_eq!(unpack_vector(blend_vector(u, v, 0.5, 2.0), [2.0; 3]), [0.5, 1.0, -0.5]);
    }
}
