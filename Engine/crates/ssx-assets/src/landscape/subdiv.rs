use ssx_formats::subd::SubdMesh;

use crate::terrain::TriMesh;

pub const GAME_LEVELS: usize = 2;

#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub position: [f64; 3],
    pub attrs: [f64; 8],
}

#[derive(Clone, Copy, Debug)]
pub struct Face {
    pub reversed_mask: u8,
    pub len: u8,
    pub edges: [u32; 4],
    pub unk_a: u8,
    pub unk_flags: u8,
}

#[derive(Clone, Debug, Default)]
pub struct SubdLevel {
    pub vertices: Vec<Vertex>,
    pub edges: Vec<[u32; 2]>,
    pub faces: Vec<Face>,
}

impl Face {
    fn edges(&self) -> &[u32] {
        &self.edges[..self.len as usize]
    }

    fn corner(&self, edges: &[[u32; 2]], i: usize) -> u32 {
        let [a, b] = edges[self.edges[i] as usize];
        if self.reversed_mask >> i & 1 == 1 { b } else { a }
    }
}

impl SubdLevel {
    pub fn from_subd(mesh: &SubdMesh) -> Self {
        let vertices = mesh
            .vertices
            .iter()
            .map(|v| {
                let mut a = v.attributes;
                crate::terrain_vertex::normalise_layer_weights(&mut a);
                Vertex { position: v.position.map(f64::from), attrs: a.map(f64::from) }
            })
            .collect();
        let edges = mesh.edges.iter().map(|e| e.map(u32::from)).collect();
        let faces = mesh
            .faces
            .iter()
            .zip(&mesh.unk_face_flags)
            .map(|(f, &flags)| {
                let mut edges = [u32::MAX; 4];
                for (dst, &e) in edges.iter_mut().zip(&f.edges) {
                    *dst = e as u32;
                }
                Face {
                    reversed_mask: f.reversed_mask,
                    len: f.edges.len() as u8,
                    edges,
                    unk_a: f.unk_a,
                    unk_flags: flags,
                }
            })
            .collect();
        Self { vertices, edges, faces }
    }

    pub fn refined(mut self, levels: usize) -> Self {
        for _ in 0..levels {
            self = self.subdivide();
        }
        self
    }
}

#[derive(Clone, Copy, Default)]
struct Acc {
    n: f64,
    sum: [f64; 3],
}

impl Acc {
    fn add(&mut self, p: [f64; 3]) {
        self.n += 1.0;
        self.sum = add(self.sum, p);
    }
}

impl SubdLevel {
    pub fn subdivide(&self) -> SubdLevel {
        let (nv, ne, nf) = (self.vertices.len(), self.edges.len(), self.faces.len());
        let mut face_acc = vec![Acc::default(); nv];
        let mut edge_acc = vec![Acc::default(); ne];
        let mut mid_acc = vec![Acc::default(); nv];
        let mut border_acc = vec![Acc::default(); nv];

        let mut out = SubdLevel { vertices: Vec::with_capacity(nv + ne + nf), ..Default::default() };

        for f in &self.faces {
            let corners: Vec<&Vertex> =
                (0..f.len as usize).map(|i| &self.vertices[f.corner(&self.edges, i) as usize]).collect();
            let inv = 1.0 / corners.len() as f64;
            let mut fp = Vertex { position: [0.0; 3], attrs: [0.0; 8] };
            for c in &corners {
                fp.position = add(fp.position, c.position);
                for (a, b) in fp.attrs.iter_mut().zip(c.attrs) {
                    *a += b;
                }
            }
            fp.position = scale(fp.position, inv);
            fp.attrs = fp.attrs.map(|a| a * inv);
            for i in 0..f.len as usize {
                face_acc[f.corner(&self.edges, i) as usize].add(fp.position);
                edge_acc[f.edges[i] as usize].add(fp.position);
            }
            out.vertices.push(fp);
        }

        for (e, &[a, b]) in self.edges.iter().enumerate() {
            let (va, vb) = (&self.vertices[a as usize], &self.vertices[b as usize]);
            let mid = scale(add(va.position, vb.position), 0.5);
            mid_acc[a as usize].add(mid);
            mid_acc[b as usize].add(mid);
            let acc = edge_acc[e];
            let position = if acc.n == 1.0 {
                border_acc[a as usize].add(vb.position);
                border_acc[b as usize].add(va.position);
                mid
            } else {
                scale(add(add(va.position, vb.position), acc.sum), 1.0 / (acc.n + 2.0))
            };
            let mut attrs = [0.0; 8];
            for (i, v) in attrs.iter_mut().enumerate() {
                *v = (va.attrs[i] + vb.attrs[i]) * 0.5;
            }
            out.vertices.push(Vertex { position, attrs });
        }

        for (v, vert) in self.vertices.iter().enumerate() {
            let n = mid_acc[v].n;
            let p = vert.position;
            let position = if n == face_acc[v].n {
                if n == 0.0 {
                    p
                } else {
                    let r = scale(mid_acc[v].sum, 2.0 / n);
                    let f = scale(face_acc[v].sum, 1.0 / n);
                    scale(add(add(r, f), scale(p, n - 3.0)), 1.0 / n)
                }
            } else {
                scale(add(border_acc[v].sum, scale(p, 6.0)), 1.0 / (border_acc[v].n + 6.0))
            };
            out.vertices.push(Vertex { position, attrs: vert.attrs });
        }

        let (fp0, ep0, vp0) = (0u32, nf as u32, (nf + ne) as u32);
        for (e, &[a, b]) in self.edges.iter().enumerate() {
            out.edges.push([vp0 + a, ep0 + e as u32]);
            out.edges.push([vp0 + b, ep0 + e as u32]);
        }
        let spoke0 = out.edges.len() as u32;
        let mut spoke_start = Vec::with_capacity(nf);
        for (fi, f) in self.faces.iter().enumerate() {
            spoke_start.push(out.edges.len() as u32 - spoke0);
            for &e in f.edges() {
                out.edges.push([fp0 + fi as u32, ep0 + e]);
            }
        }
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.len as usize;
            for k in 0..n {
                let k1 = (k + 1) % n;
                let bit = |i: usize| (f.reversed_mask >> i & 1) as u32;
                let edges = [
                    spoke0 + spoke_start[fi] + k as u32,
                    f.edges[k] * 2 + 1 - bit(k),
                    f.edges[k1] * 2 + bit(k1),
                    spoke0 + spoke_start[fi] + k1 as u32,
                ];
                out.faces.push(Face { reversed_mask: 0b1010, len: 4, edges, unk_a: f.unk_a, unk_flags: f.unk_flags });
            }
        }
        out
    }

    pub fn face_loops(&self) -> impl Iterator<Item = Vec<u32>> + '_ {
        self.faces.iter().map(|f| (0..f.len as usize).map(|i| f.corner(&self.edges, i)).collect())
    }

    pub fn to_trimesh(&self) -> TriMesh {
        let positions: Vec<[f32; 3]> = self.vertices.iter().map(|v| v.position.map(|c| c as f32)).collect();
        let mut normals = vec![[0.0f64; 3]; positions.len()];
        let mut indices = Vec::with_capacity(self.faces.len() * 6);
        let drawn = self.faces.iter().zip(self.face_loops()).filter(|(f, _)| !crate::terrain::is_hidden_face(f.unk_a));
        for (_, face) in drawn {
            for k in 1..face.len().saturating_sub(1) {
                let tri = [face[0], face[k], face[k + 1]];
                let [a, b, c] = tri.map(|i| self.vertices[i as usize].position);
                let n = cross(sub(b, a), sub(c, a));
                for &i in &tri {
                    normals[i as usize] = add(normals[i as usize], n);
                }
                indices.extend_from_slice(&tri);
            }
        }
        let normals = normals
            .into_iter()
            .map(|n| {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len > 0.0 { n.map(|c| (c / len) as f32) } else { [0.0, 1.0, 0.0] }
            })
            .collect();
        let attributes = self.vertices.iter().map(|v| v.attrs.map(|a| a as f32)).collect();
        TriMesh { positions, normals, attributes, indices }
    }
}

