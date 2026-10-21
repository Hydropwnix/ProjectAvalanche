use super::*;

pub(super) fn new_text_state(lib: &Library, movie: MovieId, e: &Arc<EditText>) -> TextState {
    let (default_char, default_para) = text::default_formats(e);
    let html = e.flags2 & EditText::HTML != 0;
    let mut t = TextState {
        def: e.clone(),
        html,
        source: e.initial_text.clone().unwrap_or_default(),
        paragraphs: Vec::new(),
        default_char,
        default_para,
        bounds: RectF::from_swf(&e.bounds),
        layout: TextLayout::default(),
        auto_size: AutoSize::None,
        word_wrap: e.flags1 & EditText::WORD_WRAP != 0,
        text_auto_size: TextAutoSize::None,
    };
    relayout(lib, movie, &mut t);
    t
}

pub(super) fn relayout(lib: &Library, movie: MovieId, t: &mut TextState) {
    t.paragraphs = if t.html {
        text::parse_html(&t.source, &t.default_char, &t.default_para)
    } else {
        text::plain(&t.source, &t.default_char, &t.default_para)
    };
    t.layout = text::layout(lib, movie, t.def.font_id, &t.paragraphs, &t.default_char, t.bounds, t.word_wrap);
    apply_auto_size(lib, movie, t);
    apply_text_auto_size(lib, movie, t);
}

pub(super) fn apply_text_auto_size(lib: &Library, movie: MovieId, t: &mut TextState) {
    let gutter = text::GUTTER;
    let (w, h) = (t.bounds.width() - 2.0 * gutter, t.bounds.height() - 2.0 * gutter);
    let (tw, th) = (t.layout.text_width, t.layout.text_height);
    let mut k = match t.text_auto_size {
        TextAutoSize::None => return,
        TextAutoSize::Shrink => {
            let kx = if w < tw { w / tw } else { 1.0 };
            let ky = if h < th { h / th } else { 1.0 };
            kx.min(ky)
        }
        TextAutoSize::Fit => {
            let kx = if tw > 19.0 { w / tw } else { f32::MAX };
            let ky = if th > 19.0 { h / th } else { f32::MAX };
            let k = kx.min(ky);
            if k == f32::MAX { 1.0 } else { k }
        }
    };
    if k == 1.0 {
        return;
    }
    let smallest = t.layout.line_boxes.iter().map(|b| b.1.trunc()).fold(f32::MAX, f32::min);
    if smallest > 0.0 && smallest < f32::MAX && smallest * k < 120.0 {
        k = 120.0 / smallest;
    }
    if !k.is_finite() {
        return;
    }
    // TODO(guess): the scale is applied by laying the text out again at the scaled sizes (the game scales the formatted lines in place, 0x8271DA80); settle: whether the line buffer's flag 0x01 (+0x34) realigns the lines before they draw.
    let mut paras = t.paragraphs.clone();
    for p in &mut paras {
        for r in &mut p.runs {
            r.format.size *= k;
        }
    }
    let mut def = t.default_char.clone();
    def.size *= k;
    t.layout = text::layout(lib, movie, t.def.font_id, &paras, &def, t.bounds, t.word_wrap);
}

pub(super) fn apply_auto_size(lib: &Library, movie: MovieId, t: &mut TextState) {
    if t.auto_size == AutoSize::None {
        return;
    }
    let gutter = 40.0;
    let b = t.bounds;
    let h = t.layout.text_height + 2.0 * gutter;
    let mut nb = RectF::new(b.x0, b.y0, b.x1, b.y0 + h);
    if !t.word_wrap {
        let w = t.layout.text_width + 2.0 * gutter;
        let (x0, x1) = match t.auto_size {
            AutoSize::Left | AutoSize::None => (b.x0, b.x0 + w),
            AutoSize::Center => ((b.x0 + b.x1 - w) / 2.0, (b.x0 + b.x1 + w) / 2.0),
            AutoSize::Right => (b.x1 - w, b.x1),
        };
        nb.x0 = x0;
        nb.x1 = x1;
    }
    if nb != t.bounds {
        t.bounds = nb;
        t.layout = text::layout(lib, movie, t.def.font_id, &t.paragraphs, &t.default_char, t.bounds, t.word_wrap);
    }
}
