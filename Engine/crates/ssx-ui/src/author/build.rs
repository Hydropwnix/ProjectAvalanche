use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::place::PlaceObject;
use ssx_formats::gfx::shape::{DefineShape, FillStyle, Shape, ShapeRecord, Styles};
use ssx_formats::gfx::text::{EditText, Font, TextLayout};
use ssx_formats::gfx::types::{ColorTransform, Matrix, Rect, Rgba};

use super::LayoutError;
use super::json::Pos;
use super::layout::{Align, Child, ChildKind, Layout, TextDef};
use crate::geom::RectF;
use crate::library::{CharRef, Character, FrameDef, FrameTag, Import, Library, MovieDef, MovieId, TimelineDef};

pub fn movie_path(view: &str) -> String {
    format!("views/{view}.gfx")
}

pub fn movie_url(view: &str) -> String {
    format!("views/{view}.swf")
}

impl Layout {
    pub fn build(&self, lib: &mut Library) -> Result<MovieId, LayoutError> {
        let def = self.movie(lib)?;
        let id = lib.insert_movie(def);
        Ok(id)
    }

    pub fn movie(&self, lib: &mut Library) -> Result<MovieDef, LayoutError> {
        let err = |pos: Pos, m: String| LayoutError { file: self.file.clone(), pos, message: m };
        let path = movie_path(&self.view);
        if lib.movie_by_path(&path).is_some() || lib.files().exists(&format!("{}{path}", crate::fs::FE_ROOT)) {
            return Err(err(Pos { line: 1, col: 1 }, format!("the movie {path} exists already")));
        }
        let mut targets = Vec::new();
        for imp in &self.imports {
            let other = lib
                .load_movie(&imp.url)
                .map_err(|e| err(imp.pos, format!("import {:?}: {} cannot be loaded: {e:#}", imp.name, imp.url)))?;
            let Some(&target) = lib.movie(other).exports.get(&imp.symbol) else {
                return Err(err(imp.pos, format!("import {:?}: {} exports no {:?}", imp.name, imp.url, imp.symbol)));
            };
            targets.push(CharRef { movie: other, id: target });
        }
        let mut b =
            Builder { layout: self, id: lib.movies().len(), chars: HashMap::new(), next: 1, ids: HashMap::new() };
        let mut imports = HashMap::new();
        for (imp, target) in self.imports.iter().zip(targets) {
            let cid = b.alloc(&imp.name, imp.pos)?;
            imports.insert(cid, Import { url: imp.url.clone(), name: imp.symbol.clone(), target: Some(target) });
        }
        let mut fonts: HashMap<String, u16> = HashMap::new();
        for (child, text) in self.texts() {
            let Some(lib_font) = lib.font_library().get(&text.font).cloned() else {
                let mut names: Vec<&str> = lib.font_library().keys().map(String::as_str).collect();
                names.sort_unstable();
                return Err(err(
                    child.pos,
                    format!("font {:?} is not in the font library ({})", text.font, names.join(", ")),
                ));
            };
            if let Some(c) = text.text.chars().find(|&c| !has_glyph(&lib_font, c)) {
                return Err(err(child.pos, format!("text: the font {:?} has no glyph for {c:?}", text.font)));
            }
            if !fonts.contains_key(&text.font) {
                let id = b.fresh(child.pos)?;
                b.chars.insert(id, Character::Font(Arc::new(placeholder_font(id, &text.font))));
                fonts.insert(text.font.clone(), id);
            }
        }
        for s in &self.symbols {
            b.alloc(&s.name, s.pos)?;
        }
        for s in &self.symbols {
            let mut frame = FrameDef::default();
            for (depth, c) in s.children.iter().enumerate() {
                let ch = b.child_character(c, &fonts)?;
                frame.tags.push(FrameTag::Place(Arc::new(place(c, depth as u16 + 1, ch))));
            }
            let id = b.ids[s.name.as_str()];
            b.chars.insert(id, Character::Sprite(Arc::new(TimelineDef { movie: b.id, id, frames: vec![frame] })));
        }
        let mut exports = HashMap::new();
        for s in self.symbols.iter().filter(|s| s.export) {
            exports.insert(s.name.clone(), b.ids[s.name.as_str()]);
        }
        let view = Child {
            pos: Pos::default(),
            instance: "mView".into(),
            kind: ChildKind::Symbol(self.view_symbol.clone()),
            x: self.view_x,
            y: self.view_y,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            alpha: 100.0,
            visible: true,
        };
        let root = FrameDef {
            tags: vec![FrameTag::Place(Arc::new(place(&view, 1, b.ids[self.view_symbol.as_str()])))],
            ..Default::default()
        };
        let (w, h) = (self.stage.0 as f32 * 20.0, self.stage.1 as f32 * 20.0);
        let def = MovieDef {
            path: path.clone(),
            dir: format!("{}views/", crate::fs::FE_ROOT),
            version: 8,
            frame_rate: self.frame_rate,
            stage: RectF::new(0.0, 0.0, w, h),
            background: None,
            swf_name: self.view.clone(),
            root: Arc::new(TimelineDef { movie: b.id, id: 0, frames: vec![root] }),
            chars: b.chars,
            exports,
            imports,
            scaling_grids: HashMap::new(),
        };
        Ok(def)
    }

