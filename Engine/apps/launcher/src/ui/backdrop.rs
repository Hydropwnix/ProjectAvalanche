use eframe::egui::epaint::Mesh;
use eframe::egui::{Color32, Painter, Rect, Shape, pos2, vec2};

use super::layout::{CONTENT_TOP, HEADER_H, HEADER_TOP, Layout};
use super::shapes::{self, Cut};
use super::theme;
use crate::art::{Gallery, Slot};

fn gradient(painter: &Painter, rect: Rect, [tl, tr, br, bl]: [Color32; 4]) {
    let mut mesh = Mesh::default();
    for (pos, color) in
        [(rect.left_top(), tl), (rect.right_top(), tr), (rect.right_bottom(), br), (rect.left_bottom(), bl)]
    {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

fn night(alpha: f32) -> Color32 {
    theme::NIGHT.gamma_multiply(alpha)
}

pub fn draw(painter: &Painter, l: &Layout, gallery: &Gallery) {
    let screen = l.screen;
    match gallery.get(Slot::Background) {
        Some(art) => {
            let uv = shapes::cover_uv(art.size, screen.size(), vec2(0.5, 0.35));
            painter.image(art.texture.id(), screen, uv, Color32::WHITE);
        }
        None => {
            let (top, bottom) = (Color32::from_rgb(0x0A, 0x35, 0x4C), theme::NIGHT);
            gradient(painter, screen, [top, top, bottom, bottom]);
        }
    }
    painter.rect_filled(screen, 0.0, night(0.30));
    let clear = night(0.0);
    let left = Rect::from_min_max(screen.min, pos2(l.canvas.left() + 760.0, screen.bottom()));
    gradient(painter, left, [night(0.86), clear, clear, night(0.86)]);
    let top = Rect::from_min_max(screen.min, pos2(screen.right(), l.canvas.top() + CONTENT_TOP + 24.0));
    gradient(painter, top, [night(0.66), night(0.66), clear, clear]);
    let bottom = Rect::from_min_max(pos2(screen.left(), l.canvas.bottom() - 200.0), screen.max);
    gradient(painter, bottom, [clear, clear, night(0.80), night(0.80)]);
    rail(painter, l);
}

fn rail(painter: &Painter, l: &Layout) {
    let (x0, x1) = (l.screen.left() - 2.0, l.snap(l.screen.left() + 24.0));
    let y = |u: f32| l.snap(l.canvas.top() + u);
    let fill = theme::TAB[0].gamma_multiply(0.30);
    let segments = [
        (l.screen.top() - 2.0, y(HEADER_TOP - 16.0)),
        (y(HEADER_TOP - 8.0), y(HEADER_TOP + HEADER_H + 8.0)),
        (y(CONTENT_TOP), l.screen.bottom() + 2.0),
    ];
    for (top, bottom) in segments {
        let rect = Rect::from_min_max(pos2(x0, top), pos2(x1, bottom));
        let points = shapes::chamfered(rect, Cut { tr: 8.0, ..Default::default() });
        shapes::fill(painter, &points, fill);
        shapes::outline(painter, &points, l.px(1.0), theme::LINE.gamma_multiply(0.7));
    }
}
