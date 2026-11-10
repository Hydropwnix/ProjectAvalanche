use std::collections::HashSet;

use super::json::{Node, Pos};
use super::layout::{ChildKind, Layout};
use super::read::Cx;
use super::{LayoutError, limits};

pub const PRIORITIES: &[(&str, &str)] = &[
    ("default", "INPUT_PRIORITY_DEFAULT"),
    ("medium", "INPUT_PRIORITY_MEDIUM"),
    ("high", "INPUT_PRIORITY_HIGH"),
    ("tutorial", "INPUT_PRIORITY_TUTORIAL"),
    ("popup", "INPUT_PRIORITY_POPUP"),
];

pub const KEYS: &[&str] = &[
    "ADVANCE",
    "BACK",
    "START",
    "SELECT",
    "DIGITAL_ONE",
    "DIGITAL_TWO",
    "DIGITAL_THREE",
    "DIGITAL_FOUR",
    "LEFT_TRIGGER",
    "RIGHT_TRIGGER",
    "LEFT_BUMPER",
    "RIGHT_BUMPER",
    "DPAD_UP",
    "DPAD_DOWN",
    "DPAD_LEFT",
    "DPAD_RIGHT",
    "LAS_UP",
    "LAS_DOWN",
    "LAS_LEFT",
    "LAS_RIGHT",
    "RAS",
];

pub const ICONS: &[&str] = &[
    "CALLOUT_FACE_BUTTON_PRIMARY",
    "CALLOUT_FACE_BUTTON_SECONDARY",
    "CALLOUT_FACE_BUTTON_TERTIARY",
    "CALLOUT_FACE_BUTTON_QUATERNARY",
    "CALLOUT_DPAD",
    "CALLOUT_DPAD_LR",
    "CALLOUT_DPAD_UD",
    "CALLOUT_LAS",
    "CALLOUT_START",
    "CALLOUT_SELECT",
    "CALLOUT_L1",
    "CALLOUT_R1",
    "CALLOUT_L2",
    "CALLOUT_R2",
    "CALLOUT_R3",
    "CALLOUT_ADVANCE",
    "CALLOUT_BACK",
];

pub const SOUNDS: &[&str] = &[
    "eg_ready_select",
    "menu_highlight",
    "menu_unhighlightable",
    "menu_select",
    "menu_unselectable",
    "menu_back",
    "button_press_advance",
    "button_press_back",
    "button_press_select",
    "button_press_reset",
];

