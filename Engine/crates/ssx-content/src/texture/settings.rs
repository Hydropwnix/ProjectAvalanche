use std::collections::BTreeMap;

use serde::Deserialize;
use toml::Spanned;

use super::TextureKind;
use crate::id::Kind;
use crate::limits::{MAX_RECORD_BYTES, MAX_TOML_DEPTH, table_depth};
use crate::manifest::line_col;
use crate::path::check_package_path;

pub const SETTINGS_FILE: &str = "textures.toml";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureSettings {
    pub kind: Option<TextureKind>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderSettings {
    files: BTreeMap<String, TextureSettings>,
    positions: BTreeMap<String, (usize, usize)>,
}

impl FolderSettings {
    pub fn parse(bytes: &[u8], rel: &str) -> Result<Self, String> {
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(format!("{rel}: over {MAX_RECORD_BYTES} bytes"));
        }
        let text = std::str::from_utf8(bytes).map_err(|e| format!("{rel}: not UTF-8: {e}"))?;
        let at = |span: std::ops::Range<usize>| {
            let (line, col) = line_col(text, span.start);
            format!("{rel}:{line}:{col}")
        };
        let raw: toml::Table = toml::from_str(text).map_err(|e| {
            let (line, col) = e.span().map_or((0, 0), |s| line_col(text, s.start));
            format!("{rel}:{line}:{col}: {}", ssx_core::text::message(e.message()))
        })?;
        if table_depth(&raw) > MAX_TOML_DEPTH {
            return Err(format!("{rel}: nested deeper than {MAX_TOML_DEPTH}"));
        }
        let parsed: BTreeMap<Spanned<String>, Spanned<TextureSettings>> = toml::from_str(text).map_err(|e| {
            let (line, col) = e.span().map_or((0, 0), |s| line_col(text, s.start));
            format!("{rel}:{line}:{col}: {}", ssx_core::text::message(e.message()))
        })?;
        let mut files = BTreeMap::new();
        let mut positions = BTreeMap::new();
        for (name, settings) in parsed {
            let span = name.span();
            let name = name.into_inner();
            if name.contains('/') {
                return Err(format!(
                    "{}: {} names a file in another folder (one name per file here)",
                    at(span),
                    ssx_core::text::quoted(&name)
                ));
            }
            check_package_path(&name, false).map_err(|e| format!("{}: {e}", at(span.clone())))?;
            let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
            if !Kind::Texture.formats().contains(&ext.as_str()) {
                let allowed = Kind::Texture.formats().join(", ");
                return Err(format!(
                    "{}: {} is not a texture file ({allowed})",
                    at(span),
                    ssx_core::text::quoted(&name)
                ));
            }
            positions.insert(name.clone(), line_col(text, span.start));
            files.insert(name, settings.into_inner());
        }
        Ok(FolderSettings { files, positions })
    }

    pub fn of(&self, file: &str) -> TextureSettings {
        self.files.get(file).copied().unwrap_or_default()
    }

    pub fn position(&self, file: &str) -> (usize, usize) {
        self.positions.get(file).copied().unwrap_or((0, 0))
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, &TextureSettings)> {
        self.files.iter().map(|(n, s)| (n.as_str(), s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_per_file() {
        let s =
            FolderSettings::parse(b"[\"rock_n.png\"]\nkind = \"normal\"\n[\"a.dds\"]\n", "t/textures.toml").unwrap();
        assert_eq!(s.of("rock_n.png").kind, Some(TextureKind::Normal));
        assert_eq!(s.of("a.dds").kind, None);
        assert_eq!(s.of("other.png"), TextureSettings::default());
    }

    #[test]
    fn hostile_settings_fail_with_the_position() {
        let e = FolderSettings::parse(b"[\"a.png\"]\nkind = \"colour\"\n", "t.toml").unwrap_err();
        assert!(e.starts_with("t.toml:2:"), "{e}");
        let e = FolderSettings::parse(b"[\"a.png\"]\nkin = \"color\"\n", "t.toml").unwrap_err();
        assert!(e.starts_with("t.toml:2:") && e.contains("kin"), "{e}");
        let e = FolderSettings::parse(b"\n[\"x/a.png\"]\n", "t.toml").unwrap_err();
        assert!(e.starts_with("t.toml:2:2") && e.contains("another folder"), "{e}");
        let e = FolderSettings::parse(b"[\"a.txt\"]\n", "t.toml").unwrap_err();
        assert!(e.contains("not a texture file"), "{e}");
        let e = FolderSettings::parse(b"[\"con.png\"]\n", "t.toml").unwrap_err();
        assert!(e.contains("reserved device name"), "{e}");
        assert!(FolderSettings::parse(&[0xff, 0xfe], "t.toml").unwrap_err().contains("not UTF-8"));
        assert!(FolderSettings::parse(b"kind = \"color\"\n", "t.toml").is_err());
        let deep = format!("[\"a.png\"]\nkind = {}1{}\n", "[".repeat(40), "]".repeat(40));
        assert!(FolderSettings::parse(deep.as_bytes(), "t.toml").is_err());
    }
}
