use std::collections::HashMap;

use crate::avm1::Ctx;
use crate::avm1::object::ObjId;
use crate::avm1::value::Value;
use crate::display::InstanceId;
use crate::display::hit::Hit;
use crate::game::input::Button;
use crate::geom::RectF;

const MAX_CELLS: i32 = 128;
const MAX_VISITS: usize = 50_000;
const MAX_LISTENERS: usize = 64;

pub(super) struct List {
    pub clip: InstanceId,
    pub selected: i32,
    pub cells: Vec<Cell>,
    pub selected_cell: Option<InstanceId>,
}

#[derive(Clone, Copy)]
pub(super) struct Cell {
    pub item: i32,
    pub clip: InstanceId,
    pub slot: RectF,
    pub preview: bool,
}

pub(super) struct Reach {
    pub lists: Vec<List>,
    rank: HashMap<InstanceId, usize>,
    owner: Option<usize>,
    pub prompts_in: Option<InstanceId>,
}

impl Reach {
    pub(super) fn prompts(&self, cx: &Ctx, prompts: Vec<InstanceId>) -> Vec<InstanceId> {
        let owned: Vec<(usize, InstanceId)> = prompts
            .into_iter()
            .filter(|id| self.prompts_in.is_none_or(|top| holds(cx, top, *id)))
            .filter_map(|id| owner(cx, &self.rank, id).map(|o| (o, id)))
            .collect();
        let limit = self.owner.or_else(|| owned.iter().map(|(o, _)| *o).min());
        owned.into_iter().filter(|(o, _)| limit.is_some_and(|l| *o <= l)).map(|(_, id)| id).collect()
    }
}

pub(super) fn global_path(cx: &mut Ctx, path: &str) -> Value {
    let mut v = Value::Object(cx.vm.global);
    for p in path.split('.') {
        v = cx.get_member(&v, p).unwrap_or_default();
    }
    v
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Menu,
    Grid,
    Popup,
    Label,
    Billboards,
}

impl Kind {
    fn marks(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Kind::Menu => (&["GetCellByIndex", "GetSelectedIndex", "SetSelectedIndex", "CanHighlightIndex"], &[]),
            Kind::Grid => (&["GetCellByIndex", "MoveHighlightRight", "GetRowByCellIndex"], &[]),
            Kind::Popup => (&["HandleInput", "Publish", "mNumButtons"], &["mcPopup"]),
            Kind::Label => (&["SetButtonIcon", "SetEventType", "GetButtonWidth"], &[]),
            Kind::Billboards => (&["AddBillboardDataProvider", "EnsureCorrectNumberOfBillboards"], &[]),
        }
    }
}

fn has(cx: &Ctx, obj: ObjId, name: &str) -> bool {
    let mut cur = Some(obj);
    for _ in 0..256 {
        let Some(o) = cur.and_then(|c| cx.vm.heap.get(c)) else { return false };
        if o.props.contains(name) {
            return true;
        }
        cur = o.proto.as_object();
    }
    false
}

pub(super) fn is(cx: &Ctx, clip: InstanceId, kind: Kind) -> bool {
    let Some(&obj) = cx.vm.display_objs.get(&clip) else { return false };
    let (members, children) = kind.marks();
    members.iter().all(|m| has(cx, obj, m)) && children.iter().all(|c| cx.player.child_by_name(clip, c, true).is_some())
}

pub(super) fn instances_of(cx: &Ctx, order: &[InstanceId], kind: Kind) -> Vec<InstanceId> {
    order.iter().copied().filter(|&id| is(cx, id, kind)).collect()
}

pub(super) fn paint_order(cx: &Ctx) -> Vec<InstanceId> {
    let mut out = Vec::new();
    let mut stack: Vec<InstanceId> = cx.player.levels.values().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if out.len() >= MAX_VISITS {
            break;
        }
        let Some(inst) = cx.player.get(id) else { continue };
        if !inst.visible || inst.clip_depth.is_some() {
            continue;
        }
        out.push(id);
        for (_, c) in cx.player.children(id).into_iter().rev() {
            stack.push(c);
        }
    }
    out
}

pub(super) fn number(cx: &mut Ctx, obj: &Value, name: &str) -> Option<f64> {
    let v = cx.get_member(obj, name).ok()?;
    let n = cx.to_number(&v);
    n.is_finite().then_some(n)
}

