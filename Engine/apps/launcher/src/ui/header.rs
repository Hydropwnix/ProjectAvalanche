use eframe::egui::{Color32, Painter, Rect, pos2, vec2};

use super::layout::Layout;
use super::wordmark;
use crate::art::{Gallery, Slot};

const GAP: f32 = 24.0;

pub fn draw(painter: &Painter, l: &Layout, gallery: &Gallery) {
    let header = l.header;
    let mut left = header.left();
    if let Some(art) = gallery.get(Slot::Logo) {
        let size = vec2(header.height() * art.size.x / art.size.y.max(1.0), header.height());
        let rect = Rect::from_min_size(header.min, size);
        let rect = Rect::from_min_max(
            pos2(l.snap(rect.left()), l.snap(rect.top())),
            pos2(l.snap(rect.right()), l.snap(rect.bottom())),
        );
        painter.image(art.texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        left = rect.right() + GAP;
    }
    let top = header.center().y - wordmark::HEIGHT / 2.0;
    wordmark::draw(painter, l, pos2(l.snap(left), l.snap(top)));
}
