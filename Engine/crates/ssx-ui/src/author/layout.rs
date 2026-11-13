use std::collections::HashSet;

use super::json::{Node, Pos};
use super::read::{Cx, is_name};
use super::{LayoutError, limits};

pub const LAYERS: &[(&str, &str)] = &[
    ("dimmer", "LAYER_DIMMER"),
    ("background", "LAYER_BACKGROUND"),
    ("tutorialhud", "LAYER_TUTORIALHUD"),
    ("hud", "LAYER_HUD"),
    ("screen", "LAYER_SCREEN"),
    ("activityfeed", "LAYER_ACTIVITY_FEED"),
    ("dock", "LAYER_DOCK"),
    ("loginbanner", "LAYER_LOGINBANNER"),
    ("dynamicleaderboard", "LAYER_DYNAMIC_LEADERBOARD"),
    ("eventtimer", "LAYER_EVENT_TIMER"),
    ("hud_finishmessage", "LAYER_HUD_FINISHMESSAGE"),
    ("hud_boostnotification", "LAYER_HUD_BOOSTNOTIFICATION"),
    ("hud_nisprompt", "LAYER_HUD_NISPROMPT"),
    ("hint", "LAYER_HINT"),
    ("tutorial", "LAYER_TUTORIAL"),
    ("popup", "LAYER_POPUP"),
    ("busyicon", "LAYER_BUSY_ICON"),
    ("hud_nistitle", "LAYER_HUD_NISTITLE"),
    ("hud_hudmeters", "LAYER_HUD_HUDMETERS"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub file: String,
    pub view: String,
    pub layer: &'static str,
    pub stage: (u32, u32),
    pub frame_rate: f32,
    pub imports: Vec<ImportDef>,
    pub symbols: Vec<SymbolDef>,
    pub view_symbol: String,
    pub view_x: f32,
    pub view_y: f32,
    pub logic: super::logic::Logic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportDef {
    pub pos: Pos,
    pub name: String,
    pub url: String,
    pub symbol: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SymbolDef {
    pub pos: Pos,
    pub name: String,
    pub export: bool,
    pub cell: Option<CellDef>,
    pub children: Vec<Child>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CellDef {
    Forward(String),
    ForwardTitle(String),
    Own {
        texts: Vec<(String, String)>,
        images: Vec<(String, String)>,
        shrink: Vec<String>,
        highlight: String,
        width: f32,
        height: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Child {
    pub pos: Pos,
    pub instance: String,
    pub kind: ChildKind,
    pub x: f32,
    pub y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation: f32,
    pub alpha: f32,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChildKind {
    Symbol(String),
    Text(TextDef),
    Rect(RectDef),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextDef {
    pub font: String,
    pub size: f32,
    pub color: [u8; 4],
    pub align: Align,
    pub width: f32,
    pub height: f32,
    pub multiline: bool,
    pub wrap: bool,
    pub html: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RectDef {
    pub width: f32,
    pub height: f32,
    pub color: [u8; 4],
}

impl Layout {
    pub fn parse(file: &str, text: &str) -> Result<Layout, LayoutError> {
        let cx = Cx { file };
        let root = super::json::parse(text, &limits::JSON).map_err(|e| LayoutError {
            file: file.to_string(),
            pos: e.pos,
            message: e.message,
        })?;
        let mut o = cx.obj(&root, "layout")?;
        let format = o.req(&cx, "format")?;
        if cx.int(format, "format", 1, 1)? != 1 {
            return Err(cx.err(format.pos, "format: only version 1 exists"));
        }
        let view = cx.name(o.req(&cx, "view")?, "view", "ava_", limits::NAME)?.to_string();
        if view.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(cx.err(root.pos, format!("view: {view:?} must be lower case (it names the movie's file)")));
        }
        let layer_node = o.req(&cx, "layer")?;
        let names: Vec<&str> = LAYERS.iter().map(|l| l.0).collect();
        let layer_name = cx.choice(layer_node, "layer", &names)?;
        let layer = LAYERS.iter().find(|l| l.0 == layer_name).map_or("", |l| l.1);
        let stage_node = o.req(&cx, "stage")?;
        let stage = cx.array(stage_node, "stage", 2)?;
        if stage.len() != 2 {
            return Err(cx.err(stage_node.pos, "stage: expected [width, height]"));
        }
        let stage = (
            cx.int(&stage[0], "stage width", 1, limits::STAGE)? as u32,
            cx.int(&stage[1], "stage height", 1, limits::STAGE)? as u32,
        );
        let frame_rate = cx.num(o.req(&cx, "frame_rate")?, "frame_rate", 1.0, 120.0)? as f32;
        let mut imports = Vec::new();
        for n in cx.array(o.req(&cx, "imports")?, "imports", limits::IMPORTS)? {
            imports.push(read_import(&cx, n)?);
        }
        let mut symbols = Vec::new();
        for n in cx.array(o.req(&cx, "symbols")?, "symbols", limits::SYMBOLS)? {
            symbols.push(read_symbol(&cx, n)?);
        }
        let mut vo = cx.obj(o.req(&cx, "view_symbol")?, "view_symbol")?;
        let vs_node = vo.req(&cx, "symbol")?;
        let view_symbol = cx.name(vs_node, "view_symbol.symbol", "", limits::NAME)?.to_string();
        let view_x = opt_num(&cx, &mut vo, "x", -limits::COORD, limits::COORD, 0.0)?;
        let view_y = opt_num(&cx, &mut vo, "y", -limits::COORD, limits::COORD, 0.0)?;
        vo.finish(&cx)?;
        let logic_node = o.req(&cx, "logic")?;
        o.finish(&cx)?;
        let mut layout = Layout {
            file: file.to_string(),
            view,
            layer,
            stage,
            frame_rate,
            imports,
            symbols,
            view_symbol,
            view_x,
            view_y,
            logic: Default::default(),
        };
        layout.check_names(&cx, vs_node.pos)?;
        layout.logic = super::logic::Logic::read(&cx, logic_node, &layout)?;
        Ok(layout)
    }

    pub fn import(&self, name: &str) -> Option<&ImportDef> {
        self.imports.iter().find(|i| i.name == name)
    }

    pub fn symbol(&self, name: &str) -> Option<&SymbolDef> {
        self.symbols.iter().find(|s| s.name == name)
    }

    fn check_names(&self, cx: &Cx, view_pos: Pos) -> Result<(), LayoutError> {
        let mut seen = HashSet::new();
        for (name, pos) in
            self.imports.iter().map(|i| (&i.name, i.pos)).chain(self.symbols.iter().map(|s| (&s.name, s.pos)))
        {
            if !seen.insert(name.as_str()) {
                return Err(cx.err(pos, format!("the name {name:?} is given twice (imports and symbols share names)")));
            }
        }
        for s in &self.symbols {
            if s.export && !s.name.starts_with(limits::EXPORT_PREFIX) {
                return Err(cx.err(
                    s.pos,
                    format!("symbol {:?}: an exported symbol's name starts with `{}`", s.name, limits::EXPORT_PREFIX),
                ));
            }
            let mut instances = HashSet::new();
            for c in &s.children {
                if !instances.insert(c.instance.as_str()) {
                    return Err(cx.err(c.pos, format!("symbol {:?}: instance {:?} is given twice", s.name, c.instance)));
                }
                if let ChildKind::Symbol(r) = &c.kind
                    && self.import(r).is_none()
                    && self.symbol(r).is_none()
                {
                    return Err(cx.err(c.pos, format!("instance {:?}: no import or symbol is named {r:?}", c.instance)));
                }
            }
        }
        for s in &self.symbols {
            let import_of = |inner: &str| {
                s.children.iter().find(|c| c.instance == inner).and_then(|c| match &c.kind {
                    ChildKind::Symbol(r) => self.import(r),
                    _ => None,
                })
            };
            match &s.cell {
                None => {}
                Some(CellDef::Forward(inner)) if import_of(inner).is_none() => {
                    return Err(cx.err(
                        s.pos,
                        format!("symbol {:?}: its cell {inner:?} must place an imported component", s.name),
                    ));
                }
                Some(CellDef::ForwardTitle(inner))
                    if import_of(inner).is_none_or(|i| i.symbol != "components.MainMenuCell") =>
                {
                    let (name, need) = (&s.name, "`components.MainMenuCell`");
                    return Err(cx.err(s.pos, format!("symbol {name:?}: its title cell {inner:?} must place {need}")));
                }
                Some(CellDef::Own { texts, images, highlight, .. }) => {
                    for (text, _) in texts {
                        if !s.children.iter().any(|c| c.instance == *text && matches!(c.kind, ChildKind::Text(_))) {
                            return Err(cx.err(
                                s.pos,
                                format!("symbol {:?}: its cell's text {text:?} is not a text field of it", s.name),
                            ));
                        }
                    }
                    for (loader, _) in images {
                        if import_of(loader).is_none_or(|i| i.symbol != "components.ImageLoader") {
                            return Err(cx.err(
                                s.pos,
                                format!(
                                    "symbol {:?}: its cell's image {loader:?} must place `components.ImageLoader`",
                                    s.name
                                ),
                            ));
                        }
                    }
                    if import_of(highlight).is_none_or(|i| i.symbol != "components.Highlight") {
                        return Err(cx.err(
                            s.pos,
                            format!(
                                "symbol {:?}: its cell's highlight {highlight:?} must place `components.Highlight`",
                                s.name
                            ),
                        ));
                    }
                }
                Some(CellDef::Forward(_) | CellDef::ForwardTitle(_)) => {}
            }
        }
        match self.symbol(&self.view_symbol) {
            Some(s) if s.export => {}
            Some(s) => return Err(cx.err(s.pos, format!("the view symbol {:?} must be exported", s.name))),
            None => return Err(cx.err(view_pos, format!("view_symbol: no symbol is named {:?}", self.view_symbol))),
        }
        let counts = self.instance_counts(cx)?;
        let view = self.symbols.iter().position(|s| s.name == self.view_symbol).map_or(0, |i| counts[i]);
        if view > limits::INSTANCES {
            return Err(cx.err(view_pos, format!("the view places {view} instances, at most {}", limits::INSTANCES)));
        }
        Ok(())
    }

    fn instance_counts(&self, cx: &Cx) -> Result<Vec<u64>, LayoutError> {
        let n = self.symbols.len();
        let index = |name: &str| self.symbols.iter().position(|s| s.name == name);
        let refs: Vec<Vec<usize>> = self
            .symbols
            .iter()
            .map(|s| {
                s.children
                    .iter()
                    .filter_map(|c| match &c.kind {
                        ChildKind::Symbol(r) => index(r),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        let mut state = vec![0u8; n];
        let mut count = vec![0u64; n];
        for start in 0..n {
            if state[start] != 0 {
                continue;
            }
            let mut stack = vec![(start, 0usize)];
            state[start] = 1;
            while let Some(top) = stack.last_mut() {
                let s = top.0;
                if let Some(&r) = refs[s].get(top.1) {
                    top.1 += 1;
                    match state[r] {
                        1 => {
                            let sym = &self.symbols[r];
                            return Err(cx.err(sym.pos, format!("symbol {:?} contains itself", sym.name)));
                        }
                        0 => {
                            state[r] = 1;
                            stack.push((r, 0));
                        }
                        _ => {}
                    }
                    continue;
                }
                stack.pop();
                state[s] = 2;
                let below: u64 = self.symbols[s]
                    .children
                    .iter()
                    .map(|c| match &c.kind {
                        ChildKind::Symbol(r) => index(r).map_or(1, |i| count[i]),
                        _ => 1,
                    })
                    .fold(0u64, u64::saturating_add);
                count[s] = below.saturating_add(1);
            }
        }
        Ok(count)
    }

    pub fn resolve_path(&self, path: &str) -> Option<&Child> {
        let mut sym = self.symbol(&self.view_symbol)?;
        let mut parts = path.split('.').peekable();
        while let Some(p) = parts.next() {
            let c = sym.children.iter().find(|c| c.instance == p)?;
            if parts.peek().is_none() {
                return Some(c);
            }
            match &c.kind {
                ChildKind::Symbol(r) => sym = self.symbol(r)?,
                _ => return None,
            }
        }
        None
    }
}

fn opt_num(cx: &Cx, o: &mut super::read::Obj, key: &str, min: f64, max: f64, default: f32) -> Result<f32, LayoutError> {
    match o.opt(key) {
        Some(n) => Ok(cx.num(n, key, min, max)? as f32),
        None => Ok(default),
    }
}

fn read_import(cx: &Cx, n: &Node) -> Result<ImportDef, LayoutError> {
    let mut o = cx.obj(n, "import")?;
    let name = cx.name(o.req(cx, "name")?, "import name", "", limits::NAME)?.to_string();
    let url_node = o.req(cx, "url")?;
    let url = cx.str(url_node, "import url", limits::URL)?.to_string();
    if !valid_component_url(&url) {
        return Err(cx.err(
            url_node.pos,
            format!("import url: {url:?} is not `components/<name>.swf` (letters, digits, `_`, `/`)"),
        ));
    }
    let sym_node = o.req(cx, "symbol")?;
    let symbol = cx.str(sym_node, "import symbol", limits::NAME)?.to_string();
    if symbol.is_empty() || !symbol.split('.').all(is_name) {
        return Err(
            cx.err(sym_node.pos, format!("import symbol: {symbol:?} is not an export name (`components.Panel`)"))
        );
    }
    o.finish(cx)?;
    Ok(ImportDef { pos: n.pos, name, url, symbol })
}

pub fn valid_component_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("components/").and_then(|r| r.strip_suffix(".swf")) else { return false };
    !rest.is_empty() && rest.split('/').all(|seg| !seg.is_empty() && is_name(seg))
}

fn read_symbol(cx: &Cx, n: &Node) -> Result<SymbolDef, LayoutError> {
    let mut o = cx.obj(n, "symbol")?;
    let name = cx.name(o.req(cx, "name")?, "symbol name", "", limits::NAME)?.to_string();
    let export = match o.opt("export") {
        Some(e) => cx.bool(e, "export")?,
        None => false,
    };
    let cell_node = o.opt("cell");
    let mut children = Vec::new();
    for c in cx.array(o.req(cx, "children")?, "children", limits::CHILDREN)? {
        children.push(read_child(cx, c)?);
    }
    let cell = match cell_node {
        Some(c) => {
            if !export {
                return Err(cx.err(c.pos, format!("symbol {name:?}: a cell symbol must be exported")));
            }
            Some(match &c.value {
                super::json::Value::String(_) => {
                    let inner = cx.name(c, "cell", "", limits::NAME)?.to_string();
                    if !children.iter().any(|ch| ch.instance == inner && matches!(ch.kind, ChildKind::Symbol(_))) {
                        return Err(cx.err(c.pos, format!("cell: {name:?} places no symbol named {inner:?}")));
                    }
                    CellDef::Forward(inner)
                }
                super::json::Value::Object(members) if members.iter().any(|(k, _)| k.name == "forward") => {
                    let mut co = cx.obj(c, "cell")?;
                    let f = co.req(cx, "forward")?;
                    let inner = cx.name(f, "cell forward", "", limits::NAME)?.to_string();
                    if !children.iter().any(|ch| ch.instance == inner && matches!(ch.kind, ChildKind::Symbol(_))) {
                        return Err(cx.err(f.pos, format!("cell: {name:?} places no symbol named {inner:?}")));
                    }
                    cx.choice(co.req(cx, "frames")?, "frames", &["title"])?;
                    co.finish(cx)?;
                    CellDef::ForwardTitle(inner)
                }
                _ => {
                    let mut co = cx.obj(c, "cell")?;
                    let mut texts = Vec::new();
                    let mut shrink = Vec::new();
                    if let Some(t) = co.opt("text") {
                        texts.push((cx.name(t, "cell text", "", limits::NAME)?.to_string(), "strItem".to_string()));
                    }
                    let mut pairs = |co: &mut super::read::Obj, key: &str, inst: &str| -> Result<_, LayoutError> {
                        let mut out = Vec::new();
                        for n in co.opt(key).map_or(Ok(&[][..]), |a| cx.array(a, key, limits::CHILDREN))? {
                            let mut po = cx.obj(n, key)?;
                            let i = cx.name(po.req(cx, inst)?, inst, "", limits::NAME)?.to_string();
                            let f = cx.name(po.req(cx, "field")?, "field", "", limits::NAME)?.to_string();
                            if key == "texts"
                                && let Some(fit) = po.opt("fit")
                            {
                                cx.choice(fit, "fit", &["shrink"])?;
                                shrink.push(i.clone());
                            }
                            po.finish(cx)?;
                            out.push((i, f));
                        }
                        Ok(out)
                    };
                    let more = pairs(&mut co, "texts", "text")?;
                    let images = pairs(&mut co, "images", "loader")?;
                    texts.extend(more);
                    if texts.is_empty() {
                        return Err(cx.err(c.pos, format!("cell of {name:?}: give `text` or `texts`")));
                    }
                    let highlight = cx.name(co.req(cx, "highlight")?, "cell highlight", "", limits::NAME)?.to_string();
                    let width = cx.num(co.req(cx, "width")?, "width", 1.0, limits::COORD)? as f32;
                    let height = cx.num(co.req(cx, "height")?, "height", 1.0, limits::COORD)? as f32;
                    co.finish(cx)?;
                    CellDef::Own { texts, images, shrink, highlight, width, height }
                }
            })
        }
        None => None,
    };
    o.finish(cx)?;
    Ok(SymbolDef { pos: n.pos, name, export, cell, children })
}

fn read_child(cx: &Cx, n: &Node) -> Result<Child, LayoutError> {
    let mut o = cx.obj(n, "child")?;
    let instance = cx.name(o.req(cx, "instance")?, "instance", "", limits::NAME)?.to_string();
    let symbol = o.opt("symbol");
    let text = o.opt("text");
    let rect = o.opt("rect");
    let kind = match (symbol, text, rect) {
        (Some(s), None, None) => ChildKind::Symbol(cx.name(s, "symbol", "", limits::NAME)?.to_string()),
        (None, Some(t), None) => ChildKind::Text(read_text(cx, t)?),
        (None, None, Some(r)) => ChildKind::Rect(read_rect(cx, r)?),
        _ => return Err(cx.err(n.pos, format!("instance {instance:?}: give exactly one of `symbol`, `text`, `rect`"))),
    };
    let c = Child {
        pos: n.pos,
        instance,
        kind,
        x: opt_num(cx, &mut o, "x", -limits::COORD, limits::COORD, 0.0)?,
        y: opt_num(cx, &mut o, "y", -limits::COORD, limits::COORD, 0.0)?,
        scale_x: opt_num(cx, &mut o, "scale_x", -limits::SCALE, limits::SCALE, 1.0)?,
        scale_y: opt_num(cx, &mut o, "scale_y", -limits::SCALE, limits::SCALE, 1.0)?,
        rotation: opt_num(cx, &mut o, "rotation", -360.0, 360.0, 0.0)?,
        alpha: opt_num(cx, &mut o, "alpha", 0.0, 100.0, 100.0)?,
        visible: match o.opt("visible") {
            Some(v) => cx.bool(v, "visible")?,
            None => true,
        },
    };
    o.finish(cx)?;
    Ok(c)
}

fn read_text(cx: &Cx, n: &Node) -> Result<TextDef, LayoutError> {
    let mut o = cx.obj(n, "text")?;
    let font = cx.str(o.req(cx, "font")?, "font", limits::NAME)?.to_string();
    let size = cx.num(o.req(cx, "size")?, "size", 1.0, 200.0)? as f32;
    let color = cx.color(o.req(cx, "color")?, "color")?;
    let align = match o.opt("align") {
        Some(a) => match cx.choice(a, "align", &["left", "right", "center"])? {
            "right" => Align::Right,
            "center" => Align::Center,
            _ => Align::Left,
        },
        None => Align::Left,
    };
    let width = cx.num(o.req(cx, "width")?, "width", 1.0, limits::COORD)? as f32;
    let height = cx.num(o.req(cx, "height")?, "height", 1.0, limits::COORD)? as f32;
    let flag = |o: &mut super::read::Obj, k: &str| -> Result<bool, LayoutError> {
        o.opt(k).map_or(Ok(false), |v| cx.bool(v, k))
    };
    let multiline = flag(&mut o, "multiline")?;
    let wrap = flag(&mut o, "wrap")?;
    let html = flag(&mut o, "html")?;
    let text = match o.opt("text") {
        Some(t) => cx.str(t, "text", limits::TEXT)?.to_string(),
        None => String::new(),
    };
    o.finish(cx)?;
    Ok(TextDef { font, size, color, align, width, height, multiline, wrap, html, text })
}

fn read_rect(cx: &Cx, n: &Node) -> Result<RectDef, LayoutError> {
    let mut o = cx.obj(n, "rect")?;
    let width = cx.num(o.req(cx, "width")?, "width", 1.0, limits::COORD)? as f32;
    let height = cx.num(o.req(cx, "height")?, "height", 1.0, limits::COORD)? as f32;
    let color = cx.color(o.req(cx, "color")?, "color")?;
    o.finish(cx)?;
    Ok(RectDef { width, height, color })
}