pub(super) fn flag(cx: &mut Ctx, obj: &Value, name: &str) -> bool {
    let v = cx.get_member(obj, name).unwrap_or_default();
    cx.to_bool(&v)
}

fn count(cx: &mut Ctx, obj: &Value, name: &str, max: i32) -> Option<i32> {
    number(cx, obj, name).map(|n| n.clamp(0.0, f64::from(max)) as i32)
}

pub(super) fn read_menu(cx: &mut Ctx, clip: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(clip));
    let grid = is(cx, clip, Kind::Grid);
    if !flag(cx, &obj, "mIsActive") || (!grid && flag(cx, &obj, "mIsPassive")) {
        return None;
    }
    menu_cells(cx, clip)
}

fn menu_cells(cx: &mut Ctx, clip: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(clip));
    let first = number(cx, &obj, "mFirstVisibleItemIndex")?.clamp(0.0, f64::from(i32::MAX)) as i32;
    let selected_index = number(cx, &obj, "mSelectedCellIndex")?.clamp(-1.0, f64::from(MAX_CELLS)) as i32;
    let total = count(cx, &obj, "mNumTotalItems", i32::MAX)?;
    let visible = count(cx, &obj, "mNumVisibleItems", MAX_CELLS)?;
    let circular = flag(cx, &obj, "mIsCircular") && total > 0;
    let wrap = |i: i32| if circular { i.rem_euclid(total) } else { i };
    let selected = wrap(first.saturating_add(selected_index));
    let mut selected_cell = None;
    let (get, _) = cx.get_method(&obj, "GetCellByIndex").ok()?;
    let width = number(cx, &obj, "mCellWidth").filter(|w| *w > 0.0);
    let (custom, _) = cx.get_method(&obj, "GetCellHeight").ok().unwrap_or_default();
    let fixed = number(cx, &obj, "mCellHeight").filter(|h| *h > 0.0);
    let band = selectable_band(cx, &obj, visible, selected_index);
    let mut cells = Vec::new();
    for i in 0..visible {
        let item = wrap(first.saturating_add(i));
        if item >= total {
            continue;
        }
        let cell = cx.call_top(&get, obj.clone(), &[Value::Number(f64::from(i))]);
        let Some(c) = cx.display_of(&cell) else { continue };
        if i == selected_index {
            selected_cell = Some(c);
        }
        let height = if cx.is_function(&custom) {
            let h = cx.call_top(&custom, obj.clone(), &[Value::Number(f64::from(i))]);
            Some(cx.to_number(&h)).filter(|h| h.is_finite() && *h > 0.0)
        } else {
            fixed
        };
        let Some(m) = cx.player.get(c).map(|i| i.matrix) else { continue };
        let slot = match (width, height) {
            (Some(w), Some(h)) => {
                let (w, h) = ((w.min(1e5) * 20.0) as f32, (h.min(1e5) * 20.0) as f32);
                RectF::new(m.tx, m.ty, m.tx + w, m.ty + h)
            }
            _ => cx.player.local_bounds(cx.lib, c).transform(&m),
        };
        cells.push(Cell { item, clip: c, slot, preview: band.is_some_and(|(a, b)| i < a || i > b) });
    }
    Some(List { clip, selected, cells, selected_cell })
}

fn selectable_band(cx: &mut Ctx, obj: &Value, visible: i32, selected_index: i32) -> Option<(i32, i32)> {
    let cells = |v: f64| v.clamp(0.0, f64::from(MAX_CELLS)) as i32;
    if let (Some(left), Some(right)) = (number(cx, obj, "mUnselectableLeft"), number(cx, obj, "mUnselectableRight")) {
        if flag(cx, obj, "mStaticIndex") {
            return Some((selected_index, selected_index));
        }
        return Some((cells(left), visible - cells(right) - 1));
    }
    if number(cx, obj, "mNumUnselectableGridsTotal").is_some_and(|t| t > 0.0) {
        let top = number(cx, obj, "mNumUnselectableGridsTop")?;
        let bottom = number(cx, obj, "mNumUnselectableGridsBottom")?;
        return Some((cells(top), visible - cells(bottom) - 1));
    }
    None
}

