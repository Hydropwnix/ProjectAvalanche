use eframe::egui::{self, Align, CursorIcon, Painter, Rect, Sense, Ui, UiBuilder, pos2};

use super::layout::{CUT_HERO, Layout, PAD};
use super::shapes::{self, Cut};
use super::text::{self, Fit, Style};
use super::theme::{self, Face, size};

pub const HEAD_H: f32 = 64.0;
const TAB_PAD: f32 = 14.0;

pub fn frame(painter: &Painter, l: &Layout, title: &str) -> Rect {
    let rect = l.content;
    let points = shapes::chamfered(rect, Cut::diagonal(CUT_HERO));
    shapes::glow(painter, &points, 14.0, theme::CYAN.gamma_multiply(0.16));
    shapes::fill(painter, &points, theme::PANEL_DEEP);
    shapes::outline(painter, &points, l.px(1.0), theme::CYAN.gamma_multiply(0.70));
    let base = rect.top() + HEAD_H / 2.0 + size::TITLE * theme::CAP_HEIGHT / 2.0;
    text::draw(painter, pos2(rect.left() + PAD, base), title, &Style::new(Face::Bold, size::TITLE, theme::TEXT));
    let (rule, line) = (l.snap(rect.top() + HEAD_H), l.px(1.0));
    let across = Rect::from_min_max(pos2(rect.left() + PAD, rule - line), pos2(rect.right() - PAD, rule));
    painter.rect_filled(across, 0.0, theme::LINE);
    Rect::from_min_max(pos2(rect.left() + PAD, rule + 16.0), pos2(rect.right() - PAD, rect.bottom() - PAD))
}

pub fn tabs<T: Copy + PartialEq>(ui: &mut Ui, l: &Layout, all: &[(T, &str)], current: &mut T) {
    let painter = ui.painter().clone();
    let rect = l.content;
    let rule = l.snap(rect.top() + HEAD_H);
    let base = rect.top() + HEAD_H / 2.0 + size::CAPS * theme::CAP_HEIGHT / 2.0 + 2.0;
    let style = Style::new(Face::Title, size::CAPS, theme::TEXT_DIM).spaced(1.4);
    let mut right = rect.right() - PAD;
    for (i, (tab, label)) in all.iter().enumerate().rev() {
        let galley = text::layout(&painter, label, &style, Fit::Free);
        let width = text::width(&galley, &style) + 2.0 * TAB_PAD;
        let cell = Rect::from_min_max(pos2(l.snap(right - width), rect.top() + 12.0), pos2(l.snap(right), rule));
        let response =
            ui.interact(cell, ui.id().with(("tab", i)), Sense::CLICK).on_hover_cursor(CursorIcon::PointingHand);
        if response.clicked() {
            *current = *tab;
        }
        let colour = match (*current == *tab, response.hovered()) {
            (true, _) => theme::CYAN,
            (false, true) => theme::TEXT,
            (false, false) => theme::TEXT_DIM,
        };
        text::draw(&painter, pos2(cell.left() + TAB_PAD, base), label, &style.color(colour));
        if *current == *tab {
            let under = Rect::from_min_max(pos2(cell.left(), rule - l.px(2.0)), pos2(cell.right(), rule));
            painter.rect_filled(under, 0.0, theme::CYAN);
        }
        right -= width;
    }
}

pub fn body(ui: &mut Ui, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    let builder = UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(Align::Min)).id_salt(id);
    ui.scope_builder(builder, |ui| {
        ui.set_clip_rect(rect.expand2(egui::vec2(4.0, 2.0)));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    });
}
