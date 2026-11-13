use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RangeMovies {
    #[default]
    EveryTime,
    FirstTimeOnly,
}

impl RangeMovies {
    pub const ALL: [RangeMovies; 2] = [RangeMovies::EveryTime, RangeMovies::FirstTimeOnly];
    pub const MEMBER: &str = "range_movies";
    pub const FLAG: &str = "--range-movies";

    pub fn name(self) -> &'static str {
        match self {
            RangeMovies::EveryTime => "EveryTime",
            RangeMovies::FirstTimeOnly => "FirstTimeOnly",
        }
    }

    pub fn from_name(name: &str) -> Option<RangeMovies> {
        RangeMovies::ALL.into_iter().find(|r| r.name() == name)
    }

    pub fn arg(self) -> &'static str {
        match self {
            RangeMovies::EveryTime => "every-time",
            RangeMovies::FirstTimeOnly => "first-time-only",
        }
    }
}

impl fmt::Display for RangeMovies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.arg())
    }
}

impl FromStr for RangeMovies {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RangeMovies::ALL.into_iter().find(|r| r.arg() == s).ok_or_else(|| {
            let values: Vec<&str> = RangeMovies::ALL.iter().map(|r| r.arg()).collect();
            format!("{}: one of {}", ssx_core::text::quoted_max(s, 64), values.join(", "))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choices_read_back_by_name_and_by_flag_value() {
        assert_eq!(RangeMovies::default(), RangeMovies::EveryTime);
        for r in RangeMovies::ALL {
            assert_eq!(RangeMovies::from_name(r.name()), Some(r));
            assert_eq!(r.arg().parse::<RangeMovies>(), Ok(r));
            assert_eq!(r.to_string(), r.arg());
        }
        for bad in ["", "everytime", "firsttimeonly", "FIRST-TIME-ONLY", " every-time", "EveryTime", "first"] {
            assert!(bad.parse::<RangeMovies>().is_err(), "{bad:?}");
        }
        for bad in ["", "everyTime", "first-time-only", "FirstTimeOnly ", "Once"] {
            assert_eq!(RangeMovies::from_name(bad), None, "{bad:?}");
        }
        let e = "x\u{1b}[2J".repeat(100).parse::<RangeMovies>().unwrap_err();
        assert!(!e.contains('\u{1b}') && e.len() < 200 && e.ends_with("every-time, first-time-only"), "{e}");
    }
}
