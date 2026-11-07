use std::sync::Arc;

use super::{Shape, Texture, TextureKind};
use crate::id::ContentId;
use crate::registry::{Asset, Layer, LoadError, Registry};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sampling {
    Texels,
    NormalXy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Want {
    pub sampling: Sampling,
    pub shape: Shape,
}

impl Want {
    pub const PLANE: Want = Want { sampling: Sampling::Texels, shape: Shape::Plane };
    pub const NORMAL: Want = Want { sampling: Sampling::NormalXy, shape: Shape::Plane };
    pub const CUBE: Want = Want { sampling: Sampling::Texels, shape: Shape::Cube };
}

#[derive(Debug)]
pub struct Found {
    pub texture: Texture,
    pub kind: TextureKind,
    pub layer: Layer,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fallback {
    pub id: ContentId,
    pub layer: Layer,
    pub source: String,
    pub error: String,
}

fn fits(texture: &Texture, want: &Want) -> Result<TextureKind, String> {
    fit(texture.shape, texture.kind, want)
}

pub fn fit(shape: Shape, kind: Option<TextureKind>, want: &Want) -> Result<TextureKind, String> {
    if shape != want.shape {
        return Err(format!("the game samples this texture as a {:?}, the file is a {:?}", want.shape, shape));
    }
    match (kind, want.sampling) {
        (Some(TextureKind::Normal), Sampling::Texels) => Err("kind = \"normal\" (a two-channel normal map), but the \
            game samples this texture's raw channels: use kind = \"color\" or \"data\""
            .into()),
        (Some(k @ (TextureKind::Color | TextureKind::Data)), Sampling::NormalXy) => Err(format!(
            "kind = \"{}\", but the game samples this texture as a two-channel normal map (X, Y): use kind = \"normal\"",
            k.as_str()
        )),
        (Some(k), _) => Ok(k),
        (None, Sampling::NormalXy) => Ok(TextureKind::Normal),
        (None, Sampling::Texels) => Ok(TextureKind::Color),
    }
}

pub fn resolve(
    registry: &Registry,
    id: &ContentId,
    want: &Want,
    passed: impl FnMut(Fallback),
) -> Result<Found, LoadError> {
    resolve_in(registry, id, want, |_| true, passed)
}

pub fn resolve_in(
    registry: &Registry,
    id: &ContentId,
    want: &Want,
    layers: impl Fn(Layer) -> bool,
    mut passed: impl FnMut(Fallback),
) -> Result<Found, LoadError> {
    let mut last = None;
    for (mount, source) in registry.providers(id).filter(|(m, _)| layers(m.layer)) {
        let got = source.load(id).and_then(|asset| match asset {
            Asset::Value(v) => match v.downcast::<Texture>() {
                Ok(t) => Ok(Arc::try_unwrap(t).unwrap_or_else(|shared| (*shared).clone())),
                Err(_) => {
                    Err(LoadError::Failed { id: id.clone(), message: "the source's asset is not a texture".into() })
                }
            },
            _ => Err(LoadError::Failed { id: id.clone(), message: "the source's asset is not a texture".into() }),
        });
        let checked = got.and_then(|t| match fits(&t, want) {
            Ok(kind) => Ok((t, kind)),
            Err(message) => Err(LoadError::Failed { id: id.clone(), message }),
        });
        match checked {
            Ok((texture, kind)) => {
                return Ok(Found { texture, kind, layer: mount.layer, source: mount.name.to_string() });
            }
            Err(e) => {
                passed(Fallback {
                    id: id.clone(),
                    layer: mount.layer,
                    source: mount.name.to_string(),
                    error: e.to_string(),
                });
                last = Some(e);
            }
        }
    }
    Err(last.unwrap_or_else(|| LoadError::Missing(id.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::MemorySource;
    use crate::texture::Level;

    fn tex(kind: Option<TextureKind>, shape: Shape, v: u8) -> Asset {
        let images = vec![vec![v; 4]; if shape == Shape::Cube { 6 } else { 1 }];
        let t =
            Texture { shape, levels: vec![Level { width: 1, height: 1, images }], blocks: None, kind, source: None };
        Asset::Value(Arc::new(t))
    }

    fn id() -> ContentId {
        "ssx:texture/worlds/textures/rock".parse().unwrap()
    }

    #[test]
    fn the_strongest_fitting_source_wins_and_the_others_are_reported() {
        let mut r = Registry::new();
        r.mount(Layer::Base, 0, Box::new(MemorySource::new("ssx").with(id(), tex(None, Shape::Plane, 1))));
        let bad_kind = MemorySource::new("normals").with(id(), tex(Some(TextureKind::Normal), Shape::Plane, 2));
        r.mount(Layer::ClientMods, 0, Box::new(bad_kind));
        let cube = MemorySource::new("cube").with(id(), tex(None, Shape::Cube, 3));
        r.mount(Layer::ClientMods, 1, Box::new(cube));
        let mut passed = Vec::new();
        let found = resolve(&r, &id(), &Want::PLANE, |f| passed.push(f)).unwrap();
        assert_eq!((found.source.as_str(), found.layer, found.kind), ("ssx", Layer::Base, TextureKind::Color));
        assert_eq!(found.texture.levels[0].images[0][0], 1);
        let names: Vec<_> = passed.iter().map(|f| f.source.as_str()).collect();
        assert_eq!(names, ["cube", "normals"]);
        assert!(passed[0].error.contains("as a Plane, the file is a Cube"), "{}", passed[0].error);
        assert!(passed[1].error.contains("use kind = \"color\" or \"data\""), "{}", passed[1].error);
        let found = resolve(&r, &id(), &Want::NORMAL, |_| {}).unwrap();
        assert_eq!((found.source.as_str(), found.kind), ("normals", TextureKind::Normal));
        let found = resolve(&r, &id(), &Want::CUBE, |_| {}).unwrap();
        assert_eq!(found.source, "cube");
    }

    #[test]
    fn nothing_that_fits_is_the_last_error() {
        let mut r = Registry::new();
        let colour = MemorySource::new("pack").with(id(), tex(Some(TextureKind::Data), Shape::Plane, 2));
        r.mount(Layer::ClientMods, 0, Box::new(colour));
        let e = resolve(&r, &id(), &Want::NORMAL, |_| {}).unwrap_err().to_string();
        assert!(e.contains("use kind = \"normal\""), "{e}");
        let other: ContentId = "ssx:texture/none".parse().unwrap();
        assert!(matches!(resolve(&r, &other, &Want::PLANE, |_| {}), Err(LoadError::Missing(_))));
    }
}
