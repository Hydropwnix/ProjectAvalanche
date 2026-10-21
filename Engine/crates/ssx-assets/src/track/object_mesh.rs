use std::ops::Range;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_formats::geom::{GeomFile, MeshVersion, Semantic};

#[derive(Clone, Debug, Default)]
pub struct ObjectMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 3]>>,
    pub uv0: Option<Vec<[f32; 2]>>,
    pub uv1: Option<Vec<[f32; 2]>>,
    pub colours: Option<Vec<[u8; 4]>>,
    pub indices: Vec<u32>,
    pub groups: Vec<Range<u32>>,
    pub translucent: bool,
}

impl ObjectMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first, first), |(lo, hi), p| {
            (std::array::from_fn(|i| lo[i].min(p[i])), std::array::from_fn(|i| hi[i].max(p[i])))
        }))
    }
}

pub fn load(path: &Path) -> Result<ObjectMesh> {
    let data = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let geom = GeomFile::parse(&data).with_context(|| format!("parsing {}", path.display()))?;
    object_mesh(&geom)
}

pub fn object_mesh(geom: &GeomFile) -> Result<ObjectMesh> {
    let [mesh] = geom.meshes.as_slice() else {
        bail!("{} meshes in one .geom; only single-mesh files are known", geom.meshes.len());
    };
    let values = |semantic: Semantic, set: u8| -> Result<Option<Vec<[f32; 4]>>> {
        geom.attribute(semantic, set).map(|a| geom.attribute_values(a)).transpose().map_err(Into::into)
    };
    let xyz = |v: Vec<[f32; 4]>| v.into_iter().map(|[x, y, z, _]| [x, y, z]).collect::<Vec<_>>();
    let xy = |v: Vec<[f32; 4]>| v.into_iter().map(|[x, y, _, _]| [x, y]).collect::<Vec<_>>();

    let positions = xyz(values(Semantic::Position, 0)?.context(".geom without a p0 attribute")?);
    let colours =
        values(Semantic::Colour, 0)?.map(|v| v.into_iter().map(|c| c.map(|x| (x * 255.0).round() as u8)).collect());
    let groups = match mesh.version {
        MeshVersion::Geo3 => Vec::new(),
        MeshVersion::Geo4 => {
            mesh.groups.iter().map(|g| g.first_triangle as u32..(g.first_triangle + g.triangle_count) as u32).collect()
        }
    };

    Ok(ObjectMesh {
        name: mesh.name.clone(),
        positions,
        normals: values(Semantic::Normal, 0)?.map(xyz),
        tangents: values(Semantic::Tangent, 0)?.map(xyz),
        uv0: values(Semantic::TexCoord, 0)?.map(xy),
        uv1: values(Semantic::TexCoord, 1)?.map(xy),
        colours,
        indices: geom.indices().iter().map(|&i| u32::from(i)).collect(),
        groups,
        translucent: mesh.translucent,
    })
}
