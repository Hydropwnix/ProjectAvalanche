use std::sync::Mutex;

pub use super::store::Mt19937;

static GLOBAL: Mutex<Option<Mt19937>> = Mutex::new(None);

pub const TIME_BASE_HZ: u64 = 50_000_000;

// TODO(guess): a port choice with no game value behind it — the console's time base counts from power-on; the host's wall clock in 50 MHz ticks stands in (both unknown to the player); settle: none, the game's value cannot be reproduced.
pub fn time_base() -> u32 {
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    (ns * u128::from(TIME_BASE_HZ) / 1_000_000_000) as u32
}

pub fn boot(time_base: u32) -> Mt19937 {
    let mut local = Mt19937::seeded(time_base);
    let hi = local.below(0x1_0000) as u32;
    for _ in 0..=local.below(8) {
        local.next_u32();
    }
    let lo = local.below(0x1_0000) as u32;
    Mt19937::seeded(hi << 16 | lo)
}

pub fn with<R>(f: impl FnOnce(&mut Mt19937) -> R) -> R {
    let mut g = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    let g = g.get_or_insert_with(|| match std::env::var("SSX_RANDOM_SEED").ok().and_then(|s| s.parse().ok()) {
        Some(seed) => Mt19937::seeded(seed),
        None => boot(time_base()),
    });
    f(g)
}

pub fn reseed(seed: u32) {
    *GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Mt19937::seeded(seed));
}

pub fn below(n: i64) -> i64 {
    with(|g| g.below(n))
}

pub fn range(lo: f32, hi: f32) -> f32 {
    with(|g| g.range_f(lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_seeding_and_ranges() {
        let (mut a, mut b) = (Mt19937::seeded(4), Mt19937::seeded(5));
        assert!((0..100).all(|_| a.next_u32() == b.next_u32()));
        let mut g = Mt19937::seeded(1);
        for _ in 0..1000 {
            let x = g.range_f(0.0, 1.0);
            assert!((0.0..=1.0).contains(&x));
        }
        let (mut x, mut y) = (boot(11), boot(11));
        assert_eq!(x.next_u32(), y.next_u32());
        let (mut x, mut y) = (boot(11), boot(13));
        assert_ne!((x.next_u32(), x.next_u32()), (y.next_u32(), y.next_u32()));
    }
}
