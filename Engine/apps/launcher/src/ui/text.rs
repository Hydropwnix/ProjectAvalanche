use std::sync::Arc;

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{Color32, Galley, Painter, Pos2, Rect, pos2};

use super::theme::Face;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub face: Face,
    pub size: f32,
    pub color: Color32,
    pub spacing: f32,
    pub leading: Option<f32>,
}

impl Style {
    pub const fn new(face: Face, size: f32, color: Color32) -> Self {
        Self { face, size, color, spacing: 0.0, leading: None }
    }

    pub const fn spaced(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    pub const fn leading(mut self, leading: f32) -> Self {
        self.leading = Some(leading);
        self
    }

    pub const fn color(mut self, color: Color32) -> Self {
        self.color = color;
        self
    }

    fn format(&self) -> TextFormat {
        TextFormat {
            font_id: self.face.id(self.size),
            color: self.color,
            extra_letter_spacing: self.spacing,
            line_height: self.leading,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    Free,
    Wrap(f32),
    Cut(f32),
}

pub fn layout(painter: &Painter, text: &str, style: &Style, fit: Fit) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), style.format());
    job.wrap = match fit {
        Fit::Free => TextWrapping::no_max_width(),
        Fit::Wrap(w) => TextWrapping::wrap_at_width(w),
        Fit::Cut(w) => TextWrapping { max_rows: 1, break_anywhere: true, ..TextWrapping::truncate_at_width(w) },
    };
    painter.layout_job(job)
}

pub fn baseline(galley: &Galley) -> f32 {
    galley.rows.first().map_or(0.0, |r| r.pos.y + r.glyphs.first().map_or(0.0, |g| g.pos.y))
}

pub fn width(galley: &Galley, style: &Style) -> f32 {
    (galley.rect.width() - style.spacing).max(0.0)
}

fn snapped(painter: &Painter, p: Pos2) -> Pos2 {
    let ppp = painter.pixels_per_point();
    pos2((p.x * ppp).round() / ppp, (p.y * ppp).round() / ppp)
}

pub fn paint(painter: &Painter, at: Pos2, galley: Arc<Galley>) -> Rect {
    let pos = snapped(painter, pos2(at.x, at.y - baseline(&galley)));
    let rect = Rect::from_min_size(pos, galley.rect.size());
    painter.galley(pos, galley, Color32::PLACEHOLDER);
    rect
}

pub fn draw(painter: &Painter, at: Pos2, text: &str, style: &Style) -> Rect {
    paint(painter, at, layout(painter, text, style, Fit::Free))
}

pub fn draw_right(painter: &Painter, at: Pos2, text: &str, style: &Style) -> Rect {
    let galley = layout(painter, text, style, Fit::Free);
    let w = width(&galley, style);
    paint(painter, pos2(at.x - w, at.y), galley)
}

pub fn draw_fit(painter: &Painter, at: Pos2, text: &str, style: &Style, fit: Fit) -> Rect {
    paint(painter, at, layout(painter, text, style, fit))
}