pub(super) fn read_popup(cx: &mut Ctx, clip: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(clip));
    if !flag(cx, &obj, "mReceivedPublish") {
        return None;
    }
    let n = count(cx, &obj, "mNumButtons", MAX_CELLS)?;
    let selected = number(cx, &obj, "mSelectedIndex")?.clamp(-1.0, f64::from(MAX_CELLS)) as i32;
    let popup = cx.get_member(&obj, "mcPopup").ok()?;
    let panel = cx.get_member(&popup, "mcPanelContainer").ok()?;
    let mut cells = Vec::new();
    for i in 0..n {
        let b = cx.get_member(&panel, &format!("mcButton{i}")).ok()?;
        let Some(c) = cx.display_of(&b) else { continue };
        let Some(m) = cx.player.get(c).map(|i| i.matrix) else { continue };
        let slot = crate::display::hit::drawn_bounds(cx.player, cx.lib, c).transform(&m);
        cells.push(Cell { item: i, clip: c, slot, preview: false });
    }
    stack(cx, &mut cells);
    Some(List { clip, selected, cells, selected_cell: None })
}

pub(super) fn read_rows(cx: &mut Ctx, view: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(view));
    let rows = cx.get_member(&obj, "mRowItemArray").ok().filter(|v| !v.is_nullish())?;
    let n = count(cx, &obj, "mNumRows", MAX_CELLS)?;
    let selected = number(cx, &obj, "mCurRowIdx")?.clamp(-1.0, f64::from(MAX_CELLS)) as i32;
    let mut cells = Vec::new();
    for i in 0..n {
        let Ok(row) = cx.get_member(&rows, &i.to_string()) else { continue };
        let Some(c) = cx.display_of(&row) else { continue };
        let Some(m) = cx.player.get(c).map(|i| i.matrix) else { continue };
        let slot = crate::display::hit::drawn_bounds(cx.player, cx.lib, c).transform(&m);
        cells.push(Cell { item: i, clip: c, slot, preview: false });
    }
    stack(cx, &mut cells);
    Some(List { clip: view, selected, cells, selected_cell: None })
}

pub(super) fn read_panels(cx: &mut Ctx, view: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(view));
    let panels = cx.get_member(&obj, "mPanelRefs").ok().filter(|v| !v.is_nullish())?;
    let cursor = cx.get_member(&obj, "mCursorPos").ok().filter(|v| !v.is_nullish())?;
    let cols = count(cx, &panels, "length", MAX_CELLS)?;
    let mut cells = Vec::new();
    let mut rows_max = 0;
    let mut columns = Vec::new();
    for c in 0..cols {
        let Ok(column) = cx.get_member(&panels, &c.to_string()) else { continue };
        let rows = count(cx, &column, "length", MAX_CELLS).unwrap_or(0);
        rows_max = rows_max.max(rows);
        columns.push((c, column, rows));
    }
    for (c, column, rows) in columns {
        for r in 0..rows {
            let Ok(panel) = cx.get_member(&column, &r.to_string()) else { continue };
            let Some(id) = cx.display_of(&panel) else { continue };
            let Some(m) = cx.player.get(id).map(|i| i.matrix) else { continue };
            let slot = crate::display::hit::drawn_bounds(cx.player, cx.lib, id).transform(&m);
            cells.push(Cell { item: c * rows_max + r, clip: id, slot, preview: false });
        }
    }
    let (col, row) = (number(cx, &cursor, "col")? as i32, number(cx, &cursor, "row")? as i32);
    Some(List { clip: view, selected: col.saturating_mul(rows_max).saturating_add(row), cells, selected_cell: None })
}

pub(super) fn read_equipment(cx: &mut Ctx, view: InstanceId) -> Option<(List, Vec<InstanceId>)> {
    let obj = Value::Object(cx.object_for(view));
    let selected_start = cx.get_member(&obj, "mAdvanceSelected").ok().filter(|v| !v.is_nullish())?;
    let root = cx.get_member(&obj, "mcEquipmentRoot").ok()?;
    let slots = cx.get_member(&root, "mcSlots").ok()?;
    let menu = cx.get_member(&slots, "mcCategoryMenu").ok().and_then(|m| cx.display_of(&m))?;
    if let Some(grids) = read_gear_grids(cx, &obj, view, menu) {
        return Some(grids);
    }
    let start = cx.get_member(&slots, "mcAdvance").ok().and_then(|a| cx.display_of(&a))?;
    let menu_obj = Value::Object(cx.object_for(menu));
    if !flag(cx, &menu_obj, "mIsActive") {
        return None;
    }
    let mut list = menu_cells(cx, menu)?;
    let m = cx.player.get(start).map(|i| i.matrix)?;
    let slot = crate::display::hit::drawn_bounds(cx.player, cx.lib, start).transform(&m);
    list.cells.insert(0, Cell { item: -1, clip: start, slot, preview: false });
    if cx.to_bool(&selected_start) {
        list.selected = -1;
    }
    list.clip = view;
    Some((list, vec![menu]))
}

