use std::collections::HashMap;

use bevy_mikktspace::{Geometry, TangentSpace, generate_tangents};

use super::Mesh;

struct Corners<'a> {
    mesh: &'a Mesh,
    uv: &'a [[f32; 2]],
    normals: &'a [[f32; 3]],
    out: Vec<[f32; 4]>,
}

impl Geometry for Corners<'_> {
    fn num_faces(&self) -> usize {
        self.mesh.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.mesh.positions[self.mesh.indices[face * 3 + vert] as usize]
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.mesh.indices[face * 3 + vert] as usize]
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.uv[self.mesh.indices[face * 3 + vert] as usize]
    }

    fn set_tangent(&mut self, tangent_space: Option<TangentSpace>, face: usize, vert: usize) {
        let [x, y, z, w] = tangent_space.unwrap_or_default().tangent_encoded();
        let t = [x, y, z, -w];
        self.out[face * 3 + vert] = if t.iter().all(|x| x.is_finite()) { t } else { [1.0, 0.0, 0.0, 1.0] };
    }
}

pub fn generate(mesh: &mut Mesh) -> bool {
    let (Some(normals), Some(uv)) = (mesh.normals.clone(), mesh.uv0.clone()) else { return false };
    let mut c = Corners { mesh: &*mesh, uv: &uv, normals: &normals, out: vec![[0.0; 4]; mesh.indices.len()] };
    if generate_tangents(&mut c).is_err() {
        return false;
    }
    let corners = c.out;
    let n = mesh.positions.len();
    let mut tangent: Vec<Option<[f32; 4]>> = vec![None; n];
    let mut copies: Vec<(usize, [f32; 4])> = Vec::new();
    let mut copy_of: HashMap<(usize, [u32; 4]), usize> = HashMap::new();
    for (corner, t) in corners.into_iter().enumerate() {
        let v = mesh.indices[corner] as usize;
        match tangent[v] {
            None => tangent[v] = Some(t),
            Some(have) if have == t => {}
            Some(_) => {
                let index = *copy_of.entry((v, t.map(f32::to_bits))).or_insert_with(|| {
                    copies.push((v, t));
                    n + copies.len() - 1
                });
                mesh.indices[corner] = index as u32;
            }
        }
    }
    dup(&mut mesh.positions, &copies);
    if let Some(a) = mesh.normals.as_mut() {
        dup(a, &copies);
    }
    if let Some(a) = mesh.uv0.as_mut() {
        dup(a, &copies);
    }
    if let Some(a) = mesh.uv1.as_mut() {
        dup(a, &copies);
    }
    if let Some(a) = mesh.colours.as_mut() {
        dup(a, &copies);
    }
    if let Some(s) = mesh.skin.as_mut() {
        dup(&mut s.joints, &copies);
        dup(&mut s.weights, &copies);
    }
    let all: Vec<[f32; 4]> =
        tangent.into_iter().map(|t| t.unwrap_or([1.0, 0.0, 0.0, 1.0])).chain(copies.iter().map(|&(_, t)| t)).collect();
    mesh.tangents = Some(all.iter().map(|t| [t[0], t[1], t[2]]).collect());
    mesh.tangent_signs = Some(all.iter().map(|t| if t[3] < 0.0 { -1.0 } else { 1.0 }).collect());
    true
}

fn dup<T: Copy>(v: &mut Vec<T>, copies: &[(usize, [f32; 4])]) {
    for &(src, _) in copies {
        let x = v[src];
        v.push(x);
    }
}
