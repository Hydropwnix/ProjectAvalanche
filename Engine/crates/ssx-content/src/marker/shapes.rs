use std::f32::consts::TAU;
use std::fmt::Write as _;

use super::Shape;
use crate::model::Mesh;

#[derive(Default)]
struct Build {
    p: Vec<[f32; 3]>,
    n: Vec<[f32; 3]>,
    i: Vec<u32>,
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { [0.0, 1.0, 0.0] }
}

impl Build {
    fn vertex(&mut self, p: [f32; 3], n: [f32; 3]) -> u32 {
        self.p.push(p);
        self.n.push(norm(n));
        (self.p.len() - 1) as u32
    }

    fn triangle(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3]) {
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let k = [self.vertex(a, n), self.vertex(b, n), self.vertex(c, n)];
        self.i.extend(k);
    }

    fn quad(&mut self, a: [f32; 3], b: [f32; 3], c: [f32; 3], d: [f32; 3]) {
        self.triangle(a, b, c);
        self.triangle(a, c, d);
    }

    fn cuboid(&mut self, c: [f32; 3], h: [f32; 3], yaw: f32) {
        let (s, co) = yaw.sin_cos();
        let at = |x: f32, y: f32, z: f32| {
            let (x, z) = (x * h[0], z * h[2]);
            [c[0] + x * co + z * s, c[1] + y * h[1], c[2] - x * s + z * co]
        };
        let faces: [[(f32, f32, f32); 4]; 6] = [
            [(1., -1., -1.), (1., 1., -1.), (1., 1., 1.), (1., -1., 1.)],
            [(-1., -1., 1.), (-1., 1., 1.), (-1., 1., -1.), (-1., -1., -1.)],
            [(-1., 1., -1.), (-1., 1., 1.), (1., 1., 1.), (1., 1., -1.)],
            [(-1., -1., 1.), (-1., -1., -1.), (1., -1., -1.), (1., -1., 1.)],
            [(-1., -1., 1.), (1., -1., 1.), (1., 1., 1.), (-1., 1., 1.)],
            [(1., -1., -1.), (-1., -1., -1.), (-1., 1., -1.), (1., 1., -1.)],
        ];
        for f in faces {
            let q = f.map(|(x, y, z)| at(x, y, z));
            self.quad(q[0], q[1], q[2], q[3]);
        }
    }

    fn cylinder(&mut self, r: f32, y0: f32, y1: f32, n: u32) {
        let base = self.p.len() as u32;
        for k in 0..=n {
            let (s, c) = (TAU * k as f32 / n as f32).sin_cos();
            self.vertex([r * c, y0, r * s], [c, 0.0, s]);
            self.vertex([r * c, y1, r * s], [c, 0.0, s]);
        }
        for k in 0..n {
            let (a, b, c, d) = (base + 2 * k, base + 2 * k + 1, base + 2 * k + 3, base + 2 * k + 2);
            self.i.extend([a, b, c, a, c, d]);
        }
    }

    fn torus(&mut self, c: [f32; 3], big: f32, small: f32, around: u32, tube: u32) {
        let base = self.p.len() as u32;
        for a in 0..=around {
            let (sa, ca) = (TAU * a as f32 / around as f32).sin_cos();
            for t in 0..=tube {
                let (st, ct) = (TAU * t as f32 / tube as f32).sin_cos();
                let n = [ca * ct, sa * ct, st];
                let p = [c[0] + (big + small * ct) * ca, c[1] + (big + small * ct) * sa, c[2] + small * st];
                self.vertex(p, n);
            }
        }
        let row = tube + 1;
        for a in 0..around {
            for t in 0..tube {
                let (k0, k1) = (base + a * row + t, base + (a + 1) * row + t);
                self.i.extend([k0, k1, k1 + 1, k0, k1 + 1, k0 + 1]);
            }
        }
    }

    fn mesh(self, name: &str) -> Mesh {
        let mut m = Mesh { name: name.to_string(), positions: self.p, indices: self.i, ..Mesh::default() };
        m.normals = Some(self.n);
        m.bounds = m.position_bounds();
        m
    }
}