    fn texts(&self) -> impl Iterator<Item = (&Child, &TextDef)> {
        self.symbols.iter().flat_map(|s| &s.children).filter_map(|c| match &c.kind {
            ChildKind::Text(t) => Some((c, t)),
            _ => None,
        })
    }
}

struct Builder<'l> {
    layout: &'l Layout,
    id: MovieId,
    chars: HashMap<u16, Character>,
    next: u16,
    ids: HashMap<&'l str, u16>,
}

impl<'l> Builder<'l> {
    fn fresh(&mut self, pos: Pos) -> Result<u16, LayoutError> {
        let id = self.next;
        self.next = self.next.checked_add(1).filter(|&n| n < u16::MAX).ok_or_else(|| LayoutError {
            file: self.layout.file.clone(),
            pos,
            message: "more characters than a movie holds".into(),
        })?;
        Ok(id)
    }

    fn alloc(&mut self, name: &'l str, pos: Pos) -> Result<u16, LayoutError> {
        let id = self.fresh(pos)?;
        self.ids.insert(name, id);
        Ok(id)
    }

    fn child_character(&mut self, c: &Child, fonts: &HashMap<String, u16>) -> Result<u16, LayoutError> {
        match &c.kind {
            ChildKind::Symbol(name) => Ok(self.ids[name.as_str()]),
            ChildKind::Text(t) => {
                let id = self.fresh(c.pos)?;
                self.chars.insert(id, Character::EditText(Arc::new(edit_text(id, t, fonts[&t.font]))));
                Ok(id)
            }
            ChildKind::Rect(r) => {
                let id = self.fresh(c.pos)?;
                let [red, g, b, a] = r.color;
                let shape = rect_shape(id, (r.width * 20.0) as i32, (r.height * 20.0) as i32, Rgba { r: red, g, b, a });
                self.chars.insert(id, Character::Shape(Arc::new(shape)));
                Ok(id)
            }
        }
    }
}

pub fn has_glyph(font: &Font, c: char) -> bool {
    c == '\n' || u16::try_from(u32::from(c)).is_ok_and(|code| font.codes.contains(&code))
}

fn placeholder_font(id: u16, name: &str) -> Font {
    Font {
        version: 3,
        id,
        flags: Font::WIDE_CODES,
        language: 0,
        name: name.to_string(),
        glyphs: Vec::new(),
        codes: Vec::new(),
        layout: None,
        unk_trailing_layout: None,
    }
}

