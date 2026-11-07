fn value(c: u8) -> Option<u32> {
    Some(match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    } as u32)
}

pub fn decode(text: &[u8], max: u64) -> Result<Vec<u8>, String> {
    let body = match text {
        [rest @ .., b'=', b'='] => rest,
        [rest @ .., b'='] => rest,
        _ => text,
    };
    if text.len() != body.len() && !text.len().is_multiple_of(4) {
        return Err("base64 padding on a length that is not a multiple of 4".into());
    }
    if body.len() % 4 == 1 {
        return Err("base64 of a length no encoder makes".into());
    }
    let size = body.len() as u64 / 4 * 3 + [0, 0, 1, 2][body.len() % 4];
    if size > max {
        return Err(format!("a data URI of {size} bytes; at most {max} are read"));
    }
    let mut out = Vec::with_capacity(size as usize);
    for chunk in body.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = value(c).ok_or_else(|| format!("{:?} is not a base64 character", c as char))?;
            acc |= v << (18 - 6 * i);
        }
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(&bytes[1..chunk.len()]);
        let unused = match chunk.len() {
            2 => acc & 0xFFFF,
            3 => acc & 0xFF,
            _ => 0,
        };
        if unused != 0 {
            return Err("base64 with stray bits in its last character".into());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn decodes_and_refuses() {
        assert_eq!(decode(b"aGVsbG8=", 100).unwrap(), b"hello");
        assert_eq!(decode(b"aGVsbG8", 100).unwrap(), b"hello");
        assert_eq!(decode(b"aGVsbA==", 100).unwrap(), b"hell");
        assert_eq!(decode(b"", 100).unwrap(), b"");
        assert!(decode(b"aGVsbG8=", 4).is_err(), "over the cap");
        assert!(decode(b"aGV sbG8=", 100).is_err(), "a space");
        assert!(decode(b"a", 100).is_err());
        assert!(decode(b"aGVsbG9=", 100).is_err(), "stray bits");
        assert!(decode(b"aG=", 100).is_err(), "padding on a short length");
    }
}