fn read_gear_grids(
    cx: &mut Ctx,
    obj: &Value,
    view: InstanceId,
    category: InstanceId,
) -> Option<(List, Vec<InstanceId>)> {
    let class = global_path(cx, "Equipment");
    let gear_state = number(cx, &class, "SCREEN_STATE_GEAR_SELECT")?;
    let fsm = cx.get_member(obj, "mScreenFSM").ok()?;
    if number(cx, &fsm, "mCurrentStateId")? != gear_state {
        return None;
    }
    let root = cx.get_member(obj, "mcEquipmentRoot").ok()?;
    let container = cx.get_member(&root, "mcSelectionPanelContainer").ok()?;
    let panel = cx.get_member(&container, "mcSelectionPanel").ok()?;
    let menus = cx.get_member(&panel, "mcGearMenus").ok()?;
    let gear = cx.get_member(&menus, "mcGearMenu").ok().and_then(|m| cx.display_of(&m))?;
    let store = cx.get_member(&menus, "mcStoreMenu").ok().and_then(|m| cx.display_of(&m))?;
    let current = cx.get_member(obj, "mcCurrentMenu").ok().and_then(|m| cx.display_of(&m))?;
    let gear_list = menu_cells(cx, gear)?;
    let store_list = menu_cells(cx, store)?;
    let gear_obj = Value::Object(cx.object_for(gear));
    let gear_total = count(cx, &gear_obj, "mNumTotalItems", i32::MAX)?;
    let shifted = |i: i32| if i < 0 { i } else { gear_total.saturating_add(i) };
    let (selected, selected_cell) = if current == gear {
        (gear_list.selected, gear_list.selected_cell)
    } else if current == store {
        (shifted(store_list.selected), store_list.selected_cell)
    } else {
        return None;
    };
    let mut cells = gear_list.cells;
    cells.extend(store_list.cells.into_iter().map(|c| Cell { item: shifted(c.item), ..c }));
    Some((List { clip: view, selected, cells, selected_cell }, vec![gear, store, category]))
}

fn row_cells(cx: &mut Ctx, panel: &Value, prefix: &str, n: i32) -> Vec<Cell> {
    let mut cells = Vec::new();
    for i in 0..n.clamp(0, MAX_CELLS) {
        let Ok(row) = cx.get_member(panel, &format!("{prefix}{i}")) else { continue };
        let Some(c) = cx.display_of(&row) else { continue };
        let Some(m) = cx.player.get(c).map(|i| i.matrix) else { continue };
        let slot = crate::display::hit::drawn_bounds(cx.player, cx.lib, c).transform(&m);
        cells.push(Cell { item: i, clip: c, slot, preview: false });
    }
    stack(cx, &mut cells);
    cells
}

fn stack(cx: &Ctx, cells: &mut [Cell]) {
    if cells.len() < 2 {
        return;
    }
    let parent = |c: &Cell| cx.player.get(c.clip).and_then(|i| i.parent);
    let top = |c: &Cell| cx.player.get(c.clip).map_or(0.0, |i| i.matrix.ty);
    let same_parent = cells.iter().all(|c| parent(c) == parent(&cells[0]));
    let tops: Vec<f32> = cells.iter().map(top).collect();
    if !same_parent || !tops.windows(2).all(|w| w[1] > w[0]) {
        return;
    }
    let step = tops.windows(2).map(|w| w[1] - w[0]).fold(f32::INFINITY, f32::min);
    let (x0, x1) = cells
        .iter()
        .filter(|c| !c.slot.is_empty())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), c| (a.min(c.slot.x0), b.max(c.slot.x1)));
    if !x0.is_finite() || !x1.is_finite() {
        return;
    }
    for (i, c) in cells.iter_mut().enumerate() {
        let bottom = tops.get(i + 1).copied().unwrap_or(tops[i] + step);
        c.slot = RectF::new(x0, tops[i], x1, bottom);
    }
}

