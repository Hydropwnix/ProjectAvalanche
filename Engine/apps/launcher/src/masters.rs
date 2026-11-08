pub fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    use serde::Deserialize;
    serde::de::IgnoredAny::deserialize(d)?;
    Ok(true)
}

pub fn old_member_note(present: bool) -> Option<String> {
    present.then(|| {
        format!(
            "settings.json: `masters` is not read any more: the master server is fixed ({}); the member is dropped at \
             the next save",
            ssx_core::OFFICIAL_MASTER
        )
    })
}
