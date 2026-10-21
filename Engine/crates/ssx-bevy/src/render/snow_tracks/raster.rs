use bevy::math::{Vec2, Vec3, Vec4};

use crate::render::snow_tracks::model::{CasterTransforms, Map128, TEXELS, VELDISP_CLEAR, unorm8};

pub fn draw(positions: &[Vec3], triangles: &[[u32; 3]], t: &CasterTransforms) -> (Map128, usize) {
    let mut map = Map128::filled(VELDISP_CLEAR);
    let mut depth = vec![1.0_f32; TEXELS * TEXELS];
    let mut written = 0;
    let views: Vec<Vec3> = positions.iter().map(|&p| t.view(p)).collect();
    let mut attrs: Vec<Option<Vec4>> = vec![None; positions.len()];
    let n = TEXELS as f32;
    for tri in triangles {
        let [i0, i1, i2] = tri.map(|i| i as usize);
        let v = [views[i0], views[i1], views[i2]];
        let lo = v[0].min(v[1]).min(v[2]);
        let hi = v[0].max(v[1]).max(v[2]);
        if hi.x < 0.0 || hi.y < 0.0 || lo.x > n || lo.y > n || hi.z < 0.0 || lo.z > 1.0 {
            continue;
        }
        let (a, b, c) = (v[0].truncate(), v[1].truncate(), v[2].truncate());
        let area = edge(a, b, c);
        if area == 0.0 {
            continue;
        }
        let x0 = lo.x.ceil().max(0.0) as usize;
        let y0 = lo.y.ceil().max(0.0) as usize;
        let x1 = (hi.x.floor() as i64).min(TEXELS as i64 - 1);
        let y1 = (hi.y.floor() as i64).min(TEXELS as i64 - 1);
        if x1 < x0 as i64 || y1 < y0 as i64 {
            continue;
        }
        let attr = [i0, i1, i2].map(|i| *attrs[i].get_or_insert_with(|| t.texel(positions[i])));
        for y in y0..=y1 as usize {
            for x in x0..=x1 as usize {
                let p = Vec2::new(x as f32, y as f32);
                let (w0, w1, w2) = (edge(b, c, p), edge(c, a, p), edge(a, b, p));
                if !(covers(w0, b, c, area) && covers(w1, c, a, area) && covers(w2, a, b, area)) {
                    continue;
                }
                let (l0, l1, l2) = (w0 / area, w1 / area, w2 / area);
                let z = l0 * v[0].z + l1 * v[1].z + l2 * v[2].z;
                let i = y * TEXELS + x;
                if !(0.0..=1.0).contains(&z) || z > depth[i] {
                    continue;
                }
                depth[i] = z;
                let o: Vec4 = attr[0] * l0 + attr[1] * l1 + attr[2] * l2;
                map.texels[i] = [unorm8(o.x), unorm8(o.y), unorm8(o.z), unorm8(o.w)];
                written += 1;
            }
        }
    }
    (map, written)
}

fn edge(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn covers(w: f32, a: Vec2, b: Vec2, area: f32) -> bool {
    let w = w * area.signum();
    if w != 0.0 {
        return w > 0.0;
    }
    let d = (b - a) * area.signum();
    (d.y == 0.0 && d.x > 0.0) || d.y < 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::snow_tracks::model::Caster;
    use bevy::math::Quat;

    #[test]
    fn a_flat_board_at_the_contact_reads_its_depth() {
        let c = Caster {
            orientation: Quat::IDENTITY,
            prev_orientation: Quat::IDENTITY,
            contact: Vec3::new(4.0, 2.0, -1.0),
            prev_contact: Vec3::new(4.0, 2.0, -1.0),
            normal: Vec3::Y,
            depth: 0.25,
            spray: Vec3::ZERO,
        };
        let t = CasterTransforms::new(&c);
        let (x0, x1, z0, z1, y) = (3.2, 4.8, -1.15, -0.85, 2.0);
        let quad = [Vec3::new(x0, y, z0), Vec3::new(x1, y, z0), Vec3::new(x1, y, z1), Vec3::new(x0, y, z1)];
        let (map, written) = draw(&quad, &[[0, 1, 2], [0, 2, 3]], &t);
        assert_eq!(written, 69 * 13);
        assert_eq!(map.at(64, 64), [128, 128, 128, unorm8(0.45)]);
        assert_eq!(map.at(2, 2), VELDISP_CLEAR);
    }
}