pub(super) fn read_music(cx: &mut Ctx, view: InstanceId) -> Option<List> {
    let obj = Value::Object(cx.object_for(view));
    let class = |cx: &mut Ctx, path: &str| {
        let v = global_path(cx, path);
        let n = cx.to_number(&v);
        n.is_finite().then_some(n)
    };
    let indices = cx.get_member(&obj, "mIndices").ok().filter(|v| !v.is_nullish());
    let state = number(cx, &obj, "mState");
    if let (Some(indices), Some(state)) = (indices, state) {
        let state = state as i32;
        let selected = number(cx, &indices, &state.to_string())? as i32;
        let (panel, prefix, n) = match state {
            0 => ("mcCategoryPanel", "mcCategory", class(cx, "ActivityFeedPersonalizeMusic.NUM_CATEGORIES")?),
            1 => {
                let sorts = cx.get_member(&obj, "mSortTypes").ok()?;
                ("mcSubmenu", "mcRow", number(cx, &sorts, "length")?)
            }
            2 => {
                let max = class(cx, "ActivityFeedPersonalizeMusic.MAX_NUM_SONG_ROWS")?;
                ("mcSongPanel", "mcRow", number(cx, &obj, "mNumMusicData")?.min(max))
            }
            _ => return None,
        };
        let panel = cx.get_member(&obj, panel).ok()?;
        let cells = row_cells(cx, &panel, prefix, n as i32);
        return Some(List { clip: view, selected, cells, selected_cell: None });
    }
    let index = number(cx, &obj, "mIndex")?;
    let songs = number(cx, &obj, "mNumMusicData")?;
    let max = class(cx, "ActivityFeedTrackListing.MAX_NUM_SONG_ROWS")?;
    let panel = cx.get_member(&obj, "mcSongPanel").ok().filter(|v| !v.is_nullish())?;
    let cells = row_cells(cx, &panel, "mcRow", songs.min(max) as i32);
    Some(List { clip: view, selected: index as i32, cells, selected_cell: None })
}

pub(super) fn listeners(cx: &mut Ctx, subscribers: &[String]) -> Vec<(InstanceId, f64)> {
    let mut out = Vec::new();
    for path in subscribers {
        let Some(manager) = crate::game::resolve_path(cx, path) else { continue };
        let min = number(cx, &manager, "mMinPriority").unwrap_or(f64::NEG_INFINITY);
        let Ok(list) = cx.get_member(&manager, "mListenerLists") else { continue };
        let n = count(cx, &list, "length", MAX_LISTENERS as i32).unwrap_or(0);
        for i in (0..n).rev() {
            let Ok(entry) = cx.get_member(&list, &i.to_string()) else { continue };
            let priority = number(cx, &entry, "priority").unwrap_or(0.0);
            let Ok(l) = cx.get_member(&entry, "listener") else { continue };
            if priority < min {
                continue;
            }
            if let Some(id) = cx.display_of(&l) {
                out.push((id, priority));
            }
        }
    }
    out
}

fn owner(cx: &Ctx, rank: &HashMap<InstanceId, usize>, clip: InstanceId) -> Option<usize> {
    let mut cur = Some(clip);
    let mut best: Option<usize> = None;
    for _ in 0..512 {
        let Some(c) = cur else { break };
        if let Some(&r) = rank.get(&c) {
            best = Some(best.map_or(r, |b| b.min(r)));
        }
        cur = cx.player.get(c).and_then(|i| i.parent);
    }
    best
}

