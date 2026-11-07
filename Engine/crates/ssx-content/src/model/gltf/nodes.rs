use std::ops::Range;

use super::{FileReader, Held, NodeAt, Placing, Reader};
use crate::model::{Extras, Lod, Model, Transform};

pub(in crate::model) struct NodeMeshes {
    pub(in crate::model) model: Model,
    pub(in crate::model) ranges: Vec<Range<usize>>,
    pub(in crate::model) held_bytes: u64,
    pub(in crate::model) drawn_image_bytes: u64,
}

impl Reader<'_> {
    pub(in crate::model) fn node_meshes(
        &mut self,
        nodes: &[usize],
        dir: &str,
        files: &dyn FileReader,
        held: &mut Held<'_>,
    ) -> Result<NodeMeshes, String> {
        let root = self.root;
        let mut lods = vec![Lod::default()];
        let (mut collision, mut ranges) = (Vec::new(), Vec::with_capacity(nodes.len().min(root.nodes.len())));
        for &i in nodes {
            let node = root.nodes.get(i).ok_or_else(|| format!("nodes[{i}]: no such node"))?;
            let m = node.mesh.ok_or_else(|| format!("nodes[{i}]: no mesh"))?.value();
            let start = lods[0].parts.len();
            let place = NodeAt {
                placing: Placing { xf: Transform::IDENTITY, name: node.name.as_deref().unwrap_or("") },
                lod: 0,
                in_collision: false,
                extras: Extras::default(),
                skin: None,
            };
            self.mesh(m, &place, &mut lods, &mut collision)?;
            ranges.push(start..lods[0].parts.len());
        }
        let (materials, images) = self.materials(dir, files, held)?;
        let drawn_image_bytes = self.check_drawn_images(&materials, &images)?;
        let notes = std::mem::take(&mut self.notes);
        let model = Model { lods, collision: None, materials, images, skins: Vec::new(), source: None, notes };
        Ok(NodeMeshes { model, ranges, held_bytes: held.total, drawn_image_bytes })
    }
}
