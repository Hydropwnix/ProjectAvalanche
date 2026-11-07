use eframe::egui::{self, Color32, CursorIcon, Rect, Response, RichText, Sense, Ui, pos2, vec2};

use super::layout::CUT_BUTTON;
use super::shapes::{self, Cut};
use super::text::{self, Fit, Style};
use super::theme::{self, Face, size};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Plain,
}

const BUTTON_H: f32 = 32.0;
const BUTTON_PAD: f32 = 18.0;

pub fn button(ui: &mut Ui, label: &str, kind: Kind, enabled: bool) -> Response {
    let ppp = ui.pixels_per_point();
    let style = Style::new(Face::Title, size::SMALL, theme::TEXT).spaced(1.0);
    let galley = text::layout(ui.painter(), &label.to_uppercase(), &style, Fit::Free);
    let width = text::width(&galley, &style) + 2.0 * BUTTON_PAD;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut response) = ui.allocate_exact_size(vec2(width, BUTTON_H), sense);
    if enabled {
        response = response.on_hover_cursor(CursorIcon::PointingHand);
    }
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let snap = |v: f32| (v * ppp).round() / ppp;
    let rect =
        Rect::from_min_max(pos2(snap(rect.left()), snap(rect.top())), pos2(snap(rect.right()), snap(rect.bottom())));
    let points = shapes::chamfered(rect, Cut::diagonal(CUT_BUTTON));
    let painter = ui.painter();
    let hot = enabled && (response.hovered() || response.has_focus());
    let down = enabled && response.is_pointer_button_down_on();
    if hot {
        shapes::glow(painter, &points, 8.0, theme::CYAN.gamma_multiply(0.30));
    }
    let fill = match (kind, hot, down) {
        (_, _, true) => Color32::from_rgb(0x0E, 0x76, 0x8A),
        (Kind::Primary, true, _) => Color32::from_rgb(0x0B, 0x66, 0x78),
        (Kind::Primary, false, _) => theme::TAB[1],
        (Kind::Plain, true, _) => Color32::from_rgb(0x0A, 0x40, 0x50),
        (Kind::Plain, false, _) => Color32::from_rgb(0x06, 0x26, 0x33),
    };
    let dim = if enabled { 1.0 } else { 0.40 };
    shapes::fill(painter, &points, fill.gamma_multiply(dim));
    let line = if hot || kind == Kind::Primary { theme::CYAN } else { theme::LINE };
    shapes::outline(painter, &points, (1.0 * ppp).round().max(1.0) / ppp, line.gamma_multiply(dim));
    let base = rect.center().y + size::SMALL * theme::CAP_HEIGHT / 2.0;
    let colour = if hot || kind == Kind::Primary { theme::TEXT } else { theme::TEXT_SOFT };
    let galley = text::layout(painter, &label.to_uppercase(), &style.color(colour.gamma_multiply(dim)), Fit::Free);
    text::paint(painter, pos2(rect.left() + BUTTON_PAD, base), galley);
    response
}

pub fn heading(ui: &mut Ui, label: &str) {
    let style = Style::new(Face::Title, size::CAPS, theme::CYAN_SOFT).spaced(1.6);
    let galley = text::layout(ui.painter(), &label.to_uppercase(), &style, Fit::Free);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let base = rect.top() + 6.0 + size::CAPS * theme::CAP_HEIGHT;
    let ink = text::paint(ui.painter(), pos2(rect.left(), base), galley);
    let ppp = ui.pixels_per_point();
    let y = ((base - size::CAPS * theme::CAP_HEIGHT / 2.0) * ppp).round() / ppp;
    let rule = Rect::from_min_max(pos2(ink.right() + 6.0, y), pos2(rect.right(), y + 1.0 / ppp));
    ui.painter().rect_filled(rule, 0.0, theme::LINE_FAINT);
}

pub fn field(ui: &mut Ui, label: &str) {
    ui.label(RichText::new(label).color(theme::TEXT_DIM));
}

pub fn paragraph(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(theme::TEXT_SOFT));
}

pub fn note(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(shown(text)).color(theme::TEXT_DIM));
}

pub fn shown(text: &str) -> String {
    ssx_core::text::shown_lines(text, ssx_core::text::LINE_CHARS).to_string()
}

pub fn path(ui: &mut Ui, path: &std::path::Path) {
    let text = ssx_core::text::shown_max(&path.to_string_lossy(), ssx_core::text::PATH_CHARS).to_string();
    let label = egui::Label::new(RichText::new(&text).monospace().color(theme::TEXT)).truncate();
    ui.add(label).on_hover_text(text);
}

pub fn status(ui: &mut Ui, colour: Color32, text: impl Into<String>) {
    ui.label(RichText::new(shown(&text.into())).color(colour));
}

pub fn progress(ui: &mut Ui, fraction: f32) {
    let ppp = ui.pixels_per_point();
    let snap = |v: f32| (v * ppp).round() / ppp;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::hover());
    let rect =
        Rect::from_min_max(pos2(snap(rect.left()), snap(rect.top())), pos2(snap(rect.right()), snap(rect.bottom())));
    let track = shapes::chamfered(rect, Cut::diagonal(4.0));
    shapes::fill(ui.painter(), &track, Color32::from_rgb(0x02, 0x10, 0x18));
    let done =
        Rect::from_min_max(rect.min, pos2(snap(rect.left() + rect.width() * fraction.clamp(0.0, 1.0)), rect.bottom()));
    if done.width() > 8.0 {
        let fill = shapes::chamfered(done, Cut { bl: 4.0, ..Default::default() });
        let (x0, w) = (rect.left(), rect.width());
        shapes::fill_with(ui.painter(), &fill, 1.0 / ppp, |p| {
            theme::TAB[1].lerp_to_gamma(theme::CYAN, ((p.x - x0) / w).clamp(0.0, 1.0))
        });
    }
    shapes::outline(ui.painter(), &track, 1.0 / ppp, theme::LINE);
}
