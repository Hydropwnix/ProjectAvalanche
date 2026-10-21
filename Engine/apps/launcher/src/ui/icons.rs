use std::f32::consts::TAU;

use eframe::egui::epaint::PathStroke;
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Mountain,
    Gear,
    Wrench,
}

pub fn draw(painter: &Painter, icon: Icon, rect: Rect, color: Color32) {
    match icon {
        Icon::Mountain => mountain(painter, rect, color),
        Icon::Gear => gear(painter, rect, color),
        Icon::Wrench => wrench(painter, rect, color),
    }
}

fn polygon(painter: &Painter, points: &[Pos2], color: Color32) {
    painter.add(Shape::convex_polygon(points.to_vec(), color, PathStroke::NONE));
}

pub const MOUNTAIN: [&[(f32, f32)]; 3] = [
    &[(0.00, 1.00), (0.47, 0.00), (0.53, 0.44), (0.33, 1.00)],
    &[(0.54, 0.12), (1.00, 1.00), (0.72, 1.00), (0.585, 0.52)],
    &[(0.42, 1.00), (0.535, 0.64), (0.62, 1.00)],
];

pub fn mountain(painter: &Painter, rect: Rect, color: Color32) {
    for facet in MOUNTAIN {
        let points: Vec<Pos2> = facet.iter().map(|&(x, y)| rect.min + vec2(x, y) * rect.size()).collect();
        polygon(painter, &points, color);
    }
}

fn gear(painter: &Painter, rect: Rect, color: Color32) {
    let (c, r) = (rect.center(), rect.width() / 2.0);
    painter.add(Shape::circle_stroke(c, r * 0.50, Stroke::new(r * 0.36, color)));
    for i in 0..8 {
        let a = TAU * i as f32 / 8.0;
        let (out, side) = (Vec2::angled(a), Vec2::angled(a).rot90());
        let (inner, outer, wide, narrow) = (r * 0.62, r * 0.98, r * 0.21, r * 0.15);
        polygon(
            painter,
            &[
                c + out * inner - side * wide,
                c + out * outer - side * narrow,
                c + out * outer + side * narrow,
                c + out * inner + side * wide,
            ],
            color,
        );
    }
}

fn wrench(painter: &Painter, rect: Rect, color: Color32) {
    let s = rect.width();
    let at = |x: f32, y: f32| rect.min + vec2(x, y) * s;
    let head = at(0.68, 0.32);
    let (radius, weight) = (s * 0.20, s * 0.17);
    let open = -TAU / 8.0;
    let arc: Vec<Pos2> =
        (0..=24).map(|i| head + Vec2::angled(open + TAU * (0.14 + 0.72 * i as f32 / 24.0)) * radius).collect();
    painter.add(Shape::line(arc, Stroke::new(weight, color)));
    let (dir, side) = (vec2(-1.0, 1.0).normalized(), vec2(1.0, 1.0).normalized());
    let (from, to, half) = (head + dir * (radius * 0.9), at(0.16, 0.84), s * 0.085);
    polygon(painter, &[from - side * half, from + side * half, to + side * half, to - side * half], color);
    painter.add(Shape::circle_filled(to, half, color));
}

pub fn chevrons(painter: &Painter, right: Pos2, height: f32, weight: f32, color: Color32) {
    let (half, depth) = (height / 2.0, height * 0.42);
    for i in 0..2 {
        let tip = pos2(right.x - i as f32 * (depth + weight * 1.1), right.y);
        let points = vec![pos2(tip.x - depth, tip.y - half), tip, pos2(tip.x - depth, tip.y + half)];
        painter.add(Shape::line(points, Stroke::new(weight, color)));
    }
}

pub fn dot(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    painter.add(Shape::circle_filled(center, radius * 2.1, color.gamma_multiply(0.18)));
    painter.add(Shape::circle_filled(center, radius, color));
}
