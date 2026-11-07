use super::NameKind;

pub const MAX_NAME: usize = 128;

pub fn max_len(kind: NameKind) -> usize {
    match kind {
        NameKind::ContentId => ssx_content::id::MAX_ID_LEN,
        _ => MAX_NAME,
    }
}

const RESERVED_KINDS: [&str; 13] = [
    "root",
    "resource",
    "player",
    "rider",
    "instance",
    "marker",
    "checkpoint",
    "spawn",
    "prop",
    "trigger",
    "blip",
    "text3d",
    "sound",
];

fn plain(s: &str, max: usize, extra: &[u8], what: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > max {
        return Err(format!("{what} is 1 to {max} bytes"));
    }
    if !s.bytes().all(|b| b.is_ascii_alphanumeric() || extra.contains(&b)) {
        let allowed: String = extra.iter().map(|&b| char::from(b)).collect();
        return Err(format!("{what} is made of A-Z a-z 0-9 and {allowed:?}"));
    }
    Ok(())
}

pub fn check_event_name(s: &str) -> Result<(), String> {
    plain(s, 128, b"_.:-", "an event name")
}

pub fn check_element_kind(s: &str) -> Result<(), String> {
    plain(s, 64, b"_-", "an element type")
}

pub fn check_element_name(s: &str) -> Result<(), String> {
    if s.len() > 128 {
        return Err("an element id is at most 128 bytes".into());
    }
    if s.chars().any(char::is_control) {
        return Err("an element id has no control characters".into());
    }
    Ok(())
}

pub fn check_data_key(s: &str) -> Result<(), String> {
    plain(s, 128, b"_.:-", "a data key")
}

pub fn check_command_name(s: &str) -> Result<(), String> {
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err("a command name is lower case".into());
    }
    plain(s, 32, b"_-", "a command name")
}

pub fn check_server_command_name(s: &str) -> Result<(), String> {
    check_command_name(s)?;
    if s.contains('-') {
        return Err(format!(
            "{}: a server command name is a-z, 0-9 and _ (an operator runs it with the right command.<name>)",
            ssx_core::text::quoted(s)
        ));
    }
    Ok(())
}

pub fn check_export_name(s: &str) -> Result<(), String> {
    plain(s, 64, b"_", "an export name")
}

pub fn check(kind: NameKind, s: &str) -> Result<(), String> {
    match kind {
        NameKind::Event => check_event_name(s),
        NameKind::ElementKind => check_element_kind(s),
        NameKind::NewElementKind => {
            check_element_kind(s)?;
            if RESERVED_KINDS.contains(&s) {
                return Err(format!("{s:?} is a type the host or the game makes"));
            }
            Ok(())
        }
        NameKind::ElementName => check_element_name(s),
        NameKind::DataKey => check_data_key(s),
        NameKind::Command => check_command_name(s),
        NameKind::Export => check_export_name(s),
        NameKind::StorageKey => crate::host::storage::check_key(s.as_bytes()).map(|_| ()),
        NameKind::Setting => plain(s, 128, b"_.-", "a setting name"),
        NameKind::Package => crate::host::package::check_name(s),
        NameKind::ContentId => s.parse::<ssx_content::ContentId>().map(|_| ()).map_err(|e| e.to_string()),
    }
}
