use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use ssx_content::marker::{Look, MarkerDef, Shape, from_table};
use ssx_content::model::Mesh;
use ssx_content::{Asset, ContentId, Kind, Layer, LoadError, PORT_NAMESPACE, Source};

use super::resolve::ModelUse;
use super::store::GameContent;

const SHAPES: [(Shape, &str, &str); 6] = [
    (
        Shape::Ring,
        include_str!("../../../../assets/markers/ring.toml"),
        include_str!("../../../../assets/markers/ring.gltf"),
    ),
    (
        Shape::Cylinder,
        include_str!("../../../../assets/markers/cylinder.toml"),
        include_str!("../../../../assets/markers/cylinder.gltf"),
    ),
    (
        Shape::Arrow,
        include_str!("../../../../assets/markers/arrow.toml"),
        include_str!("../../../../assets/markers/arrow.gltf"),
    ),
    (
        Shape::Chevron,
        include_str!("../../../../assets/markers/chevron.toml"),
        include_str!("../../../../assets/markers/chevron.gltf"),
    ),
    (
        Shape::Beam,
        include_str!("../../../../assets/markers/beam.toml"),
        include_str!("../../../../assets/markers/beam.gltf"),
    ),
    (
        Shape::Gate,
        include_str!("../../../../assets/markers/gate.toml"),
        include_str!("../../../../assets/markers/gate.gltf"),
    ),
];

pub const MAX_MARKER_VERTICES: usize = 65_536;
pub const MAX_MARKER_TRIANGLES: usize = 65_536;

pub struct PortMarkers;

impl Source for PortMarkers {
    fn name(&self) -> &str {
        PORT_NAMESPACE
    }

    fn has(&self, id: &ContentId) -> bool {
        id.kind() == Kind::Marker && id.namespace() == PORT_NAMESPACE && Shape::parse(id.path()).is_some()
    }

    fn load(&self, id: &ContentId) -> Result<Asset, LoadError> {
        let (_, toml_text, _) = SHAPES
            .iter()
            .find(|(s, _, _)| self.has(id) && s.name() == id.path())
            .ok_or_else(|| LoadError::Missing(id.clone()))?;
        let table: toml::Table = toml::from_str(toml_text)
            .map_err(|e| LoadError::Failed { id: id.clone(), message: format!("the port's marker: {e}") })?;
        Ok(Asset::Record(table))
    }

    fn list(&self, kind: Kind) -> Vec<ContentId> {
        if kind == Kind::Marker { Shape::ALL.iter().map(|s| s.id()).collect() } else { Vec::new() }
    }
}

fn shape_meshes() -> &'static BTreeMap<Shape, Result<Arc<Mesh>, String>> {
    static MESHES: OnceLock<BTreeMap<Shape, Result<Arc<Mesh>, String>>> = OnceLock::new();
    MESHES.get_or_init(|| {
        let limits = ssx_content::model::Limits::default();
        SHAPES
            .iter()
            .map(|(shape, _, gltf)| {
                let file = format!("{}.gltf", shape.name());
                let mesh = ssx_content::model::decode_file(&file, gltf.as_bytes(), &(), &limits)
                    .map_err(|e| e.message)
                    .and_then(|m| merged(&m).ok_or_else(|| "no mesh".to_string()));
                (*shape, mesh.map(Arc::new))
            })
            .collect()
    })
}

fn merged(model: &ssx_content::model::Model) -> Option<Mesh> {
    let lod = model.lods.first()?;
    let mut out = Mesh { name: "marker".into(), normals: Some(Vec::new()), ..Mesh::default() };
    for part in &lod.parts {
        let base = u32::try_from(out.positions.len()).ok()?;
        let m = &part.mesh;
        out.positions.extend_from_slice(&m.positions);
        let normals = out.normals.get_or_insert_with(Vec::new);
        match &m.normals {
            Some(n) if n.len() == m.positions.len() => normals.extend_from_slice(n),
            _ => normals.extend(std::iter::repeat_n([0.0, 1.0, 0.0], m.positions.len())),
        }
        out.indices.extend(m.indices.iter().map(|i| i + base));
    }
    out.bounds = out.position_bounds();
    (!out.indices.is_empty()).then_some(out)
}

#[derive(Clone, Debug)]
pub struct MarkerLook {
    pub id: ContentId,
    pub def: MarkerDef,
    pub mesh: Arc<Mesh>,
    pub source: String,
    pub layer: Layer,
}

impl MarkerLook {
    pub fn vertices(&self) -> u64 {
        self.mesh.positions.len() as u64
    }

    pub fn triangles(&self) -> u64 {
        self.mesh.triangle_count() as u64
    }
}

impl GameContent {
    pub fn marker(&self, id: &ContentId) -> Result<MarkerLook, LoadError> {
        let failed = |message: String| LoadError::Failed { id: id.clone(), message };
        if id.kind() != Kind::Marker {
            return Err(failed("not a marker id".into()));
        }
        let resolved = self.with_registry(|r| r.load(id))?;
        let (source, layer) = (resolved.source.clone(), resolved.layer);
        let table = resolved.asset.record().ok_or_else(|| failed("a marker is a marker.toml record".into()))?;
        let def = from_table(table)
            .map_err(|ps| failed(ps.iter().map(|(l, c, m)| format!("{l}:{c}: {m}")).collect::<Vec<_>>().join("; ")))?;
        let mesh = match &def.look {
            Look::Shape(s) => {
                shape_meshes().get(s).cloned().unwrap_or_else(|| Err("no such shape".into())).map_err(failed)?
            }
            Look::Model(m) => {
                let found = self.model(m, ModelUse::Static)?;
                Arc::new(merged(&found.value).ok_or_else(|| failed(format!("{m}: its LOD 0 has no triangles")))?)
            }
        };
        if mesh.positions.len() > MAX_MARKER_VERTICES || mesh.triangle_count() > MAX_MARKER_TRIANGLES {
            return Err(failed(format!(
                "its mesh has {} vertices and {} triangles (a marker has at most {MAX_MARKER_VERTICES} and \
                 {MAX_MARKER_TRIANGLES})",
                mesh.positions.len(),
                mesh.triangle_count()
            )));
        }
        Ok(MarkerLook { id: id.clone(), def, mesh, source, layer })
    }

    pub fn marker_ids(&self) -> Vec<ContentId> {
        self.with_registry(|r| r.list(Kind::Marker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_base_set_reads_and_its_meshes_decode() {
        for (shape, toml_text, _) in SHAPES {
            assert_eq!(toml_text, ssx_content::marker::shapes::marker_toml(shape), "{}", shape.name());
            let mesh = shape_meshes()[&shape].clone().unwrap();
            assert_eq!(mesh.positions, ssx_content::marker::shapes::mesh(shape).positions, "{}", shape.name());
            let t = PortMarkers.load(&shape.id()).unwrap();
            assert_eq!(from_table(t.record().unwrap()).unwrap().look, Look::Shape(shape));
        }
        assert!(!PortMarkers.has(&"ava:marker/star".parse().unwrap()));
        assert!(!PortMarkers.has(&"ava:sky/ring".parse().unwrap()));
    }
}