pub fn refined_file_trimesh(file: &ssx_formats::subd::SubdFile, levels: usize) -> TriMesh {
    let mut out = TriMesh::default();
    for mesh in &file.meshes {
        out.append(&SubdLevel::from_subd(mesh).refined(levels).to_trimesh());
    }
    out
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> SubdLevel {
        let p = |x: f64, y: f64, z: f64| Vertex { position: [x, y, z], attrs: [0.0; 8] };
        let vertices = vec![
            p(-1., -1., -1.),
            p(1., -1., -1.),
            p(1., 1., -1.),
            p(-1., 1., -1.),
            p(-1., -1., 1.),
            p(1., -1., 1.),
            p(1., 1., 1.),
            p(-1., 1., 1.),
        ];
        let quads: [[u32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [1, 2, 6, 5], [2, 3, 7, 6], [3, 0, 4, 7]];
        let mut edges: Vec<[u32; 2]> = Vec::new();
        let mut faces = Vec::new();
        for q in quads {
            let mut f = Face { reversed_mask: 0, len: 4, edges: [0; 4], unk_a: 0, unk_flags: 0 };
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                let idx = match edges.iter().position(|e| *e == [a, b] || *e == [b, a]) {
                    Some(idx) => idx,
                    None => {
                        edges.push([a, b]);
                        edges.len() - 1
                    }
                };
                if edges[idx] == [b, a] {
                    f.reversed_mask |= 1 << i;
                }
                f.edges[i] = idx as u32;
            }
            faces.push(f);
        }
        SubdLevel { vertices, edges, faces }
    }

    fn loops_close(m: &SubdLevel) -> bool {
        m.faces.iter().all(|f| {
            let n = f.len as usize;
            (0..n).all(|i| {
                let [a, b] = m.edges[f.edges[i] as usize];
                let end = if f.reversed_mask >> i & 1 == 1 { a } else { b };
                end == f.corner(&m.edges, (i + 1) % n)
            })
        })
    }

    #[test]
    fn cube_counts_and_corner_rule() {
        let c = cube();
        assert!(loops_close(&c));
        let s = c.subdivide();
        assert_eq!((s.vertices.len(), s.edges.len(), s.faces.len()), (8 + 6 + 12, 2 * 12 + 24, 24));
        assert!(loops_close(&s));
        let corner = s.vertices[6 + 12 + 6].position;
        for c in corner {
            assert!((c - 5.0 / 9.0).abs() < 1e-12, "{corner:?}");
        }
        let s2 = s.subdivide();
        assert_eq!(s2.faces.len(), 96);
        assert!(loops_close(&s2));
    }

    #[test]
    fn open_quad_keeps_boundary_rule() {
        let mut c = cube();
        c.faces.truncate(1);
        let s = c.subdivide();
        let v0 = s.vertices[1 + 12].position;
        let expect = [(-6. - 1. + 1.) / 8., (-6. + 1. - 1.) / 8., -1.];
        assert_eq!(v0, expect);
    }
}