pub fn mesh(shape: Shape) -> Mesh {
    let mut b = Build::default();
    match shape {
        Shape::Ring => b.torus([0.0; 3], 0.5, 0.035, 48, 10),
        Shape::Cylinder => b.cylinder(0.5, 0.0, 1.0, 40),
        Shape::Beam => b.cylinder(0.06, 0.0, 12.0, 16),
        Shape::Gate => {
            b.cuboid([-0.5, 0.5, 0.0], [0.04, 0.5, 0.04], 0.0);
            b.cuboid([0.5, 0.5, 0.0], [0.04, 0.5, 0.04], 0.0);
            b.cuboid([0.0, 0.96, 0.0], [0.54, 0.04, 0.04], 0.0);
        }
        Shape::Arrow => {
            b.cuboid([0.0, 0.675, 0.0], [0.06, 0.325, 0.06], 0.0);
            let (h, y) = (0.2, 0.35);
            let base = [[-h, y, -h], [h, y, -h], [h, y, h], [-h, y, h]];
            for k in 0..4 {
                b.triangle([0.0; 3], base[(k + 1) % 4], base[k]);
            }
            b.quad(base[0], base[1], base[2], base[3]);
        }
        Shape::Chevron => {
            let arm = 0.5f32.hypot(0.5);
            b.cuboid([-0.25, 0.03, 0.0], [0.075, 0.03, arm * 0.5], std::f32::consts::FRAC_PI_4);
            b.cuboid([0.25, 0.03, 0.0], [0.075, 0.03, arm * 0.5], -std::f32::consts::FRAC_PI_4);
        }
    }
    b.mesh(shape.name())
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = u32::from(c[0]) << 16 | u32::from(*c.get(1).unwrap_or(&0)) << 8 | u32::from(*c.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= c.len() {
                out.push(A[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn f(x: f32) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
}

pub fn gltf(shape: Shape) -> String {
    let m = mesh(shape);
    let normals = m.normals.clone().unwrap_or_default();
    let mut bin = Vec::new();
    for p in &m.positions {
        p.iter().for_each(|x| bin.extend(x.to_le_bytes()));
    }
    for n in &normals {
        n.iter().for_each(|x| bin.extend(x.to_le_bytes()));
    }
    for i in &m.indices {
        bin.extend(i.to_le_bytes());
    }
    let (lo, hi) = m.bounds.unwrap_or(([0.0; 3], [0.0; 3]));
    let vbytes = m.positions.len() * 12;
    let (count, tris) = (m.positions.len(), m.indices.len());
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"Project Avalanche marker shapes\"}},\
         \"scene\":0,\"scenes\":[{{\"nodes\":[0]}}],\"nodes\":[{{\"name\":\"{n}\",\"mesh\":0}}],\
         \"meshes\":[{{\"name\":\"{n}\",\"primitives\":[{{\"attributes\":{{\"POSITION\":0,\"NORMAL\":1}},\"indices\":2,\
         \"material\":0}}]}}],\
         \"materials\":[{{\"name\":\"marker\",\"pbrMetallicRoughness\":{{\"baseColorFactor\":[1.0,1.0,1.0,1.0]}}}}],\
         \"accessors\":[\
         {{\"bufferView\":0,\"componentType\":5126,\"count\":{count},\"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}},\
         {{\"bufferView\":1,\"componentType\":5126,\"count\":{count},\"type\":\"VEC3\"}},\
         {{\"bufferView\":2,\"componentType\":5125,\"count\":{tris},\"type\":\"SCALAR\"}}],\
         \"bufferViews\":[\
         {{\"buffer\":0,\"byteOffset\":0,\"byteLength\":{vbytes}}},\
         {{\"buffer\":0,\"byteOffset\":{vbytes},\"byteLength\":{vbytes}}},\
         {{\"buffer\":0,\"byteOffset\":{ib},\"byteLength\":{ibl}}}],\
         \"buffers\":[{{\"byteLength\":{bl},\"uri\":\"data:application/octet-stream;base64,{data}\"}}]}}",
        f(lo[0]),
        f(lo[1]),
        f(lo[2]),
        f(hi[0]),
        f(hi[1]),
        f(hi[2]),
        n = shape.name(),
        ib = vbytes * 2,
        ibl = tris * 4,
        bl = bin.len(),
        data = base64(&bin),
    );
    s
}

pub fn marker_toml(shape: Shape) -> String {
    let (size, colour, glow, facing, anim) = match shape {
        Shape::Ring => (6.0, "#40C8FFD0", 1.5, "world", "spin = 0.0\npulse = 0.06\npulse_period = 1.2\n"),
        Shape::Cylinder => (4.0, "#FFD04080", 1.0, "world", "pulse = 0.05\npulse_period = 1.5\n"),
        Shape::Arrow => (2.0, "#FF8020E0", 1.5, "camera", "bob = 0.4\nbob_period = 1.6\n"),
        Shape::Chevron => (3.0, "#FFE040E0", 1.2, "world", "pulse = 0.1\npulse_period = 0.8\n"),
        Shape::Beam => (4.0, "#80FFB0A0", 2.0, "world", "spin = 0.0\n"),
        Shape::Gate => (8.0, "#FF6030E0", 1.5, "world", "spin = 0.0\n"),
    };
    format!(
        "shape = \"{n}\"\nsize = {}\ncolor = \"{colour}\"\nglow = {}\nfacing = \"{facing}\"\ndraw_distance = 600.0\n\n\
         [animation]\n{anim}",
        f(size),
        f(glow),
        n = shape.name(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_back() {
        for n in 0..10usize {
            let bytes: Vec<u8> = (0..n as u8).map(|b| b.wrapping_mul(37)).collect();
            assert_eq!(crate::model::base64::decode(base64(&bytes).as_bytes(), 1 << 20).unwrap(), bytes);
        }
    }
}
