#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub first: u64,
    pub len: u64,
}

impl ByteRange {
    pub fn last(&self) -> u64 {
        self.first + self.len - 1
    }

    pub fn content_range(&self, size: u64) -> String {
        format!("bytes {}-{}/{size}", self.first, self.last())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    Unreadable,
    Unsatisfiable,
}

pub const MAX_RANGE_HEADER: usize = 64;
pub const MAX_CONTENT_RANGE: usize = 6 + 20 + 1 + 20 + 1 + 20;

fn number(s: &str) -> Option<u64> {
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) || (s.len() > 1 && s.starts_with('0')) {
        return None;
    }
    s.parse().ok()
}

pub fn parse_range(value: &[u8], size: u64) -> Result<ByteRange, RangeError> {
    if value.len() > MAX_RANGE_HEADER {
        return Err(RangeError::Unreadable);
    }
    let text = std::str::from_utf8(value).map_err(|_| RangeError::Unreadable)?;
    let (unit, spec) = text.split_once('=').ok_or(RangeError::Unreadable)?;
    if !unit.eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return Err(RangeError::Unreadable);
    }
    let (a, b) = spec.split_once('-').ok_or(RangeError::Unreadable)?;
    match (a.is_empty(), b.is_empty()) {
        (true, false) => {
            let n = number(b).ok_or(RangeError::Unreadable)?;
            if n == 0 || size == 0 {
                return Err(RangeError::Unsatisfiable);
            }
            let len = n.min(size);
            Ok(ByteRange { first: size - len, len })
        }
        (false, _) => {
            let first = number(a).ok_or(RangeError::Unreadable)?;
            let last = if b.is_empty() { None } else { Some(number(b).ok_or(RangeError::Unreadable)?) };
            if last.is_some_and(|l| l < first) {
                return Err(RangeError::Unreadable);
            }
            if first >= size {
                return Err(RangeError::Unsatisfiable);
            }
            let last = last.map_or(size - 1, |l| l.min(size - 1));
            Ok(ByteRange { first, len: last - first + 1 })
        }
        (true, true) => Err(RangeError::Unreadable),
    }
}

pub fn parse_content_range(value: &[u8]) -> Option<(ByteRange, u64)> {
    if value.len() > MAX_CONTENT_RANGE {
        return None;
    }
    let text = std::str::from_utf8(value).ok()?;
    let spec = text.strip_prefix("bytes ")?;
    let (range, size) = spec.split_once('/')?;
    let (a, b) = range.split_once('-')?;
    let (first, last, size) = (number(a)?, number(b)?, number(size)?);
    (first <= last && last < size).then(|| (ByteRange { first, len: last - first + 1 }, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_range_backwards_is_refused() {
        assert_eq!(parse_content_range(b"bytes 2-1/4096"), None);
        assert_eq!(parse_content_range(b"bytes 1-1/4096"), Some((ByteRange { first: 1, len: 1 }, 4096)));
        for size in [1_008_806_316_530_991_104, u64::MAX] {
            let r = ByteRange { first: size - 5, len: 5 };
            assert_eq!(parse_content_range(r.content_range(size).as_bytes()), Some((r, size)));
        }
        assert_eq!(parse_content_range(b"bytes 01-1/4096"), None);
        assert_eq!(parse_content_range(b"bytes 0-1/18446744073709551616"), None, "past u64::MAX");
        assert_eq!(parse_range(b"bytes=00-", 10), Err(RangeError::Unreadable));
    }

    #[test]
    fn ranges_read_one_way() {
        let r = |s: &str, size| parse_range(s.as_bytes(), size);
        assert_eq!(r("bytes=0-", 10), Ok(ByteRange { first: 0, len: 10 }));
        assert_eq!(r("bytes=4-", 10), Ok(ByteRange { first: 4, len: 6 }));
        assert_eq!(r("Bytes=2-5", 10), Ok(ByteRange { first: 2, len: 4 }));
        assert_eq!(r("bytes=2-99", 10), Ok(ByteRange { first: 2, len: 8 }), "a last past the end is the end");
        assert_eq!(r("bytes=-3", 10), Ok(ByteRange { first: 7, len: 3 }));
        assert_eq!(r("bytes=-30", 10), Ok(ByteRange { first: 0, len: 10 }));
        assert_eq!(r("bytes=10-", 10), Err(RangeError::Unsatisfiable));
        assert_eq!(r("bytes=-0", 10), Err(RangeError::Unsatisfiable));
        assert_eq!(r("bytes=0-", 0), Err(RangeError::Unsatisfiable));
        for bad in [
            "bytes=0-1,2-3",
            "bytes=0-1, 0-1",
            "bytes=5-2",
            "bytes= 1-2",
            "bytes=+1-2",
            "bytes=-",
            "bytes=1",
            "items=0-1",
            "bytes=99999999999999999999-",
            "bytes=0x1-",
        ] {
            assert_eq!(r(bad, 10), Err(RangeError::Unreadable), "{bad}");
        }
        assert_eq!(r(&format!("bytes={}-", "0".repeat(60)), 10), Err(RangeError::Unreadable));
        let big = u64::MAX;
        assert_eq!(r("bytes=18446744073709551614-", big), Ok(ByteRange { first: big - 1, len: 1 }));
        assert_eq!(ByteRange { first: 2, len: 4 }.content_range(10), "bytes 2-5/10");
        assert_eq!(parse_content_range(b"bytes 2-5/10"), Some((ByteRange { first: 2, len: 4 }, 10)));
        for bad in ["bytes 5-2/10", "bytes 2-10/10", "bytes */10", "bytes 2-5/", "bits 2-5/10"] {
            assert_eq!(parse_content_range(bad.as_bytes()), None, "{bad}");
        }
    }
}
