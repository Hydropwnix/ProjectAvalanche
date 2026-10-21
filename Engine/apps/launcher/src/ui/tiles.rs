use eframe::egui::{Color32, Painter, Rect, Vec2, pos2, vec2};

use super::icons::{self, Icon};
use super::layout::{CUT_HERO, CUT_TILE, Layout, PAD};
use super::shapes::{self, Cut};
use super::text::{self, Fit, Style};
use super::theme::{self, Face, size};
use crate::art::{Art, Source};

fn picture(painter: &Painter, l: &Layout, points: &[eframe::egui::Pos2], rect: Rect, art: Option<&Art>, focus: Vec2) {
    match art {
        Some(art) => {
            let uv = shapes::cover_uv(art.size, rect.size(), focus);
            shapes::image(painter, points, rect, art.texture.id(), uv, Color32::WHITE, 1.0 / l.ppp);
        }
        None => shapes::fill(painter, points, Color32::from_rgb(0x06, 0x2A, 0x3A)),
    }
}

pub struct Hero<'a> {
    pub title: &'a str,
    pub line: (&'a str, Color32),
    pub tab: (&'a str, Color32),
    pub hovered: bool,
    pub enabled: bool,
}

const BANNER_H: f32 = 80.0;
const BANNER_LEAN: f32 = 56.0;
const TAB_W: f32 = 168.0;
const TAB_H: f32 = 28.0;

pub fn hero(painter: &Painter, l: &Layout, rect: Rect, art: Option<&Art>, hero: &Hero) {
    let points = shapes::chamfered(rect, Cut::diagonal(CUT_HERO));
    let lit = hero.hovered && hero.enabled;
    shapes::glow(
        painter,
        &points,
        if lit { 20.0 } else { 14.0 },
        theme::CYAN.gamma_multiply(if lit { 0.50 } else { 0.26 }),
    );
    picture(painter, l, &points, rect, art, vec2(0.5, 0.5));
    let (left, right, top, bottom) = (rect.left(), rect.right(), rect.top(), rect.bottom());

    let title = text::layout(painter, hero.title, &Style::new(Face::Bold, size::HERO_TITLE, theme::TEXT), Fit::Free);
    let line_style = Style::new(Face::Text, size::HERO_TEXT, hero.line.1);
    let room = rect.width() - 2.0 * PAD - BANNER_LEAN - PAD;
    let line = text::layout(painter, hero.line.0, &line_style, Fit::Cut(room));
    let wide = title.rect.width().max(line.rect.width()) + 2.0 * PAD + BANNER_LEAN;
    let banner_right = l.snap(left + wide.max(rect.width() * 0.5));
    let banner_top = l.snap(bottom - BANNER_H);
    let banner = [
        pos2(left, banner_top),
        pos2(banner_right - BANNER_LEAN, banner_top),
        pos2(banner_right, bottom),
        pos2(left + CUT_HERO, bottom),
        pos2(left, bottom - CUT_HERO),
    ];
    shapes::fill(painter, &banner, theme::PANEL_DEEP);
    text::paint(painter, pos2(left + PAD, banner_top + 37.0), title);
    text::paint(painter, pos2(left + PAD, banner_top + 62.0), line);

    let tab_left = l.snap(right - TAB_W);
    let tab_bottom = l.snap(top + TAB_H);
    let tab = [
        pos2(tab_left, top),
        pos2(right - CUT_HERO, top),
        pos2(right, top + CUT_HERO),
        pos2(right, tab_bottom),
        pos2(tab_left + TAB_H * 0.75, tab_bottom),
    ];
    shapes::fill(painter, &tab, theme::PANEL_DEEP);
    let tab_style = Style::new(Face::Title, size::CAPS, hero.tab.1).spaced(1.6);
    let base = top + TAB_H / 2.0 + size::CAPS * theme::CAP_HEIGHT / 2.0 + 1.0;
    text::draw_right(painter, pos2(right - 20.0, base), hero.tab.0, &tab_style);

    let border = if lit { theme::TEXT } else { theme::CYAN };
    shapes::outline(painter, &points, l.px(2.0), border);
}

pub struct Tile<'a> {
    pub icon: Icon,
    pub title: &'a str,
    pub text: &'a str,
    pub hovered: bool,
}

const BAND_H: f32 = 80.0;
const BAND_CUT: f32 = 20.0;
const TILE_PAD: f32 = 20.0;

pub fn tile(painter: &Painter, l: &Layout, rect: Rect, art: Option<&Art>, tile: &Tile) {
    let points = shapes::chamfered(rect, Cut::diagonal(CUT_TILE));
    if tile.hovered {
        shapes::glow(painter, &points, 14.0, theme::CYAN.gamma_multiply(0.40));
    }
    picture(painter, l, &points, rect, art, vec2(0.5, 0.0));
    let (left, right, bottom) = (rect.left(), rect.right(), rect.bottom());
    let band_top = l.snap(bottom - BAND_H);
    if art.is_some_and(|a| a.source == Source::BuiltIn) {
        let side = (band_top - rect.top() - 24.0).max(8.0);
        let mark = Rect::from_min_size(pos2(right - PAD - side, rect.top() + 12.0), Vec2::splat(side));
        icons::draw(painter, tile.icon, mark, Color32::from_white_alpha(56));
    }
    let band = [
        pos2(left, band_top),
        pos2(right - BAND_CUT, band_top),
        pos2(right, band_top + BAND_CUT),
        pos2(right, bottom),
        pos2(left + CUT_TILE, bottom),
        pos2(left, bottom - CUT_TILE),
    ];
    shapes::fill(painter, &band, theme::PANEL_DEEP);
    let title_base = band_top + 27.0;
    let cap = size::TILE_TITLE * theme::CAP_HEIGHT;
    let icon = Rect::from_center_size(pos2(left + TILE_PAD + 10.0, title_base - cap / 2.0), Vec2::splat(20.0));
    icons::draw(painter, tile.icon, icon, theme::CYAN);
    let title_colour = if tile.hovered { theme::TEXT } else { theme::CYAN_SOFT };
    let title = Style::new(Face::Title, size::TILE_TITLE, title_colour).spaced(0.4);
    text::draw(painter, pos2(icon.right() + 10.0, title_base), tile.title, &title);
    let body = Style::new(Face::Text, size::WIDGET, theme::TEXT_SOFT).leading(18.0);
    let fit = Fit::Wrap(rect.width() - 2.0 * TILE_PAD);
    text::draw_fit(painter, pos2(left + TILE_PAD, title_base + 20.0), tile.text, &body, fit);
    let (width, colour) = if tile.hovered { (l.px(2.0), theme::CYAN) } else { (l.px(1.0), theme::LINE) };
    shapes::outline(painter, &points, width, colour);
}
