use eframe::egui::epaint::PathStroke;
use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, pos2, vec2};

use super::text::{self, Style};
use super::theme::{self, Face};
use super::{icons, layout::Layout};

const CAP: f32 = 30.0;
const WEIGHT: f32 = 6.0;
const LETTER: f32 = 25.0;
const MOUNTAIN_W: f32 = 36.0;
const MOUNTAIN_GAP: f32 = 3.0;
const ROW_GAP: f32 = 12.0;

struct Glyph {
    bars: &'static [(f32, f32, f32, f32)],
    diagonals: &'static [(f32, f32)],
}

const WX: f32 = WEIGHT / LETTER;
const WY: f32 = WEIGHT / CAP;
const DX: f32 = WX * 1.12;

const V: Glyph = Glyph { bars: &[], diagonals: &[(0.0, 0.5 - DX / 2.0), (1.0 - DX, 0.5 - DX / 2.0)] };
const A: Glyph = Glyph { bars: &[], diagonals: &[(0.5 - DX / 2.0, 0.0), (0.5 - DX / 2.0, 1.0 - DX)] };
const L: Glyph = Glyph { bars: &[(0.0, 0.0, WX, 1.0), (0.0, 1.0 - WY, 0.86, 1.0)], diagonals: &[] };
const N: Glyph = Glyph { bars: &[(0.0, 0.0, WX, 1.0), (1.0 - WX, 0.0, 1.0, 1.0)], diagonals: &[(0.0, 1.0 - DX)] };
const C: Glyph =
    Glyph { bars: &[(0.0, 0.0, WX, 1.0), (0.0, 0.0, 0.94, WY), (0.0, 1.0 - WY, 0.94, 1.0)], diagonals: &[] };
const H: Glyph = Glyph {
    bars: &[(0.0, 0.0, WX, 1.0), (1.0 - WX, 0.0, 1.0, 1.0), (0.0, 0.5 - WY / 2.0, 1.0, 0.5 + WY / 2.0)],
    diagonals: &[],
};
const E: Glyph = Glyph {
    bars: &[
        (0.0, 0.0, WX, 1.0),
        (0.0, 0.0, 0.92, WY),
        (0.0, 0.5 - WY / 2.0, 0.80, 0.5 + WY / 2.0),
        (0.0, 1.0 - WY, 0.92, 1.0),
    ],
    diagonals: &[],
};
const LETTERS: [(&Glyph, f32); 8] =
    [(&V, 3.0), (&A, 5.0), (&L, 0.0), (&A, 5.0), (&N, 8.0), (&C, 6.0), (&H, 8.0), (&E, 0.0)];

pub const WIDTH: f32 = MOUNTAIN_W + MOUNTAIN_GAP + 8.0 * LETTER + 35.0;
pub const HEIGHT: f32 = 2.0 * (theme::size::CAPS * theme::CAP_HEIGHT + ROW_GAP) + CAP;

fn glyph(painter: &Painter, l: &Layout, g: &Glyph, cell: Rect, color: Color32) {
    let at = |x: f32, y: f32| pos2(l.snap(cell.left() + x * cell.width()), l.snap(cell.top() + y * cell.height()));
    for &(x0, y0, x1, y1) in g.bars {
        painter.rect_filled(Rect::from_min_max(at(x0, y0), at(x1, y1)), 0.0, color);
    }
    for &(top, bottom) in g.diagonals {
        let points: Vec<Pos2> = vec![at(top, 0.0), at(top + DX, 0.0), at(bottom + DX, 1.0), at(bottom, 1.0)];
        painter.add(Shape::convex_polygon(points, color, PathStroke::NONE));
    }
}

pub fn draw(painter: &Painter, l: &Layout, origin: Pos2) {
    let small = Style::new(Face::Text, theme::size::CAPS, theme::TEXT_SOFT).spaced(theme::size::CAPS * 0.62);
    let small_cap = theme::size::CAPS * theme::CAP_HEIGHT;
    let big_top = origin.y + small_cap + ROW_GAP;
    let mountain = Rect::from_min_size(pos2(origin.x, big_top - 3.0), vec2(MOUNTAIN_W, CAP + 3.0));
    icons::mountain(painter, mountain, theme::CYAN_SOFT);
    let first = origin.x + MOUNTAIN_W + MOUNTAIN_GAP;
    let mut x = first;
    for (g, space) in LETTERS {
        glyph(painter, l, g, Rect::from_min_size(pos2(x, big_top), vec2(LETTER, CAP)), theme::TEXT);
        x += LETTER + space;
    }
    text::draw(painter, pos2(first, origin.y + small_cap), "PROJECT", &small);
    let right = origin.x + WIDTH;
    text::draw_right(painter, pos2(right, big_top + CAP + ROW_GAP + small_cap), "LAUNCHER", &small);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_width_is_the_sum_of_its_parts() {
        let letters: f32 = LETTERS.iter().map(|(_, space)| LETTER + space).sum();
        assert_eq!(WIDTH, MOUNTAIN_W + MOUNTAIN_GAP + letters);
    }
}