pub const UI_EVENTS: &[&str] = &[
    "ACTIVITY_FEED_OPENED",
    "ACTIVITY_FEED_CLOSED",
    "ACTIVITY_FEED_CLOSE_REQUEST",
    "ACTIVITY_FEED_CLOSE_RESPONSE",
    "ACTIVITY_FEED_WYWA_INTRO",
    "ACTIVITY_FEED_NORMAL_INTRO",
    "FE_DOCK_SHOW",
    "FE_DOCK_HIDE",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Base {
    #[default]
    View,
    FeView,
    ActivityFeedView,
}

impl Base {
    pub fn class(self) -> &'static str {
        match self {
            Base::View => "ux.view.View",
            Base::FeView => "ssx.FEView",
            Base::ActivityFeedView => "ssx.FEActivityFeedView",
        }
    }

    pub fn is_fe_view(self) -> bool {
        matches!(self, Base::FeView | Base::ActivityFeedView)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Logic {
    pub base: Base,
    pub input_priority: Option<&'static str>,
    pub data: Vec<String>,
    pub texts: Vec<TextBinding>,
    pub menus: Vec<MenuDef>,
    pub panels: Vec<PanelDef>,
    pub callouts: Option<CalloutsDef>,
    pub billboards: Vec<BillboardDef>,
    pub on_enter: OnEnter,
    pub inputs: Vec<InputDef>,
    pub paging: Option<PagingDef>,
    pub drop_panels: Vec<DropPanelDef>,
    pub visibles: Vec<VisibleBinding>,
    pub images: Vec<ImageBinding>,
    pub consume_unmapped: bool,
    pub scrollbars: Vec<ScrollBarDef>,
    pub sd: Option<SdDef>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SdDef {
    pub pos: Pos,
    pub moves: Vec<(String, bool)>,
    pub scale: Option<f32>,
    pub centre: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScrollBarDef {
    pub pos: Pos,
    pub target: String,
    pub menu: String,
    pub length: f32,
    pub window: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BillboardDef {
    pub pos: Pos,
    pub target: String,
    pub data: String,
    pub symbol: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PagingDef {
    pub pos: Pos,
    pub menu: String,
    pub setup: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DropPanelDef {
    pub pos: Pos,
    pub target: String,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VisibleBinding {
    pub pos: Pos,
    pub target: String,
    pub data: String,
    pub field: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageBinding {
    pub pos: Pos,
    pub target: String,
    pub data: String,
    pub field: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OnEnter {
    pub clear_inputs: bool,
    pub event: Option<&'static str>,
    pub sound: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextBinding {
    pub pos: Pos,
    pub target: String,
    pub data: String,
    pub field: String,
    pub html: bool,
    pub fit: TextFit,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum TextFit {
    #[default]
    None,
    Shrink,
    List {
        items: String,
        more: String,
        separator: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuKind {
    Grid {
        columns: u32,
        rows: u32,
        cell_width: f32,
        cell_height: f32,
        spacing_x: f32,
        spacing_y: f32,
        wrap: bool,
    },
    List {
        rows: u32,
        cell_width: f32,
        cell_height: f32,
        spacing: f32,
        panel_gaps: Option<(f32, f32)>,
        wrap: bool,
        use_panel: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuDef {
    pub pos: Pos,
    pub target: String,
    pub kind: MenuKind,
    pub data: String,
    pub index: String,
    pub cell: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelDef {
    pub pos: Pos,
    pub target: String,
    pub width: f32,
    pub height: PanelHeight,
    pub dimmer: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelHeight {
    Fixed(f32),
    FitMenu(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CalloutsDef {
    pub pos: Pos,
    pub target: String,
    pub buttons: Vec<(&'static str, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuOp {
    Up,
    Down,
    Left,
    Right,
    Advance,
    Back,
    PageUp,
    PageDown,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputDo {
    Menu { target: String, op: MenuOp, blocked_sound: Option<&'static str>, sound: Option<&'static str> },
    Action { name: String, sound: Option<&'static str>, param: Option<f64> },
    Take,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InputDef {
    pub pos: Pos,
    pub key: &'static str,
    pub what: InputDo,
    pub lock_ms: Option<f64>,
}

impl Logic {
    pub(super) fn read(cx: &Cx, n: &Node, layout: &Layout) -> Result<Logic, LayoutError> {
        let mut o = cx.obj(n, "logic")?;
        let base = match o.opt("base") {
            Some(v) => match cx.choice(v, "base", &["view", "fe_view", "activity_feed_view"])? {
                "fe_view" => Base::FeView,
                "activity_feed_view" => Base::ActivityFeedView,
                _ => Base::View,
            },
            None => Base::View,
        };
        let mut logic = Logic { base, ..Default::default() };
        let names: Vec<&str> = PRIORITIES.iter().map(|p| p.0).collect();
        if let Some(v) = o.opt("input_priority") {
            let p = cx.choice(v, "input_priority", &names)?;
            logic.input_priority = PRIORITIES.iter().find(|x| x.0 == p).map(|x| x.1);
        }
        logic.consume_unmapped = match o.opt("consume_unmapped") {
            Some(v) => cx.bool(v, "consume_unmapped")?,
            None => false,
        };
        for d in cx.array(o.req(cx, "data")?, "data", limits::DATA)? {
            let name = data_name(cx, d)?;
            if logic.data.contains(&name) {
                return Err(cx.err(d.pos, format!("data: {name:?} is given twice")));
            }
            logic.data.push(name);
        }
        let known = |cx: &Cx, n: &Node, what: &str, data: &[String]| -> Result<String, LayoutError> {
            let name = data_name(cx, n)?;
            if !data.contains(&name) {
                return Err(cx.err(n.pos, format!("{what}: {name:?} is not in the logic's `data`")));
            }
            Ok(name)
        };
        for t in o.opt("texts").map_or(Ok(&[][..]), |a| cx.array(a, "texts", limits::BINDINGS))? {
            let mut to = cx.obj(t, "text binding")?;
            let target = target(cx, to.req(cx, "target")?, layout, Want::Text)?;
            let data = known(cx, to.req(cx, "data")?, "text binding data", &logic.data)?;
            let field = cx.name(to.req(cx, "field")?, "field", "", limits::NAME)?.to_string();
            let html = to.opt("html").map_or(Ok(false), |h| cx.bool(h, "html"))?;
            let fit = match to.opt("fit") {
                None => TextFit::None,
                Some(f) if matches!(f.value, super::json::Value::String(_)) => {
                    cx.choice(f, "fit", &["shrink"])?;
                    TextFit::Shrink
                }
                Some(f) => {
                    let mut fo = cx.obj(f, "fit")?;
                    let items = cx.name(fo.req(cx, "items")?, "fit items", "", limits::NAME)?.to_string();
                    let more = cx.name(fo.req(cx, "more")?, "fit more", "", limits::NAME)?.to_string();
                    let sep = fo.req(cx, "separator")?;
                    let separator = cx.str(sep, "fit separator", limits::SEPARATOR)?.to_string();
                    if separator.is_empty() || separator.chars().any(char::is_control) {
                        return Err(cx.err(sep.pos, "fit separator: empty, or a control character"));
                    }
                    fo.finish(cx)?;
                    if html {
                        return Err(cx.err(f.pos, "fit: a list is plain text (not with `html`)"));
                    }
                    TextFit::List { items, more, separator }
                }
            };
            to.finish(cx)?;
            logic.texts.push(TextBinding { pos: t.pos, target, data, field, html, fit });
        }
        for m in o.opt("menus").map_or(Ok(&[][..]), |a| cx.array(a, "menus", limits::MENUS))? {
            let def = read_menu(cx, m, layout, &logic.data, &known)?;
            logic.menus.push(def);
        }
        for p in o.opt("panels").map_or(Ok(&[][..]), |a| cx.array(a, "panels", limits::MENUS))? {
            let mut po = cx.obj(p, "panel")?;
            let target = target(cx, po.req(cx, "target")?, layout, Want::Import("components.Panel"))?;
            let width = cx.num(po.req(cx, "width")?, "width", 1.0, limits::COORD)? as f32;
            let height = match (po.opt("height"), po.opt("fit_menu")) {
                (Some(h), None) => PanelHeight::Fixed(cx.num(h, "height", 1.0, limits::COORD)? as f32),
                (None, Some(m)) => {
                    let menu = cx.str(m, "fit_menu", limits::PATH)?.to_string();
                    let list =
                        logic.menus.iter().find(|x| x.target == menu).map(|x| matches!(x.kind, MenuKind::List { .. }));
                    if list != Some(true) {
                        return Err(cx.err(m.pos, format!("fit_menu {menu:?}: not one of the logic's list menus")));
                    }
                    PanelHeight::FitMenu(menu)
                }
                _ => return Err(cx.err(p.pos, "panel: give exactly one of `height` and `fit_menu`")),
            };
            let dimmer = po.opt("dimmer").map(|d| cx.num(d, "dimmer", 0.0, 100.0)).transpose()?.map(|d| d as f32);
            po.finish(cx)?;
            logic.panels.push(PanelDef { pos: p.pos, target, width, height, dimmer });
        }
        for b in o.opt("billboards").map_or(Ok(&[][..]), |a| cx.array(a, "billboards", limits::MENUS))? {
            let mut bo = cx.obj(b, "billboards")?;
            let target = target(cx, bo.req(cx, "target")?, layout, Want::Import("components.HUD.BillboardContainer"))?;
            let data = data_name(cx, bo.req(cx, "data")?)?;
            let symbol = cx.name(bo.req(cx, "symbol")?, "symbol", "", limits::NAME)?.to_string();
            bo.finish(cx)?;
            logic.billboards.push(BillboardDef { pos: b.pos, target, data, symbol });
        }
        if let Some(e) = o.opt("on_enter") {
            let mut eo = cx.obj(e, "on_enter")?;
            logic.on_enter = OnEnter {
                clear_inputs: eo.opt("clear_inputs").map_or(Ok(false), |v| cx.bool(v, "clear_inputs"))?,
                event: eo.opt("event").map(|v| cx.choice(v, "event", UI_EVENTS)).transpose()?,
                sound: eo.opt("sound").map(|v| cx.choice(v, "sound", SOUNDS)).transpose()?,
            };
            eo.finish(cx)?;
            if logic.on_enter.clear_inputs && !logic.base.is_fe_view() {
                let why =
                    "on_enter.clear_inputs: `ClearInputs` is `ssx.FEView`'s (base `fe_view`, `activity_feed_view`)";
                return Err(cx.err(e.pos, why));
            }
        }
        if let Some(c) = o.opt("callouts") {
            let mut co = cx.obj(c, "callouts")?;
            let target = target(cx, co.req(cx, "target")?, layout, Want::Import("components.ButtonCalloutBar"))?;
            let mut buttons = Vec::new();
            for b in cx.array(co.req(cx, "buttons")?, "buttons", limits::CALLOUTS)? {
                let mut bo = cx.obj(b, "button")?;
                let icon = cx.choice(bo.req(cx, "icon")?, "icon", ICONS)?;
                let label = cx.name(bo.req(cx, "label")?, "label", "TXT_", limits::NAME)?.to_string();
                bo.finish(cx)?;
                buttons.push((icon, label));
            }
            co.finish(cx)?;
            logic.callouts = Some(CalloutsDef { pos: c.pos, target, buttons });
        }
        if let Some(n) = o.opt("sd") {
            let mut so = cx.obj(n, "sd")?;
            let mut moves: Vec<(String, bool)> = Vec::new();
            for (side, left) in [("left", true), ("right", false)] {
                for t in so.opt(side).map_or(Ok(&[][..]), |a| cx.array(a, side, limits::BINDINGS))? {
                    let target = target(cx, t, layout, Want::Any)?;
                    if moves.iter().any(|(m, _)| *m == target) {
                        return Err(cx.err(t.pos, format!("sd: {target:?} is given twice")));
                    }
                    moves.push((target, left));
                }
            }
            let scale = so.opt("scale").map(|v| cx.num(v, "sd scale", 0.5, 1.0)).transpose()?.map(|v| v as f32);
            so.finish(cx)?;
            let centre = (layout.stage.0 as f32 / 2.0, layout.stage.1 as f32 / 2.0);
            logic.sd = Some(SdDef { pos: n.pos, moves, scale, centre });
        }
        if let Some(p) = o.opt("paging") {
            let mut po = cx.obj(p, "paging")?;
            let menu = cx.str(po.req(cx, "menu")?, "paging menu", limits::PATH)?.to_string();
            if !logic.menus.iter().any(|m| m.target == menu && matches!(m.kind, MenuKind::List { .. })) {
                return Err(cx.err(p.pos, format!("paging menu {menu:?}: not one of the logic's list menus")));
            }
            let setup = cx.name(po.req(cx, "setup")?, "paging setup", "A_", limits::NAME)?.to_string();
            po.finish(cx)?;
            logic.paging = Some(PagingDef { pos: p.pos, menu, setup });
        }
        for d in o.opt("drop_panels").map_or(Ok(&[][..]), |a| cx.array(a, "drop_panels", limits::MENUS))? {
            let mut po = cx.obj(d, "drop panel")?;
            let target = target(cx, po.req(cx, "target")?, layout, Want::Import("components.DropSelector"))?;
            let data = known(cx, po.req(cx, "data")?, "drop panel data", &logic.data)?;
            po.finish(cx)?;
            logic.drop_panels.push(DropPanelDef { pos: d.pos, target, data });
        }
        for v in o.opt("visibles").map_or(Ok(&[][..]), |a| cx.array(a, "visibles", limits::BINDINGS))? {
            let mut vo = cx.obj(v, "visible binding")?;
            let target = target(cx, vo.req(cx, "target")?, layout, Want::Any)?;
            let data = known(cx, vo.req(cx, "data")?, "visible binding data", &logic.data)?;
            let field = cx.name(vo.req(cx, "field")?, "field", "", limits::NAME)?.to_string();
            vo.finish(cx)?;
            logic.visibles.push(VisibleBinding { pos: v.pos, target, data, field });
        }
        for i in o.opt("images").map_or(Ok(&[][..]), |a| cx.array(a, "images", limits::BINDINGS))? {
            let mut io = cx.obj(i, "image binding")?;
            let target = target(cx, io.req(cx, "target")?, layout, Want::Import("components.ImageLoader"))?;
            let data = known(cx, io.req(cx, "data")?, "image binding data", &logic.data)?;
            let field = cx.name(io.req(cx, "field")?, "field", "", limits::NAME)?.to_string();
            io.finish(cx)?;
            logic.images.push(ImageBinding { pos: i.pos, target, data, field });
        }
        for b in o.opt("scrollbars").map_or(Ok(&[][..]), |a| cx.array(a, "scrollbars", limits::MENUS))? {
            let def = read_scrollbar(cx, b, layout, &logic, &known)?;
            if logic.scrollbars.iter().any(|x| x.target == def.target || x.menu == def.menu) {
                return Err(cx.err(b.pos, format!("scrollbar {:?}: a bar or its menu is given twice", def.target)));
            }
            logic.scrollbars.push(def);
        }
        let mut keys = HashSet::new();
        for i in o.opt("inputs").map_or(Ok(&[][..]), |a| cx.array(a, "inputs", limits::INPUTS))? {
            let def = read_input(cx, i, &logic)?;
            if !keys.insert(def.key) {
                return Err(cx.err(i.pos, format!("input {}: mapped twice", def.key)));
            }
            logic.inputs.push(def);
        }
        o.finish(cx)?;
        Ok(logic)
    }
}

fn data_name(cx: &Cx, n: &Node) -> Result<String, LayoutError> {
    let s = cx.name(n, "data name", "", limits::NAME)?;
    if s.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(cx.err(n.pos, format!("data name {s:?}: lower case (the `.dd` view names are)")));
    }
    Ok(s.to_string())
}

enum Want {
    Any,
    Text,
    Import(&'static str),
    Menu,
}

fn target(cx: &Cx, n: &Node, layout: &Layout, want: Want) -> Result<String, LayoutError> {
    let path = cx.str(n, "target", limits::PATH)?;
    if path.split('.').any(|p| !super::read::is_name(p)) {
        return Err(cx.err(n.pos, format!("target {path:?}: instance names separated by `.`")));
    }
    let Some(child) = layout.resolve_path(path) else {
        return Err(cx.err(n.pos, format!("target {path:?}: no such instance below the view symbol")));
    };
    let import = match &child.kind {
        ChildKind::Symbol(s) => layout.import(s).map(|i| i.symbol.as_str()),
        _ => None,
    };
    let ok = match want {
        Want::Any => true,
        Want::Text => matches!(child.kind, ChildKind::Text(_)),
        Want::Import(sym) => import == Some(sym),
        Want::Menu => matches!(import, Some("components.GridMenu" | "components.ListMenu")),
    };
    if !ok {
        let need = match want {
            Want::Any => "a placed child".to_string(),
            Want::Text => "a text field".to_string(),
            Want::Import(s) => format!("a placement of an import of `{s}`"),
            Want::Menu => "a placement of `components.GridMenu` or `components.ListMenu`".to_string(),
        };
        return Err(cx.err(n.pos, format!("target {path:?}: must be {need}")));
    }
    Ok(path.to_string())
}

type Known = dyn Fn(&Cx, &Node, &str, &[String]) -> Result<String, LayoutError>;

fn read_menu(cx: &Cx, m: &Node, layout: &Layout, data: &[String], known: &Known) -> Result<MenuDef, LayoutError> {
    let mut mo = cx.obj(m, "menu")?;
    let target_node = mo.req(cx, "target")?;
    let target = target(cx, target_node, layout, Want::Menu)?;
    let grid = layout
        .resolve_path(&target)
        .and_then(|c| match &c.kind {
            ChildKind::Symbol(s) => layout.import(s),
            _ => None,
        })
        .is_some_and(|i| i.symbol == "components.GridMenu");
    let dim = |cx: &Cx, o: &mut super::read::Obj, k: &str| -> Result<f32, LayoutError> {
        Ok(cx.num(o.req(cx, k)?, k, 1.0, limits::COORD)? as f32)
    };
    let gap = |cx: &Cx, o: &mut super::read::Obj, k: &str| -> Result<f32, LayoutError> {
        o.opt(k).map_or(Ok(0.0), |v| Ok(cx.num(v, k, -limits::COORD, limits::COORD)? as f32))
    };
    let wrap = mo.opt("wrap").map_or(Ok(false), |w| cx.bool(w, "wrap"))?;
    let kind = if grid {
        let columns = cx.int(mo.req(cx, "columns")?, "columns", 1, limits::GRID)? as u32;
        let rows = cx.int(mo.req(cx, "rows")?, "rows", 1, limits::GRID)? as u32;
        MenuKind::Grid {
            columns,
            rows,
            cell_width: dim(cx, &mut mo, "cell_width")?,
            cell_height: dim(cx, &mut mo, "cell_height")?,
            spacing_x: gap(cx, &mut mo, "spacing_x")?,
            spacing_y: gap(cx, &mut mo, "spacing_y")?,
            wrap,
        }
    } else {
        let rows = cx.int(mo.req(cx, "rows")?, "rows", 1, limits::GRID)? as u32;
        let panel_gaps = match mo.opt("panel_gaps") {
            Some(g) => {
                let pair = cx.array(g, "panel_gaps", 2)?;
                if pair.len() != 2 {
                    return Err(cx.err(g.pos, "panel_gaps: expected [top, bottom]"));
                }
                let n = |v: &Node| cx.num(v, "panel gap", -limits::COORD, limits::COORD).map(|v| v as f32);
                Some((n(&pair[0])?, n(&pair[1])?))
            }
            None => None,
        };
        MenuKind::List {
            rows,
            cell_width: dim(cx, &mut mo, "cell_width")?,
            cell_height: dim(cx, &mut mo, "cell_height")?,
            spacing: gap(cx, &mut mo, "spacing")?,
            panel_gaps,
            wrap,
            use_panel: mo.opt("use_panel").map_or(Ok(false), |u| cx.bool(u, "use_panel"))?,
        }
    };
    let data_name = known(cx, mo.req(cx, "data")?, "menu data", data)?;
    let index = known(cx, mo.req(cx, "index")?, "menu index", data)?;
    let cell_node = mo.req(cx, "cell")?;
    let cell = cx.name(cell_node, "cell", "", limits::NAME)?.to_string();
    if layout.symbol(&cell).is_none_or(|s| s.cell.is_none()) {
        return Err(cx.err(cell_node.pos, format!("cell: {cell:?} is not a cell symbol of the layout")));
    }
    mo.finish(cx)?;
    Ok(MenuDef { pos: m.pos, target, kind, data: data_name, index, cell })
}

fn read_input(cx: &Cx, n: &Node, logic: &Logic) -> Result<InputDef, LayoutError> {
    let mut io = cx.obj(n, "input")?;
    let key = cx.choice(io.req(cx, "key")?, "key", KEYS)?;
    let menu = io.opt("menu");
    let action = io.opt("action");
    let take = io.opt("take");
    let what = match (menu, action, take) {
        (None, None, Some(t)) => {
            if !cx.bool(t, "take")? {
                return Err(cx.err(t.pos, format!("input {key}: `take` can only be true (leave the key out instead)")));
            }
            InputDo::Take
        }
        (Some(m), None, None) => {
            let target = cx.str(m, "menu", limits::PATH)?.to_string();
            if !logic.menus.iter().any(|x| x.target == target) {
                return Err(cx.err(m.pos, format!("menu {target:?}: not one of the logic's `menus`")));
            }
            let ops = ["up", "down", "left", "right", "advance", "back", "page_up", "page_down"];
            let op = match cx.choice(io.req(cx, "do")?, "do", &ops)? {
                "up" => MenuOp::Up,
                "down" => MenuOp::Down,
                "left" => MenuOp::Left,
                "right" => MenuOp::Right,
                "advance" => MenuOp::Advance,
                "back" => MenuOp::Back,
                "page_up" => MenuOp::PageUp,
                _ => MenuOp::PageDown,
            };
            let blocked_sound = io.opt("blocked_sound").map(|s| cx.choice(s, "blocked_sound", SOUNDS)).transpose()?;
            let list = logic.menus.iter().any(|x| x.target == target && matches!(x.kind, MenuKind::List { .. }));
            if blocked_sound.is_some() && !(list && matches!(op, MenuOp::Up | MenuOp::Down)) {
                return Err(cx.err(m.pos, "blocked_sound: only for a list menu's up and down"));
            }
            let sound = io.opt("sound").map(|s| cx.choice(s, "sound", SOUNDS)).transpose()?;
            if sound.is_some() && blocked_sound.is_some() {
                return Err(cx.err(m.pos, "sound: not with `blocked_sound` (give one)"));
            }
            InputDo::Menu { target, op, blocked_sound, sound }
        }
        (None, Some(a), None) => {
            let name = cx.name(a, "action", "A_", limits::NAME)?.to_string();
            let sound = io.opt("sound").map(|s| cx.choice(s, "sound", SOUNDS)).transpose()?;
            let param = io.opt("param").map(|p| cx.num(p, "param", -limits::COORD, limits::COORD)).transpose()?;
            InputDo::Action { name, sound, param }
        }
        _ => {
            return Err(
                cx.err(n.pos, format!("input {key}: give exactly one of `menu` (with `do`), `action` and `take`"))
            );
        }
    };
    let lock_ms = io.opt("lock_ms").map(|v| cx.num(v, "lock_ms", 1.0, 5000.0)).transpose()?;
    io.finish(cx)?;
    Ok(InputDef { pos: n.pos, key, what, lock_ms })
}

fn read_scrollbar(
    cx: &Cx,
    n: &Node,
    layout: &Layout,
    logic: &Logic,
    known: &Known,
) -> Result<ScrollBarDef, LayoutError> {
    let mut bo = cx.obj(n, "scrollbar")?;
    let target = target(cx, bo.req(cx, "target")?, layout, Want::Import("components.ScrollBar"))?;
    let menu_node = bo.req(cx, "menu")?;
    let menu = cx.str(menu_node, "scrollbar menu", limits::PATH)?.to_string();
    if !logic.menus.iter().any(|m| m.target == menu && matches!(m.kind, MenuKind::List { .. })) {
        return Err(cx.err(menu_node.pos, format!("scrollbar menu {menu:?}: not one of the logic's list menus")));
    }
    let length = cx.num(bo.req(cx, "length")?, "length", 1.0, limits::COORD)? as f32;
    let window = match bo.opt("window") {
        Some(w) => {
            let data = known(cx, w, "scrollbar window", &logic.data)?;
            if logic.paging.as_ref().is_none_or(|p| p.menu != menu) {
                return Err(cx.err(w.pos, format!("scrollbar window: the menu {menu:?} is not the paging's")));
            }
            Some(data)
        }
        None => None,
    };
    bo.finish(cx)?;
    Ok(ScrollBarDef { pos: n.pos, target, menu, length, window })
}
