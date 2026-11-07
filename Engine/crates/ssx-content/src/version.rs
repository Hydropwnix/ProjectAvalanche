use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    #[error("bad version {} (expected MAJOR.MINOR.PATCH, digits only)", ssx_core::text::quoted(.0))]
    Bad(String),
    #[error(
        "bad version requirement {} (comparators `>=`, `>`, `<=`, `<`, `=` separated by commas)",
        ssx_core::text::quoted(.0)
    )]
    BadReq(String),
    #[error("bad dependency {} (expected `<package> [requirement]`)", ssx_core::text::quoted(.0))]
    BadDependency(String),
}

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Version { major, minor, patch }
    }

    pub fn parse_partial(s: &str) -> Result<Self, VersionError> {
        let bad = || VersionError::Bad(s.to_string());
        let parts: Vec<&str> = s.split('.').collect();
        if parts.is_empty() || parts.len() > 3 {
            return Err(bad());
        }
        let mut n = [0u64; 3];
        for (slot, p) in n.iter_mut().zip(&parts) {
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || (p.len() > 1 && p.starts_with('0')) {
                return Err(bad());
            }
            *slot = p.parse().map_err(|_| bad())?;
        }
        Ok(Version::new(n[0], n[1], n[2]))
    }
}

impl FromStr for Version {
    type Err = VersionError;
    fn from_str(s: &str) -> Result<Self, VersionError> {
        if s.split('.').count() != 3 {
            return Err(VersionError::Bad(s.to_string()));
        }
        Version::parse_partial(s)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Ge,
    Gt,
    Le,
    Lt,
    Eq,
}

impl Op {
    const fn as_str(self) -> &'static str {
        match self {
            Op::Ge => ">=",
            Op::Gt => ">",
            Op::Le => "<=",
            Op::Lt => "<",
            Op::Eq => "=",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VersionReq {
    pub comparators: Vec<(Op, Version)>,
}

impl VersionReq {
    pub fn matches(&self, v: Version) -> bool {
        self.comparators.iter().all(|&(op, c)| match op {
            Op::Ge => v >= c,
            Op::Gt => v > c,
            Op::Le => v <= c,
            Op::Lt => v < c,
            Op::Eq => v == c,
        })
    }
}

impl FromStr for VersionReq {
    type Err = VersionError;
    fn from_str(s: &str) -> Result<Self, VersionError> {
        let bad = || VersionError::BadReq(s.to_string());
        let t = s.trim();
        if t.is_empty() || t == "*" {
            return Ok(VersionReq::default());
        }
        let mut comparators = Vec::new();
        for part in t.split(',') {
            let part = part.trim();
            let (op, rest) = [Op::Ge, Op::Le, Op::Gt, Op::Lt, Op::Eq]
                .into_iter()
                .find_map(|op| part.strip_prefix(op.as_str()).map(|r| (op, r)))
                .ok_or_else(bad)?;
            comparators.push((op, Version::parse_partial(rest.trim()).map_err(|_| bad())?));
        }
        Ok(VersionReq { comparators })
    }
}

impl fmt::Display for VersionReq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.comparators.is_empty() {
            return f.write_str("*");
        }
        let parts: Vec<String> = self.comparators.iter().map(|(op, v)| format!("{} {v}", op.as_str())).collect();
        f.write_str(&parts.join(", "))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    pub name: String,
    pub req: VersionReq,
}

impl FromStr for Dependency {
    type Err = VersionError;
    fn from_str(s: &str) -> Result<Self, VersionError> {
        let t = s.trim();
        let end = t.find(|c: char| !crate::id::segment_char(c)).unwrap_or(t.len());
        let (name, rest) = t.split_at(end);
        if !crate::id::valid_namespace(name) {
            return Err(VersionError::BadDependency(s.to_string()));
        }
        Ok(Dependency { name: name.to_string(), req: rest.parse()? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!("1.0.0".parse::<Version>().unwrap(), Version::new(1, 0, 0));
        assert_eq!("10.20.30".parse::<Version>().unwrap(), Version::new(10, 20, 30));
        for bad in ["1.0", "1", "1.0.0.0", "01.0.0", "1.x.0", "", "1.0.0-beta", " 1.0.0"] {
            assert!(bad.parse::<Version>().is_err(), "{bad}");
        }
        assert_eq!(Version::parse_partial("2").unwrap(), Version::new(2, 0, 0));
        assert!(Version::new(1, 10, 0) > Version::new(1, 9, 9));
    }

    #[test]
    fn ranges() {
        let r: VersionReq = ">= 1.0, < 2".parse().unwrap();
        assert_eq!(r.to_string(), ">= 1.0.0, < 2.0.0");
        assert!(r.matches(Version::new(1, 0, 0)) && r.matches(Version::new(1, 99, 3)));
        assert!(!r.matches(Version::new(2, 0, 0)) && !r.matches(Version::new(0, 9, 9)));
        let eq: VersionReq = "=1.2.3".parse().unwrap();
        assert!(eq.matches(Version::new(1, 2, 3)) && !eq.matches(Version::new(1, 2, 4)));
        assert!(">1".parse::<VersionReq>().unwrap().matches(Version::new(1, 0, 1)));
        assert!("<=1.2".parse::<VersionReq>().unwrap().matches(Version::new(1, 2, 0)));
        assert!("".parse::<VersionReq>().unwrap().matches(Version::new(7, 0, 0)));
        assert!("*".parse::<VersionReq>().unwrap().comparators.is_empty());
        for bad in ["1.0", "~1.0", "^1", ">= 1.0,", ">= a", "=> 1", ">= 1.0 < 2"] {
            assert!(bad.parse::<VersionReq>().is_err(), "{bad}");
        }
    }

    #[test]
    fn dependencies() {
        let d: Dependency = "mapmanager >= 1.0, < 2".parse().unwrap();
        assert_eq!(d.name, "mapmanager");
        assert_eq!(d.req.comparators.len(), 2);
        let d: Dependency = "neon-pack".parse().unwrap();
        assert!(d.req.comparators.is_empty());
        let d: Dependency = "lib>=2".parse().unwrap();
        assert_eq!((d.name.as_str(), d.req.comparators[0]), ("lib", (Op::Ge, Version::new(2, 0, 0))));
        for bad in ["", ">= 1", "Map >= 1", "map ~1"] {
            assert!(bad.parse::<Dependency>().is_err(), "{bad}");
        }
    }
}
