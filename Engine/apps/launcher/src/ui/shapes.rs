use eframe::egui::epaint::{Mesh, PathShape, PathStroke, Vertex, WHITE_UV};
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, TextureId, Vec2, pos2};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Cut {
    pub tl: f32,
    pub tr: f32,
    pub br: f32,
    pub bl: f32,
}

impl Cut {
    pub fn diagonal(size: f32) -> Self {
        Self { tr: size, bl: size, ..Default::default() }
    }
}

pub fn chamfered(rect: Rect, cut: Cut) -> Vec<Pos2> {
    let (l, t, r, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let mut p = Vec::with_capacity(8);
    let mut corner = |c: f32, a: Pos2, b: Pos2, square: Pos2| {
        if c > 0.0 { p.extend([a, b]) } else { p.push(square) }
    };
    corner(cut.tl, pos2(l, t + cut.tl), pos2(l + cut.tl, t), pos2(l, t));
    corner(cut.tr, pos2(r - cut.tr, t), pos2(r, t + cut.tr), pos2(r, t));
    corner(cut.br, pos2(r, b - cut.br), pos2(r - cut.br, b), pos2(r, b));
    corner(cut.bl, pos2(l + cut.bl, b), pos2(l, b - cut.bl), pos2(l, b));
    p
}

pub fn tab(rect: Rect, tip: f32) -> Vec<Pos2> {
    let (l, t, r, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    vec![pos2(l, t), pos2(r, t), pos2(r + tip, rect.center().y), pos2(r, b), pos2(l, b)]
}

pub fn offset(points: &[Pos2], d: f32) -> Vec<Pos2> {
    let n = points.len();
    let normal = |a: Pos2, b: Pos2| {
        let e = (b - a).normalized();
        Vec2::new(e.y, -e.x)
    };
    (0..n)
        .map(|i| {
            let (prev, here, next) = (points[(i + n - 1) % n], points[i], points[(i + 1) % n]);
            let (n1, n2) = (normal(prev, here), normal(here, next));
            here + (n1 + n2) * (d / (1.0 + n1.dot(n2)).max(0.2))
        })
        .collect()
}

fn vertex(pos: Pos2, uv: Pos2, color: Color32) -> Vertex {
    Vertex { pos, uv, color }
}

fn polygon_mesh(points: &[Pos2], texture: TextureId, feather: f32, paint: impl Fn(Pos2) -> (Pos2, Color32)) -> Mesh {
    let n = points.len() as u32;
    let (inner, outer) = (offset(points, -feather / 2.0), offset(points, feather / 2.0));
    let mut mesh = Mesh::with_texture(texture);
    for p in &inner {
        let (uv, color) = paint(*p);
        mesh.vertices.push(vertex(*p, uv, color));
    }
    for p in &outer {
        mesh.vertices.push(vertex(*p, paint(*p).0, Color32::TRANSPARENT));
    }
    for i in 1..n - 1 {
        mesh.add_triangle(0, i, i + 1);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(i, n + i, n + j);
        mesh.add_triangle(i, n + j, j);
    }
    mesh
}

pub fn fill(painter: &Painter, points: &[Pos2], color: Color32) {
    painter.add(Shape::convex_polygon(points.to_vec(), color, PathStroke::NONE));
}

pub fn fill_with(painter: &Painter, points: &[Pos2], feather: f32, color: impl Fn(Pos2) -> Color32) {
    painter.add(Shape::mesh(polygon_mesh(points, TextureId::default(), feather, |p| (WHITE_UV, color(p)))));
}

pub fn image(
    painter: &Painter,
    points: &[Pos2],
    rect: Rect,
    texture: TextureId,
    uv: Rect,
    tint: Color32,
    feather: f32,
) {
    let map = |p: Pos2| {
        let t = (p - rect.min) / rect.size();
        (pos2(uv.min.x + t.x * uv.width(), uv.min.y + t.y * uv.height()), tint)
    };
    painter.add(Shape::mesh(polygon_mesh(points, texture, feather, map)));
}

pub fn outline(painter: &Painter, points: &[Pos2], width: f32, color: Color32) {
    painter.add(PathShape::closed_line(points.to_vec(), PathStroke::new(width, color).inside()));
}

pub fn glow(painter: &Painter, points: &[Pos2], radius: f32, color: Color32) {
    let n = points.len() as u32;
    let rings = [(0.0, 1.0), (radius * 0.3, 0.38), (radius, 0.0)];
    let mut mesh = Mesh::default();
    for (d, alpha) in rings {
        for p in offset(points, d) {
            mesh.colored_vertex(p, color.gamma_multiply(alpha));
        }
    }
    for ring in 0..2 {
        for i in 0..n {
            let (a, b) = (ring * n + i, ring * n + (i + 1) % n);
            mesh.add_triangle(a, a + n, b + n);
            mesh.add_triangle(a, b + n, b);
        }
    }
    painter.add(Shape::mesh(mesh));
}

pub fn cover_uv(texture: Vec2, target: Vec2, focus: Vec2) -> Rect {
    let scale = (target.x / texture.x).max(target.y / texture.y);
    let size = Vec2::new(target.x / (texture.x * scale), target.y / (texture.y * scale)).min(Vec2::splat(1.0));
    let min = ((Vec2::splat(1.0) - size) * focus).max(Vec2::ZERO);
    Rect::from_min_size(min.to_pos2(), size)
}
