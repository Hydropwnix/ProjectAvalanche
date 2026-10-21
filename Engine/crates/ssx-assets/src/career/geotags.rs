use ssx_formats::csvdb::Geotag;

use super::profile::Profile;

pub const RIDE_GEOTAGS: usize = 25;

pub fn pickup_value(level: u32, quality: u32) -> i64 {
    let pow = 2f64.powf(f64::from(level as f32)) as f32;
    (pow as i32 as i64) * (i64::from(quality) + 1) * 100
}

pub fn geotag_key(track: u32, id: u32) -> u64 {
    (u64::from(id) << 32).wrapping_add(u64::from(track).wrapping_mul(10_000_000_000))
}

impl Profile {
    pub fn collect_geotag(&mut self, track: u32, g: &Geotag) -> Option<i64> {
        let key = geotag_key(track, g.id);
        if !self.collected_geotags.insert(key) {
            return None;
        }
        let value = pickup_value(g.level, g.quality).max(-self.credits());
        self.add_credits(value, true);
        Some(value)
    }

    pub fn geotag_placed(&self, track: u32, id: u32) -> bool {
        !self.collected_geotags.contains(&geotag_key(track, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_double_per_level_and_scale_with_quality() {
        assert_eq!(pickup_value(1, 0), 200);
        assert_eq!(pickup_value(5, 1), 6400);
        assert_eq!(pickup_value(11, 4), 1_024_000);
        assert_eq!(geotag_key(0, 7), 7 << 32);
        assert_eq!(geotag_key(1, 0), 10_000_000_000);
    }

    #[test]
    fn a_geotag_pays_once() {
        let mut p = Profile::new();
        let g = Geotag { id: 3, quality: 2, level: 4, position: [0.0; 3] };
        assert_eq!(p.collect_geotag(9, &g), Some(4800));
        assert_eq!((p.credits(), p.earned()), (4500 + 4800, 4800));
        assert_eq!(p.collect_geotag(9, &g), None);
        assert!(!p.geotag_placed(9, 3) && p.geotag_placed(9, 4));
    }
}