pub(super) fn reach(cx: &mut Ctx, subscribers: &[String], order: &[InstanceId]) -> Reach {
    let listeners = listeners(cx, subscribers);
    let rank: HashMap<InstanceId, usize> = listeners.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
    let mut lists = Vec::new();
    lists.extend(instances_of(cx, order, Kind::Menu).into_iter().filter_map(|c| read_menu(cx, c)));
    lists.extend(instances_of(cx, order, Kind::Popup).into_iter().filter_map(|c| read_popup(cx, c)));
    lists.extend(listeners.iter().filter_map(|&(view, _)| read_rows(cx, view)));
    lists.extend(listeners.iter().filter_map(|&(view, _)| read_panels(cx, view)));
    lists.extend(listeners.iter().filter_map(|&(view, _)| read_music(cx, view)));
    let equipment: Vec<(List, Vec<InstanceId>)> =
        listeners.iter().filter_map(|&(view, _)| read_equipment(cx, view)).collect();
    lists.retain(|l| !equipment.iter().any(|(_, folded)| folded.contains(&l.clip)));
    lists.extend(equipment.into_iter().map(|(l, _)| l));
    let owned: Vec<(usize, List)> =
        lists.into_iter().filter_map(|l| owner(cx, &rank, l.clip).map(|o| (o, l))).collect();
    let first = owned.iter().map(|(o, _)| *o).min();
    let popup = {
        let manager = global_path(cx, "ux.input.InputManager");
        let v = cx.get_member(&manager, "INPUT_PRIORITY_POPUP").unwrap_or_default();
        let v = if v.is_nullish() {
            let proto = cx.get_member(&manager, "prototype").unwrap_or_default();
            cx.get_member(&proto, "INPUT_PRIORITY_POPUP").unwrap_or_default()
        } else {
            v
        };
        let n = cx.to_number(&v);
        n.is_finite().then_some(n)
    };
    let prompts_in = match (listeners.first(), popup) {
        (Some(&(top, p)), Some(popup)) if p >= popup => Some(top),
        _ => None,
    };
    let lists = owned
        .into_iter()
        .filter(|(o, l)| Some(*o) == first && prompts_in.is_none_or(|top| holds(cx, top, l.clip)))
        .map(|(_, l)| l)
        .collect();
    Reach { lists, rank, owner: first, prompts_in }
}

pub(super) fn holds(cx: &Ctx, outer: InstanceId, clip: InstanceId) -> bool {
    let mut cur = Some(clip);
    for _ in 0..512 {
        let Some(c) = cur else { return false };
        if c == outer {
            return true;
        }
        cur = cx.player.get(c).and_then(|i| i.parent);
    }
    false
}

pub(super) fn cell_hit(cx: &Ctx, hit: &mut Hit, c: &Cell, stage: (f32, f32)) -> bool {
    let parent = cx.player.get(c.clip).and_then(|i| i.parent);
    let inside = parent.and_then(|p| cx.player.to_local(p, stage)).is_some_and(|p| c.slot.contains(p.0, p.1));
    inside && hit.shown_at(c.clip, stage) && drawn(cx, c.clip)
}

pub(super) fn drawn(cx: &Ctx, clip: InstanceId) -> bool {
    let mut chain = Vec::new();
    let mut cur = Some(clip);
    while let Some(c) = cur {
        let Some(i) = cx.player.get(c) else { return false };
        chain.push(i.cxform);
        cur = i.parent;
        if chain.len() > 512 {
            return false;
        }
    }
    let total = chain.iter().rev().fold(crate::geom::Cxform::IDENTITY, |acc, c| acc.concat(c));
    total.apply([1.0; 4])[3] > 1.0 / 255.0
}

pub(super) fn slot_centre(cx: &Ctx, c: &Cell) -> Option<(f32, f32)> {
    let parent = cx.player.get(c.clip).and_then(|i| i.parent)?;
    let (x, y) = ((c.slot.x0 + c.slot.x1) / 2.0, (c.slot.y0 + c.slot.y1) / 2.0);
    Some(cx.player.world_matrix(parent).apply(x, y))
}

pub(super) fn prompt_button(cx: &mut Ctx, label: InstanceId) -> Option<Button> {
    let obj = Value::Object(cx.object_for(label));
    let icon = number(cx, &obj, "mIconId")?;
    let callout = global_path(cx, "components.ButtonCallout");
    let id = |cx: &mut Ctx, name: &str| number(cx, &callout, name);
    let circle = flag(cx, &callout, "mIsCircleAdvance");
    let (primary, secondary) = (id(cx, "CALLOUT_FACE_BUTTON_PRIMARY")?, id(cx, "CALLOUT_FACE_BUTTON_SECONDARY")?);
    let icon = if Some(icon) == id(cx, "CALLOUT_ADVANCE") {
        if circle { secondary } else { primary }
    } else if Some(icon) == id(cx, "CALLOUT_BACK") {
        if circle { primary } else { secondary }
    } else {
        icon
    };
    let table: [(&str, Button); 19] = [
        ("CALLOUT_FACE_BUTTON_PRIMARY", Button::DigitalOne),
        ("CALLOUT_FACE_BUTTON_SECONDARY", Button::DigitalTwo),
        ("CALLOUT_FACE_BUTTON_TERTIARY", Button::DigitalFour),
        ("CALLOUT_FACE_BUTTON_QUATERNARY", Button::DigitalThree),
        ("CALLOUT_START", Button::Start),
        ("CALLOUT_SELECT", Button::Select),
        ("CALLOUT_L1", Button::LeftBumper),
        ("CALLOUT_R1", Button::RightBumper),
        ("CALLOUT_L2", Button::LeftTrigger),
        ("CALLOUT_R2", Button::RightTrigger),
        ("CALLOUT_R3", Button::Ras),
        ("CALLOUT_DPAD_UP", Button::DpadUp),
        ("CALLOUT_DPAD_DOWN", Button::DpadDown),
        ("CALLOUT_DPAD_LEFT", Button::DpadLeft),
        ("CALLOUT_DPAD_RIGHT", Button::DpadRight),
        ("CALLOUT_LAS_UP", Button::DpadUp),
        ("CALLOUT_LAS_DOWN", Button::DpadDown),
        ("CALLOUT_LAS_LEFT", Button::DpadLeft),
        ("CALLOUT_LAS_RIGHT", Button::DpadRight),
    ];
    table.iter().find(|(n, _)| id(cx, n) == Some(icon)).map(|(_, b)| *b)
}

