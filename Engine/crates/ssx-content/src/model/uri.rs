use super::base64;

#[derive(Debug, PartialEq, Eq)]
pub enum Target {
    Data { media_type: String, bytes: Vec<u8> },
    File(String),
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn percent_decode(s: &str) -> Result<String, String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let (Some(h), Some(l)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) else {
                return Err(format!("{}: a broken percent escape", ssx_core::text::quoted(s)));
            };
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("{}: not UTF-8 once decoded", ssx_core::text::quoted(s)))
}

pub fn resolve(uri: &str, dir: &str, max_data: u64) -> Result<Target, String> {
    if let Some(rest) = uri.strip_prefix("data:") {
        let (head, payload) = rest.split_once(',').ok_or_else(|| "a data URI without a comma".to_string())?;
        let media_type = head.strip_suffix(";base64").ok_or_else(|| "a data URI that is not base64".to_string())?;
        if media_type.len() > 128 || media_type.contains(';') {
            return Err(format!("a data URI with the parameters {}", quoted(media_type)));
        }
        let bytes = base64::decode(payload.as_bytes(), max_data)?;
        return Ok(Target::Data { media_type: media_type.to_string(), bytes });
    }
    let scheme_end = uri.find(':');
    let first_sep = uri.find(['/', '?', '#']);
    if let Some(c) = scheme_end
        && first_sep.is_none_or(|s| c < s)
    {
        return Err(format!("{}: only files of the package and data: URIs are read", quoted(uri)));
    }
    if uri.starts_with('/') || uri.contains('\\') {
        return Err(format!("{}: an absolute path or a backslash", quoted(uri)));
    }
    if uri.contains(['?', '#']) {
        return Err(format!("{}: a query or fragment", quoted(uri)));
    }
    let decoded = percent_decode(uri)?;
    let path = if dir.is_empty() { decoded } else { format!("{dir}/{decoded}") };
    crate::path::check_package_path(&path, false).map_err(|e| format!("{}: {e}", quoted(uri)))?;
    Ok(Target::File(path))
}

pub fn truncate(s: &str) -> String {
    ssx_core::text::shown_max(s, 128).to_string()
}

pub fn quoted(s: &str) -> ssx_core::text::Shown<'_> {
    ssx_core::text::quoted_max(s, 128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_data_and_refusals() {
        assert_eq!(resolve("a.bin", "models", 64), Ok(Target::File("models/a.bin".into())));
        assert_eq!(resolve("tex/rock%2Ba.png", "", 64), Ok(Target::File("tex/rock+a.png".into())));
        assert!(resolve("tex/rock%20a.png", "", 64).is_err(), "a space is no package name");
        let d = resolve("data:application/octet-stream;base64,aGk=", "", 64).unwrap();
        assert_eq!(d, Target::Data { media_type: "application/octet-stream".into(), bytes: b"hi".to_vec() });
        for bad in [
            "http://example.com/a.bin",
            "file:///etc/passwd",
            "C:/x.bin",
            "c:x.bin",
            "/etc/passwd",
            "..\\x.bin",
            "../x.bin",
            "a/../../x.bin",
            "%2e%2e/x.bin",
            "a.bin?x=1",
            "a.bin#f",
            "%zz.bin",
            "%ff.bin",
            "con.bin",
            "data:text/plain,hello",
            "data:a;b;base64,aGk",
        ] {
            assert!(resolve(bad, "models", 64).is_err(), "{bad} accepted");
        }
        assert!(resolve("data:;base64,aGVsbG8gd29ybGQ=", "", 4).is_err(), "over the data cap");
    }
}