fn edit_text(id: u16, t: &TextDef, font: u16) -> EditText {
    let [r, g, b, a] = t.color;
    let align = match t.align {
        Align::Left => 0,
        Align::Right => 1,
        Align::Center => 2,
    };
    let initial = if t.html {
        let name = ["left", "right", "center"][usize::from(align)];
        Some(format!(
            "<p align=\"{name}\"><font face=\"{}\" size=\"{}\" color=\"#{r:02x}{g:02x}{b:02x}\">{}</font></p>",
            escape(&t.font),
            t.size,
            escape(&t.text)
        ))
    } else {
        (!t.text.is_empty()).then(|| t.text.clone())
    };
    let mut flags1 = 0x08 | 0x04 | 0x01;
    if initial.is_some() {
        flags1 |= 0x80;
    }
    if t.wrap {
        flags1 |= EditText::WORD_WRAP;
    }
    if t.multiline {
        flags1 |= EditText::MULTILINE;
    }
    let flags2 = 0x20 | 0x10 | EditText::USE_OUTLINES | if t.html { EditText::HTML } else { 0 };
    EditText {
        id,
        bounds: Rect { x_min: 0, x_max: (t.width * 20.0) as i32, y_min: 0, y_max: (t.height * 20.0) as i32 },
        flags1,
        flags2,
        font_id: Some(font),
        font_class: None,
        font_height: Some((t.size * 20.0).round() as u16),
        color: Some(Rgba { r, g, b, a }),
        max_length: None,
        layout: Some(TextLayout { align, left_margin: 0, right_margin: 0, indent: 0, leading: 0 }),
        variable_name: String::new(),
        initial_text: initial,
    }
}

pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

fn rect_shape(id: u16, w: i32, h: i32, color: Rgba) -> DefineShape {
    let records = vec![
        ShapeRecord::StyleChange { move_to: Some((0, 0)), fill0: None, fill1: Some(1), line: None, new_styles: None },
        ShapeRecord::Straight { dx: w, dy: 0 },
        ShapeRecord::Straight { dx: 0, dy: h },
        ShapeRecord::Straight { dx: -w, dy: 0 },
        ShapeRecord::Straight { dx: 0, dy: -h },
    ];
    DefineShape {
        version: 3,
        id,
        bounds: Rect { x_min: 0, x_max: w, y_min: 0, y_max: h },
        edge_bounds: None,
        unk_reserved: 0,
        uses_fill_winding_rule: false,
        uses_non_scaling_strokes: false,
        uses_scaling_strokes: false,
        shape: Shape { styles: Styles { fills: vec![FillStyle::Solid(color)], lines: Vec::new() }, records },
    }
}

fn place(c: &Child, depth: u16, character: u16) -> PlaceObject {
    let (sin, cos) = c.rotation.to_radians().sin_cos();
    let matrix = Matrix {
        a: c.scale_x * cos,
        b: c.scale_x * sin,
        c: -c.scale_y * sin,
        d: c.scale_y * cos,
        tx: (c.x * 20.0).round() as i32,
        ty: (c.y * 20.0).round() as i32,
    };
    let color_transform = (c.alpha < 100.0).then(|| {
        let mut ct = ColorTransform::default();
        ct.mult[3] = (c.alpha / 100.0 * 256.0).round() as i16;
        ct
    });
    let mut flags = PlaceObject::HAS_CHARACTER | 0x04 | 0x20;
    if color_transform.is_some() {
        flags |= 0x08;
    }
    PlaceObject {
        version: if c.visible { 2 } else { 3 },
        flags,
        flags3: if c.visible { 0 } else { 0x20 },
        depth,
        class_name: None,
        character_id: Some(character),
        matrix: Some(matrix),
        color_transform,
        ratio: None,
        name: Some(c.instance.clone()),
        clip_depth: None,
        filters: Vec::new(),
        blend_mode: None,
        cache_as_bitmap: None,
        visible: (!c.visible).then_some(0),
        background: None,
        clip_actions: None,
    }
}
