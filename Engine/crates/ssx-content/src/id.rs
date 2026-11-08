use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const BASE_NAMESPACE: &str = "ssx";
pub const PORT_NAMESPACE: &str = "ava";
pub const MAX_ID_LEN: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Texture,
    Model,
    Object,
    Material,
    Character,
    Gear,
    Board,
    Anim,
    Track,
    Drop,
    Event,
    Lighting,
    Sky,
    Postfx,
    Particles,
    Sound,
    Music,
    Movie,
    Ui,
    Font,
    String,
    Tuning,
    Scoring,
    Script,
    Marker,
    Map,
    Gamemode,
    Trick,
}

impl Kind {
    pub const ALL: [Kind; 28] = [
        Kind::Texture,
        Kind::Model,
        Kind::Object,
        Kind::Material,
        Kind::Character,
        Kind::Gear,
        Kind::Board,
        Kind::Anim,
        Kind::Track,
        Kind::Drop,
        Kind::Event,
        Kind::Lighting,
        Kind::Sky,
        Kind::Postfx,
        Kind::Particles,
        Kind::Sound,
        Kind::Music,
        Kind::Movie,
        Kind::Ui,
        Kind::Font,
        Kind::String,
        Kind::Tuning,
        Kind::Scoring,
        Kind::Script,
        Kind::Marker,
        Kind::Map,
        Kind::Gamemode,
        Kind::Trick,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Texture => "texture",
            Kind::Model => "model",
            Kind::Object => "object",
            Kind::Material => "material",
            Kind::Character => "character",
            Kind::Gear => "gear",
            Kind::Board => "board",
            Kind::Anim => "anim",
            Kind::Track => "track",
            Kind::Drop => "drop",
            Kind::Event => "event",
            Kind::Lighting => "lighting",
            Kind::Sky => "sky",
            Kind::Postfx => "postfx",
            Kind::Particles => "particles",
            Kind::Sound => "sound",
            Kind::Music => "music",
            Kind::Movie => "movie",
            Kind::Ui => "ui",
            Kind::Font => "font",
            Kind::String => "string",
            Kind::Tuning => "tuning",
            Kind::Scoring => "scoring",
            Kind::Script => "script",
            Kind::Marker => "marker",
            Kind::Map => "map",
            Kind::Gamemode => "gamemode",
            Kind::Trick => "trick",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub const fn read_from_packages(self) -> bool {
        !matches!(self, Kind::Track | Kind::Script)
    }

    pub const fn patchable(self) -> bool {
        matches!(
            self,
            Kind::Character
                | Kind::Gear
                | Kind::Board
                | Kind::Drop
                | Kind::Event
                | Kind::Lighting
                | Kind::Postfx
                | Kind::Particles
                | Kind::Tuning
                | Kind::Scoring
                | Kind::Marker
                | Kind::Map
        )
    }

    pub const fn formats(self) -> &'static [&'static str] {
        match self {
            Kind::Texture => &["png", "jpg", "jpeg", "ktx2", "dds"],
            Kind::Model => &["glb", "gltf"],
            Kind::Anim => &["toml", "glb", "gltf"],
            Kind::Character | Kind::Gear | Kind::Board | Kind::Marker | Kind::Map => &["glb", "gltf", "toml"],
            Kind::Sky => &["toml", "ktx2", "dds", "png", "hdr", "exr"],
            Kind::Postfx => &["toml", "png"],
            Kind::Particles => &["toml", "png", "dds", "ktx2"],
            Kind::Sound => &["ogg", "flac", "wav", "toml"],
            Kind::Music => &["ogg", "flac", "wav", "toml"],
            Kind::Ui => &["swf", "gfx", "json"],
            Kind::Font => &["ttf", "otf"],
            Kind::Script => &["luau"],
            Kind::Gamemode => &["toml"],
            Kind::Trick => &["toml"],
            Kind::Object
            | Kind::Material
            | Kind::Track
            | Kind::Drop
            | Kind::Event
            | Kind::Lighting
            | Kind::String
            | Kind::Tuning
            | Kind::Scoring => &["toml"],
            Kind::Movie => &["webm"],
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Kind {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        Kind::parse(s).ok_or_else(|| IdError::UnknownKind(s.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("empty id")]
    Empty,
    #[error("id is {0} bytes long (at most {MAX_ID_LEN})")]
    TooLong(usize),
    #[error("no `<namespace>:` before the kind")]
    NoNamespace,
    #[error("bad namespace {} (lower case `[a-z0-9_.-]`)", ssx_core::text::quoted(.0))]
    BadNamespace(String),
    #[error("unknown kind {}", ssx_core::text::quoted(.0))]
    UnknownKind(String),
    #[error("no path after `<kind>/`")]
    NoPath,
    #[error("empty path segment")]
    EmptySegment,
    #[error("path segment {} is not allowed", ssx_core::text::quoted(.0))]
    DotSegment(String),
    #[error("character {0:?} at byte {1} is not allowed (lower case `[a-z0-9_.-]`, `/` between segments)")]
    BadChar(char, usize),
}

pub const fn segment_char(c: char) -> bool {
    matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-')
}

pub fn valid_namespace(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(segment_char)
}

fn check_path(path: &str, offset: usize) -> Result<(), IdError> {
    if path.is_empty() {
        return Err(IdError::NoPath);
    }
    let mut at = offset;
    for seg in path.split('/') {
        if seg.is_empty() {
            return Err(IdError::EmptySegment);
        }
        if seg == "." || seg == ".." {
            return Err(IdError::DotSegment(seg.to_string()));
        }
        if let Some((i, c)) = seg.char_indices().find(|&(_, c)| !segment_char(c)) {
            return Err(IdError::BadChar(c, at + i));
        }
        at += seg.len() + 1;
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId {
    text: String,
    colon: usize,
    kind: Kind,
}

impl ContentId {
    pub fn parse(s: &str) -> Result<Self, IdError> {
        if s.is_empty() {
            return Err(IdError::Empty);
        }
        if s.len() > MAX_ID_LEN {
            return Err(IdError::TooLong(s.len()));
        }
        let colon = s.find(':').ok_or(IdError::NoNamespace)?;
        let ns = &s[..colon];
        if !valid_namespace(ns) {
            return Err(IdError::BadNamespace(ns.to_string()));
        }
        let rest = &s[colon + 1..];
        let (kind, path) = rest.split_once('/').unwrap_or((rest, ""));
        let kind = Kind::parse(kind).ok_or_else(|| IdError::UnknownKind(kind.to_string()))?;
        check_path(path, colon + 1 + kind.as_str().len() + 1)?;
        Ok(ContentId { text: s.to_string(), colon, kind })
    }

    pub fn new(namespace: &str, kind: Kind, path: &str) -> Result<Self, IdError> {
        Self::parse(&format!("{namespace}:{kind}/{path}"))
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    pub fn namespace(&self) -> &str {
        &self.text[..self.colon]
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn path(&self) -> &str {
        &self.text[self.colon + 1 + self.kind.as_str().len() + 1..]
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentId({})", self.text)
    }
}

impl FromStr for ContentId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, IdError> {
        Self::parse(s)
    }
}

impl Serialize for ContentId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        ContentId::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays() {
        let id = ContentId::parse("ssx:texture/worlds/textures/rock_02").unwrap();
        assert_eq!(id.namespace(), "ssx");
        assert_eq!(id.kind(), Kind::Texture);
        assert_eq!(id.path(), "worlds/textures/rock_02");
        assert_eq!(id.to_string(), "ssx:texture/worlds/textures/rock_02");
        let id: ContentId = "neon-pack:board/neon".parse().unwrap();
        assert_eq!((id.namespace(), id.kind(), id.path()), ("neon-pack", Kind::Board, "neon"));
        assert_eq!(ContentId::new("ava", Kind::Marker, "ring").unwrap().as_str(), "ava:marker/ring");
        assert!(ContentId::parse("ssx:postfx/dusk_orange_postfx.v2").is_ok());
    }

    #[test]
    fn every_kind_round_trips() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
            let id = ContentId::new("ssx", k, "a/b").unwrap();
            assert_eq!(id.kind(), k);
            assert_eq!(id.path(), "a/b");
        }
        assert_eq!(Kind::ALL.len(), 28);
    }

    #[test]
    fn rejects() {
        use IdError::*;
        let bad = |s: &str| ContentId::parse(s).unwrap_err();
        assert_eq!(bad(""), Empty);
        assert_eq!(bad("texture/rock"), NoNamespace);
        assert_eq!(bad(":texture/rock"), BadNamespace(String::new()));
        assert_eq!(bad("SSX:texture/rock"), BadNamespace("SSX".into()));
        assert_eq!(bad("..:texture/rock"), BadNamespace("..".into()));
        assert_eq!(bad("ssx:textures/rock"), UnknownKind("textures".into()));
        assert_eq!(bad("ssx:Texture/rock"), UnknownKind("Texture".into()));
        assert_eq!(bad("ssx:texture"), NoPath);
        assert_eq!(bad("ssx:texture/"), NoPath);
        assert_eq!(bad("ssx:texture//rock"), EmptySegment);
        assert_eq!(bad("ssx:texture/rock/"), EmptySegment);
        assert_eq!(bad("ssx:texture/../rock"), DotSegment("..".into()));
        assert_eq!(bad("ssx:texture/./rock"), DotSegment(".".into()));
        assert_eq!(bad("ssx:texture/Rock"), BadChar('R', 12));
        assert_eq!(bad("ssx:texture/a b"), BadChar(' ', 13));
        assert_eq!(bad("ssx:texture/a\\b"), BadChar('\\', 13));
        assert_eq!(bad("ssx:texture/a:b"), BadChar(':', 13));
        assert_eq!(bad("ssx:texture/é"), BadChar('é', 12));
        assert_eq!(bad(&format!("ssx:texture/{}", "a".repeat(300))), TooLong(312));
    }

    #[test]
    fn serde_as_string() {
        #[derive(Deserialize, Serialize)]
        struct T {
            id: ContentId,
        }
        let t: T = toml::from_str("id = \"ssx:sound/carve_left\"").unwrap();
        assert_eq!(t.id.kind(), Kind::Sound);
        assert_eq!(toml::to_string(&t).unwrap().trim(), "id = \"ssx:sound/carve_left\"");
        assert!(toml::from_str::<T>("id = \"ssx:sound/Carve\"").is_err());
    }

    #[test]
    fn kind_tables() {
        assert!(Kind::Board.patchable() && Kind::Tuning.patchable() && !Kind::Texture.patchable());
        assert_eq!(Kind::ALL.iter().filter(|k| k.patchable()).count(), 12);
        assert_eq!(Kind::Movie.formats(), &["webm"]);
        assert!(Kind::ALL.iter().all(|k| !k.formats().is_empty()));
    }
}