pub(super) fn step_ways(cx: &Ctx, list: &List, item: i32) -> Vec<Button> {
    let at = match list.selected_cell {
        Some(clip) => list.cells.iter().find(|c| c.clip == clip),
        None => list.cells.iter().find(|c| c.item == list.selected),
    };
    let from = at.and_then(|c| slot_centre(cx, c));
    let to = list.cells.iter().filter(|c| c.item == item).filter_map(|c| slot_centre(cx, c)).min_by(|a, b| {
        let d = |p: &(f32, f32)| from.map_or(0.0, |f| (p.0 - f.0).hypot(p.1 - f.1));
        d(a).total_cmp(&d(b))
    });
    if let (Some(a), Some(b)) = (from, to) {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let across = (dx.abs() >= 20.0).then_some(if dx > 0.0 { Button::DpadRight } else { Button::DpadLeft });
        let down = (dy.abs() >= 20.0).then_some(if dy > 0.0 { Button::DpadDown } else { Button::DpadUp });
        let ways = if dx.abs() > dy.abs() { [across, down] } else { [down, across] };
        return ways.into_iter().flatten().collect();
    }
    let centres: Vec<(f32, f32)> = list.cells.iter().filter_map(|c| slot_centre(cx, c)).collect();
    let spread = |f: fn(&(f32, f32)) -> f32| {
        let v: Vec<f32> = centres.iter().map(f).collect();
        v.iter().copied().fold(f32::NEG_INFINITY, f32::max) - v.iter().copied().fold(f32::INFINITY, f32::min)
    };
    let horizontal = centres.len() > 1 && spread(|c| c.0) > spread(|c| c.1);
    vec![match (horizontal, item > list.selected) {
        (true, true) => Button::DpadRight,
        (true, false) => Button::DpadLeft,
        (false, true) => Button::DpadDown,
        (false, false) => Button::DpadUp,
    }]
}

pub(super) fn billboards(cx: &mut Ctx, order: &[InstanceId]) -> Vec<(u32, InstanceId)> {
    let mut out = Vec::new();
    for container in instances_of(cx, order, Kind::Billboards) {
        let obj = Value::Object(cx.object_for(container));
        let (Ok(names), Ok(clips), Ok(data)) = (
            cx.get_member(&obj, "mProviderNames"),
            cx.get_member(&obj, "mBillboardClips"),
            cx.get_member(&obj, "mBillboardData"),
        ) else {
            continue;
        };
        let providers = count(cx, &names, "length", MAX_LISTENERS as i32).unwrap_or(0);
        for p in 0..providers {
            let Ok(name) = cx.get_member(&names, &p.to_string()) else { continue };
            let Ok(name) = cx.to_string(&name).map(|s| s.to_string()) else { continue };
            let (Ok(cl), dl) = (cx.get_member(&clips, &name), data.clone()) else { continue };
            let n = count(cx, &cl, "length", MAX_CELLS).unwrap_or(0);
            for i in 0..n {
                let (Ok(clip), Ok(item)) = (cx.get_member(&cl, &i.to_string()), cx.get_member(&dl, &i.to_string()))
                else {
                    continue;
                };
                let Some(id) = cx.display_of(&clip) else { continue };
                let Some(key) = number(cx, &item, "billboardid").map(|k| k as i32 as u32) else { continue };
                out.push((key, id));
            }
        }
    }
    out
}
