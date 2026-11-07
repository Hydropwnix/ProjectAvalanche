use core::num::NonZeroU64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Expiry {
    Never,
    At(NonZeroU64),
}

impl Expiry {
    pub fn to_wire(self) -> u64 {
        match self {
            Self::Never => 0,
            Self::At(t) => t.get(),
        }
    }

    pub fn from_wire(value: u64) -> Self {
        NonZeroU64::new(value).map_or(Self::Never, Self::At)
    }

    pub fn passed(self, now: u64) -> bool {
        match self {
            Self::Never => false,
            Self::At(t) => now >= t.get(),
        }
    }

    pub fn left(self, now: u64) -> Option<u64> {
        match self {
            Self::Never => None,
            Self::At(t) => Some(t.get().saturating_sub(now)),
        }
    }

    pub fn after(now: u64, seconds: Option<u64>) -> Self {
        match seconds {
            None => Self::Never,
            Some(s) => Self::At(NonZeroU64::new(now.saturating_add(s).max(1)).unwrap_or(NonZeroU64::MIN)),
        }
    }

    pub fn describe(self) -> String {
        match self {
            Self::Never => "never".into(),
            Self::At(t) => utc(t.get()),
        }
    }
}

pub fn utc(secs: u64) -> String {
    let days = secs / 86_400;
    let rest = secs % 86_400;
    let (h, m) = (rest / 3600, rest % 3600 / 60);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02} UTC")
}

pub fn span(secs: u64) -> String {
    let units = [(86_400, "d"), (3600, "h"), (60, "min"), (1, "s")];
    let mut out = Vec::new();
    let mut rest = secs;
    for (size, name) in units {
        if rest >= size && out.len() < 2 {
            out.push(format!("{} {name}", rest / size));
            rest %= size;
        } else if !out.is_empty() {
            break;
        }
    }
    if out.is_empty() { "0 s".into() } else { out.join(" ") }
}

pub fn ban_end(until: Expiry, now: u64) -> String {
    match until {
        Expiry::Never => "banned for good".into(),
        Expiry::At(t) if now >= t.get() => format!("banned until {} (the ban has ended)", utc(t.get())),
        Expiry::At(t) => format!("banned until {}, {} from now", utc(t.get()), span(t.get() - now)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bans_end_as_the_player_reads_it() {
        assert_eq!(ban_end(Expiry::Never, 5), "banned for good");
        let at = Expiry::after(1_794_076_500, Some(0)).to_wire();
        assert_eq!(ban_end(Expiry::from_wire(at), at - 90_000), "banned until 2026-11-07 18:35 UTC, 1 d 1 h from now");
        assert_eq!(ban_end(Expiry::from_wire(at), at), "banned until 2026-11-07 18:35 UTC (the ban has ended)");
    }

    #[test]
    fn the_wire_and_the_clock() {
        assert_eq!(Expiry::from_wire(0), Expiry::Never);
        assert_eq!(Expiry::from_wire(5).to_wire(), 5);
        assert_eq!(Expiry::Never.to_wire(), 0);
        let at = Expiry::after(100, Some(60));
        assert_eq!(at.to_wire(), 160);
        assert!(!at.passed(159) && at.passed(160) && at.passed(u64::MAX));
        assert_eq!(at.left(150), Some(10));
        assert_eq!(at.left(200), Some(0));
        assert!(!Expiry::Never.passed(u64::MAX));
        assert_eq!(Expiry::after(u64::MAX, Some(10)).to_wire(), u64::MAX, "saturates");
        assert_eq!(Expiry::after(0, Some(0)).to_wire(), 1, "never 0 (that is never on the wire)");
    }

    #[test]
    fn dates_and_spans() {
        assert_eq!(utc(0), "1970-01-01 00:00 UTC");
        assert_eq!(utc(1_794_076_500), "2026-11-07 18:35 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00 UTC");
        assert!(utc(u64::MAX).ends_with("UTC"), "no panic at the end of the range");
        assert_eq!(span(0), "0 s");
        assert_eq!(span(40), "40 s");
        assert_eq!(span(3600 * 26 + 120), "1 d 2 h");
        assert_eq!(span(7260), "2 h 1 min");
    }
}
