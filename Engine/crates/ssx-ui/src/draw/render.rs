use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::text::Font;

use crate::display::{BlendMode, Instance, InstanceId, InstanceKind, Player};
use crate::geom::{Cxform, Mat2d, RectF};
use crate::glyph_filter::{GlyphFilter, ShadowParam, TextFieldParam, shadow_size};
use crate::library::{CharRef, Character, Library};

pub mod cover;
pub mod overlay;
mod stage;
use crate::scale9::Slice9;
use crate::shape::{self, Paint, ShapeMesh, Tolerance};
use crate::tess::{Budget, Mesh, OverBudget, Work};
pub use stage::{ScaleMode, stage_scale, stage_transform};

#[derive(Clone, Debug)]
pub struct DrawItem {
    pub mesh: Arc<Mesh>,
    pub paint: Paint,
    pub transform: Mat2d,
    pub cxform: Cxform,
    pub blend: BlendMode,
}

#[derive(Clone, Debug)]
pub enum Command {
    Draw(DrawItem),
    MaskBegin,
    MaskEnd,
    MaskPopBegin,
    MaskPopEnd,
}

#[derive(Clone, Debug, Default)]
pub struct RenderList {
    pub commands: Vec<Command>,
    pub viewport: (u32, u32),
    pub background: Option<[f32; 4]>,
    pub stage: Option<[f32; 4]>,
}

impl RenderList {
    pub fn draw_count(&self) -> usize {
        self.commands.iter().filter(|c| matches!(c, Command::Draw(_))).count()
    }

    pub fn triangle_count(&self) -> usize {
        self.commands
            .iter()
            .map(|c| match c {
                Command::Draw(d) => d.mesh.triangle_count(),
                _ => 0,
            })
            .sum()
    }
}

// TODO(guess): fixed power-of-two buckets instead of Scaleform's history-dependent reuse band; settle: the mesh cache's reuse test.
fn scale_bucket(scale: f32) -> i32 {
    scale.max(1e-6).log2().round() as i32
}

fn bucket_tolerance(bucket: i32) -> Tolerance {
    Tolerance::for_scale(2f32.powf(bucket as f32))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MeshKey {
    Shape(CharRef, i32),
    Morph(CharRef, u16, i32),
    Glyph(usize, usize, i32),
    Bitmap(CharRef),
}

pub const FRAME_TESSELLATION: Work = Work { edges: 1 << 19, steps: 1 << 22, vertices: 1 << 20 };

pub const CACHE_VERTICES: u64 = 1 << 21;

const REFUSED_KEPT: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TessStats {
    pub shape_peak: Work,
    pub frame_peak: Work,
    pub deferred: u64,
    pub refused: u64,
    pub evicted: u64,
    pub cached_vertices_peak: u64,
}

struct Cached {
    mesh: Arc<ShapeMesh>,
    vertices: u64,
    frame: u64,
    owner: Option<std::sync::Weak<Font>>,
}

#[derive(Default)]
pub struct Renderer {
    meshes: HashMap<MeshKey, Cached>,
    cached_vertices: u64,
    frame: u64,
    frame_work: Work,
    pub tess_stats: TessStats,
    refused: Vec<String>,
    pub tessellated: usize,
    pub glyphs: crate::glyph::GlyphCache,
    pub no_filters: bool,
    overlay: overlay::OverlayCache,
}

#[derive(Default)]
struct TextLayers {
    shadow: Vec<Command>,
    raster: Vec<Command>,
    vector: Vec<Command>,
}

impl TextLayers {
    fn emit(self, out: &mut Vec<Command>) {
        out.extend(self.shadow);
        out.extend(self.raster);
        out.extend(self.vector);
    }
}

#[derive(Clone, Copy)]
struct Inherited {
    xf: Mat2d,
    cx: Cxform,
    blend: BlendMode,
    mask: bool,
    slice: Option<(Slice9, Mat2d, Mat2d)>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_frame(&mut self) {
        self.glyphs.begin_frame();
        self.tess_stats.frame_peak.peak(self.frame_work);
        self.frame += 1;
        self.frame_work = Work::default();
    }

    pub fn frame_work(&self) -> Work {
        self.frame_work
    }

    pub fn take_refused(&mut self) -> Vec<String> {
        std::mem::take(&mut self.refused)
    }

    pub fn build(&mut self, lib: &Library, player: &Player, viewport: (u32, u32), mode: ScaleMode) -> RenderList {
        self.new_frame();
        let mut out = RenderList { commands: Vec::new(), viewport, background: None, stage: None };
        let Some(root) = player.levels.values().next().and_then(|r| player.get(*r)) else { return out };
        let movie = lib.movie(root.movie);
        out.background = movie.background.map(crate::geom::rgba);
        let stage = stage_transform(movie.stage, viewport, mode);
        if let Ok(mut d) = POINTER_DRAWN.lock() {
            let now = std::mem::take(&mut d.0);
            d.1 = now;
            d.2 = Some(stage);
        }
        let ((x0, y0), (x1, y1)) =
            (stage.apply(movie.stage.x0, movie.stage.y0), stage.apply(movie.stage.x1, movie.stage.y1));
        out.stage = Some([x0, y0, x1, y1]);
        ssx_core::rider_showcase::begin_frame(viewport.0, viewport.1);
        if let Ok(mut f) = FRAME_SIZE.lock() {
            *f = Some((viewport.0.max(1) as f32, viewport.1.max(1) as f32));
        }
        RENDER_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        grow_begin_frame();
        gear_layout(lib, player, &stage);
        let frame_start = std::time::Instant::now();
        for &r in player.levels.values() {
            let inh = Inherited { xf: stage, cx: Cxform::IDENTITY, blend: BlendMode::Normal, mask: false, slice: None };
            self.instance(lib, player, r, inh, &mut out.commands);
        }
        dimmer_fade(&mut out.commands, frame_start);
        if LOADING_SCREEN.load(std::sync::atomic::Ordering::Relaxed) {
            // (the loading screen up (a run starting): none of the menus' overlays (the rider's view's frame on its
            // way back to the highlighted card, the cards' glows) drawn over it, and the frame starts afresh after)
            loading_overlays_off();
        } else {
            gear_frame(&mut out.commands);
            hover_glows(&mut out.commands);
        }
        // (the highlighted card as drawn this frame, for the mouse)
        if let Some(r) = CARD_NOW.lock().ok().and_then(|c| *c) {
            ssx_core::rider_showcase::found_card([r.x0, r.y0, r.x1, r.y1], true);
        }
        ssx_core::rider_showcase::end_frame();
        out
    }

    pub fn build_instance(
        &mut self,
        lib: &Library,
        player: &Player,
        id: InstanceId,
        xf: Mat2d,
        out: &mut Vec<Command>,
    ) {
        let inh = Inherited { xf, cx: Cxform::IDENTITY, blend: BlendMode::Normal, mask: false, slice: None };
        self.instance(lib, player, id, inh, out);
    }

    fn instance(&mut self, lib: &Library, player: &Player, id: InstanceId, inh: Inherited, out: &mut Vec<Command>) {
        let Some(inst) = player.get(id) else { return };
        if !inst.visible && !inh.mask {
            return;
        }
        if !inh.mask && (hidden_dock_tab(player, inst) || hidden_side_dimmer(lib, player, inst)) {
            return;
        }
        if !inh.mask && gear_hidden(id) {
            return;
        }
        // (the full list's grid (the 3D Rider Select mod): only the cells over the list's own width drawn, not those
        // scrolled off to the left (behind the rider's view))
        if !inh.mask
            && !GRID_MASKING.load(std::sync::atomic::Ordering::Relaxed)
            && let Some([x0, y0, x1, y1]) = full_grid_mask(id)
        {
            let mut mesh = Mesh::default();
            mesh.vertices.extend([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            let clip = DrawItem {
                mesh: Arc::new(mesh),
                paint: Paint::Solid([1.0, 1.0, 1.0, 1.0]),
                transform: Mat2d::IDENTITY,
                cxform: Cxform::IDENTITY,
                blend: BlendMode::Normal,
            };
            out.extend([Command::MaskBegin, Command::Draw(clip.clone()), Command::MaskEnd]);
            GRID_MASKING.store(true, std::sync::atomic::Ordering::Relaxed);
            self.instance(lib, player, id, inh, out);
            GRID_MASKING.store(false, std::sync::atomic::Ordering::Relaxed);
            out.extend([Command::MaskPopBegin, Command::Draw(clip), Command::MaskPopEnd]);
            return;
        }
        // (the special gear and character perks box: the rider step's only, gone as soon as a rider is chosen (the menu
        // keeps it up through the way to the gear steps))
        // (and on the gear steps (the full list's too) no button bar: its Y "Attribute Descriptions" on the suits' and
        // boards' lists; Y marks favourites there)
        if !inh.mask
            && (inst.name == "mcBioTop" || inst.name == "mcButtonCalloutBar")
            && ssx_core::rider_showcase::enabled()
            && ssx_core::rider_showcase::gear_view()
        {
            return;
        }
        // (back from the gear steps, the gear and perks box fading in with the list)
        if !inh.mask && inst.name == "mcBioTop" && ssx_core::rider_showcase::enabled() {
            let f = f32::from_bits(BIO_FADE.load(std::sync::atomic::Ordering::Relaxed));
            if f <= 0.001 {
                return;
            }
            if f < 0.999 && !BIO_FADING.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let mut faded = inh;
                faded.cx.mult[3] *= f;
                self.instance(lib, player, id, faded, out);
                BIO_FADING.store(false, std::sync::atomic::Ordering::Relaxed);
                return;
            }
        }
        // (the menus' full-screen dimmer: its draws kept, so when it goes (the equipment screen closing onto the
        // globe) it fades out rather than cutting from dark to light)
        if !inh.mask && !DIMMER_KEEPING.load(std::sync::atomic::Ordering::Relaxed) && full_dimmer(lib, player, inst) {
            let start = out.len();
            DIMMER_KEEPING.store(true, std::sync::atomic::Ordering::Relaxed);
            self.instance(lib, player, id, inh, out);
            DIMMER_KEEPING.store(false, std::sync::atomic::Ordering::Relaxed);
            if out.len() > start
                && let Ok(mut d) = DIMMER_LAST.lock()
            {
                *d = Some((out[start..].to_vec(), std::time::Instant::now(), None));
            }
            return;
        }
        // (on the rider step the menu's chosen-rider card, kept behind the list, never shows: on the way back from the
        // gear steps the menu brings it up for a moment)
        if !inh.mask
            && inst.name == "mcSelectedRider"
            && ssx_core::rider_showcase::enabled()
            && !ssx_core::rider_showcase::gear_view()
            && !ssx_core::rider_showcase::list_order().is_empty()
        {
            return;
        }
        // The rider showcase's highlighted card: drawn as a plain card (solid, a straight bottom a little below its
        // XP bar, a glowing green edge) in place of the menu's backing and highlight frames.
        // The rider list's other cards: their backing and highlight frames cut off straight a little below the XP bar
        // (in place of their pointed bottom), edged along the cut as along their sides.
        if !inh.mask
            && !PLAIN_CUT.load(std::sync::atomic::Ordering::Relaxed)
            && (inst.name == "mcBacker" || inst.character.as_ref().is_some_and(|c| c.id == 44))
            && !in_grown_cell(inst)
            && ssx_core::rider_showcase::enabled()
            && !ssx_core::rider_showcase::highlight_portrait().is_empty()
            && in_list_card(player, inst)
            && let Some((b, cut, bar)) = plain_card_cut(lib, player, id, inst)
        {
            let big = (b.x1 - b.x0).max(b.y1 - b.y0);
            let rect = |x0: f32, y0: f32, x1: f32, y1: f32| {
                let mut mesh = Mesh::default();
                mesh.vertices.extend([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
                mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                Arc::new(mesh)
            };
            let clip = DrawItem {
                mesh: rect(b.x0 - big, b.y0 - big, b.x1 + big, cut),
                paint: Paint::Solid([1.0, 1.0, 1.0, 1.0]),
                transform: inh.xf,
                cxform: Cxform::IDENTITY,
                blend: BlendMode::Normal,
            };
            out.extend([Command::MaskBegin, Command::Draw(clip.clone()), Command::MaskEnd]);
            PLAIN_CUT.store(true, std::sync::atomic::Ordering::Relaxed);
            self.instance(lib, player, id, inh, out);
            PLAIN_CUT.store(false, std::sync::atomic::Ordering::Relaxed);
            out.extend([Command::MaskPopBegin, Command::Draw(clip), Command::MaskPopEnd]);
            if inst.name == "mcBacker" && inst.visible {
                // (where it is for the mouse: the card's frame, down to the cut)
                // (its frame: as wide as its bottom edge's line (the bar and a little either side), its top the
                // backing's)
                let side = (bar.x1 - bar.x0) * 0.083;
                let w = RectF { x0: bar.x0 - side, y0: b.y0, x1: bar.x1 + side, y1: cut }.transform(&inh.xf);
                if inh.cx.mult[3] * inst.cxform.mult[3] > 0.02 {
                    ssx_core::rider_showcase::found_card([w.x0, w.y0, w.x1, w.y1], false);
                }
                let t = 2.0 / inh.xf.x_scale().max(1e-6);
                let a = (inh.cx.mult[3] * inst.cxform.mult[3]).clamp(0.0, 1.0);
                // (along the card's frame only, not its backing's clear margins: the cards overlap there, and the
                // edges would run on into one line under the whole list)
                let side = (bar.x1 - bar.x0) * 0.083;
                out.push(Command::Draw(DrawItem {
                    mesh: rect(bar.x0 - side, cut - t, bar.x1 + side, cut),
                    paint: Paint::Solid([PLAIN_EDGE[0], PLAIN_EDGE[1], PLAIN_EDGE[2], a]),
                    transform: inh.xf,
                    cxform: Cxform::IDENTITY,
                    blend: BlendMode::Normal,
                }));
            }
            return;
        }
        if !inh.mask && in_grown_cell(inst) {
            if inst.name == "mcBacker" {
                grown_card(&inh, out);
                return;
            }
            if inst.character.as_ref().is_some_and(|c| c.id == 44) {
                return;
            }
        }
        // The list's other cards never show the menu's green highlight frames: the menu highlights a card a moment
        // before the showcase moves it into the highlighted place (and fades the last one's out after), so it would
        // flash green on its way there.
        // (drawn in the plain cards' teal instead: a card the menu has highlighted, or is letting go of, shows its
        // highlight frames in place of its plain edge)
        let plain_edge = !inh.mask
            && inst.character.as_ref().is_some_and(|c| c.id == 44)
            && ssx_core::rider_showcase::enabled()
            && !ssx_core::rider_showcase::highlight_portrait().is_empty()
            && in_list_card(player, inst);
        // TODO(guess): an instance with normal (or layer) blending pushes nothing, so the innermost other mode of its ancestors applies; settle: which render-tree nodes push.
        let blend = if inh.mask {
            BlendMode::Normal
        } else if matches!(inst.blend, BlendMode::Normal | BlendMode::Layer) {
            inh.blend
        } else {
            inst.blend
        };
        let matrix = narrow_popup(player, inst).or_else(|| wide_leaderboard(lib, player, inst)).unwrap_or(inst.matrix);
        let matrix = if inh.mask { matrix } else { centred_event_title(lib, player, id, inst, matrix) };
        let matrix = if inh.mask { matrix } else { gear_moved(lib, player, id, &inh.xf, matrix) };
        let matrix = if inh.mask { matrix } else { grown_card_part(lib, player, id, inst, &inh.xf, matrix) };
        if !inh.mask && inst.name == "mcCell" {
            check_grow_cell(lib, player, id, inst, &inh.xf.mul(&matrix), &inh.cx.concat(&inst.cxform));
        }
        if !inh.mask {
            test_peak_path(player, id, inst, &inh, out);
        }
        let here = Inherited {
            xf: inh.xf.mul(&matrix),
            cx: {
                let mut cx = inh.cx.concat(&lit_look(player, id, inst).unwrap_or(inst.cxform));
                if plain_edge {
                    cx.mult[..3].copy_from_slice(&[0.0; 3]);
                    cx.add[..3].copy_from_slice(&PLAIN_EDGE);
                }
                cx
            },
            blend,
            mask: inh.mask,
            slice: None,
        };
        // (an entry the mouse asks about: where it's drawn, for it)
        if !inh.mask
            && let Ok(watch) = POINTER_WATCH.lock()
            && watch.contains(&id)
        {
            drop(watch);
            // (a box with its backing (Start Run): the backing only, as drawn (made as wide as an option))
            let backer = player.child_by_name(id, "mcBackerContainer", true).and_then(|b| {
                let bi = player.get(b)?;
                let m = gear_moved(lib, player, b, &here.xf, bi.matrix);
                let r = player.local_bounds(lib, b);
                (!r.is_empty()).then(|| r.transform(&here.xf.mul(&m)))
            });
            let r = backer.unwrap_or_else(|| player.local_bounds(lib, id).transform(&here.xf));
            if !r.is_empty()
                && let Ok(mut d) = POINTER_DRAWN.lock()
            {
                d.0.insert(id, r);
            }
        }
        if !inh.mask && inst.name == "mcInnerframe" && in_grown_cell(inst) {
            self.square_frame(lib, player, id, inst, &inh, &matrix, &here, out);
            return;
        }
        if !inh.mask
            && !ghosting()
            && !ssx_core::rider_showcase::slot_found()
            && in_list_card(player, inst)
            && (highlighted_rider_portrait(lib, player, id, inst, here.cx.mult[3])
                || (inst.name == "mcPortraitContainer"
                    && ssx_core::rider_showcase::enabled()
                    && inst.parent.is_some_and(|p| pictureless_highlight(player, p))))
        {
            let b = player.local_bounds(lib, id);
            if !b.is_empty() {
                let w = b.transform(&here.xf);
                ssx_core::rider_showcase::found_slot(grown_slot([w.x0, w.y0, w.x1, w.y1]));
            }
            if ssx_core::rider_showcase::portrait_hidden() {
                return;
            }
        }
        // (the level's X button under the mouse: its glow, its own shape drawn larger and fainter behind it)
        if !inh.mask
            && inst.name == "mcPrompt"
            && ssx_core::rider_showcase::enabled()
            && ssx_core::rider_showcase::level_offer()
            && !BUTTON_GLOWING.load(std::sync::atomic::Ordering::Relaxed)
            && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcBioPrompt")
        {
            let e = button_hover();
            let b = LEVEL_BUTTON.lock().ok().and_then(|b| *b);
            if let Some(b) = b.filter(|b| e > 0.0 && b.x1 - b.x0 > 2.0 && b.y1 - b.y0 > 2.0) {
                let pulse = card_pulse();
                // (its backing only (its largest part), not its icon and words: the glow is the backing's shape)
                let xf = inh.xf.mul(&matrix);
                let backing = match &inst.kind {
                    InstanceKind::Sprite(sp) => sp
                        .children
                        .values()
                        .copied()
                        .filter_map(|k| {
                            let c = player.get(k)?;
                            let bb = player.local_bounds(lib, k).transform(&xf.mul(&c.matrix));
                            (!bb.is_empty() && c.visible).then_some((k, bb, (bb.x1 - bb.x0) * (bb.y1 - bb.y0)))
                        })
                        .max_by(|a, b| a.2.total_cmp(&b.2)),
                    _ => None,
                };
                let (part, bb) = match backing {
                    Some((k, bb, _)) => (Some(k), bb),
                    None => (None, b),
                };
                let (cx, cy) = ((bb.x0 + bb.x1) * 0.5, (bb.y0 + bb.y1) * 0.5);
                let (w, h) = ((bb.x1 - bb.x0).max(1.0), (bb.y1 - bb.y0).max(1.0));
                let layers = 20;
                let reach = 4.0 + 8.0 * e;
                BUTTON_GLOWING.store(true, std::sync::atomic::Ordering::Relaxed);
                for i in (1..=layers).rev() {
                    let d = reach * i as f32 / layers as f32;
                    let f = 1.0 - (i as f32 - 0.5) / layers as f32;
                    let grow = Mat2d::translate(cx, cy)
                        .mul(&Mat2d::scale(1.0 + 2.0 * d / w, 1.0 + 2.0 * d / h))
                        .mul(&Mat2d::translate(-cx, -cy));
                    let a = 0.22 * f * f * e * pulse * inh.cx.mult[3].clamp(0.0, 1.0);
                    let tint = Cxform { mult: [0.0, 0.0, 0.0, a], add: [CARD_GLOW[0], CARD_GLOW[1], CARD_GLOW[2], 0.0] };
                    match part {
                        Some(k) => {
                            let glow = Inherited { xf: grow.mul(&xf), cx: tint, ..inh };
                            self.instance(lib, player, k, glow, out);
                        }
                        None => {
                            let glow = Inherited { xf: grow.mul(&inh.xf), cx: tint, ..inh };
                            self.instance(lib, player, id, glow, out);
                        }
                    }
                }
                BUTTON_GLOWING.store(false, std::sync::atomic::Ordering::Relaxed);
            }
        }
        match &inst.kind {
            InstanceKind::Sprite(s) => {
                // TODO(guess): the nine regions' mapping is Flash's model; settle: the consumer of the key data 0x82687D48 fills (kept at the mesh's +0x34, 0x82688BE8 → 0x82687250).
                let slice = match inst.scale9 {
                    Some(grid) => {
                        let (sx, sy, rest) = Slice9::split(&matrix);
                        let scaled = (sx - 1.0).abs() >= 1e-3 || (sy - 1.0).abs() >= 1e-3;
                        let bounds = player.local_bounds(lib, id);
                        let grid = widened_grid(inst, grid, bounds);
                        (scaled && !bounds.is_empty())
                            .then(|| (Slice9 { grid, bounds, sx, sy }, inh.xf.mul(&rest), Mat2d::IDENTITY))
                    }
                    None => inh.slice.map(|(sl, base, to_grid)| (sl, base, to_grid.mul(&matrix))),
                };
                let here = Inherited { slice, ..here };
                let mut masks: Vec<(i32, InstanceId)> = Vec::new();
                for (&depth, &child) in &s.children {
                    while masks.last().is_some_and(|m| depth > m.0) {
                        let (_, m) = masks.pop().expect("an open mask");
                        self.pop_mask(lib, player, m, here, out);
                    }
                    let Some(c) = player.get(child) else { continue };
                    if let Some(clip) = c.clip_depth.filter(|_| !here.mask) {
                        out.push(Command::MaskBegin);
                        self.instance(lib, player, child, Inherited { mask: true, ..here }, out);
                        out.push(Command::MaskEnd);
                        masks.push((clip, child));
                        continue;
                    }
                    // (the rider list's newly highlighted card, as it was, sliding in under itself)
                    let list_card = !here.mask
                        && inst.name == "mcRiderMenu"
                        && c.name.starts_with("clipInstance")
                        && ssx_core::rider_showcase::enabled();
                    if list_card {
                        let top = GHOST_TOP.lock().ok().and_then(|t| *t);
                        if top == Some(child) {
                            draw_card_copies(&here.xf, out);
                            self.instance(lib, player, child, here, out);
                        } else {
                            let start = out.len();
                            let found = ssx_core::rider_showcase::cards_found();
                            // (the card under the mouse: a little larger, edged in a blue glow)
                            let hover = hovered_card_px().filter(|h| {
                                let b = player.local_bounds(lib, child).transform(&here.xf.mul(&c.matrix));
                                let (bx, hx) = ((b.x0 + b.x1) * 0.5, (h.x0 + h.x1) * 0.5);
                                !b.is_empty() && (h.x0..=h.x1).contains(&bx) && (b.x0..=b.x1).contains(&hx)
                            });
                            match hover {
                                Some(h) => {
                                    let (cx, cy) = ((h.x0 + h.x1) * 0.5, (h.y0 + h.y1) * 0.5);
                                    let grow = Mat2d::translate(cx, cy)
                                        .mul(&Mat2d::scale(HOVER_GROW, HOVER_GROW))
                                        .mul(&Mat2d::translate(-cx, -cy));
                                    let bigger = Inherited { xf: grow.mul(&here.xf), ..here };
                                    let before = ssx_core::rider_showcase::cards_found();
                                    self.instance(lib, player, child, bigger, out);
                                    // (round its frame as just drawn (larger); else as last drawn, made larger)
                                    let frame = (ssx_core::rider_showcase::cards_found() > before)
                                        .then(ssx_core::rider_showcase::last_card_found)
                                        .flatten()
                                        .zip(FRAME_SIZE.lock().ok().and_then(|s| *s))
                                        .map(|(r, (w, hgt))| [r[0] * w, r[1] * hgt, r[2] * w, r[3] * hgt]);
                                    let (hw, hh) = ((h.x1 - h.x0) * HOVER_GROW * 0.5, (h.y1 - h.y0) * HOVER_GROW * 0.5);
                                    let alpha = (here.cx.mult[3] * c.cxform.mult[3]).clamp(0.0, 1.0);
                                    hover_glow(out, frame.unwrap_or([cx - hw, cy - hh, cx + hw, cy + hh]), alpha);
                                }
                                None => self.instance(lib, player, child, here, out),
                            }
                            // (whose card it is, for the mouse; and where, for its copies sliding on)
                            if ssx_core::rider_showcase::cards_found() > found
                                && let Some((rider, _)) =
                                    CARD_RIDERS.lock().ok().and_then(|m| m.as_ref().and_then(|h| h.get(&child).cloned()))
                            {
                                ssx_core::rider_showcase::name_last_card(&rider);
                                if c.visible
                                    && c.cxform.mult[3] >= 0.99
                                    && hover.is_none()
                                    && let (Some(r), Some((w, h))) = (
                                        ssx_core::rider_showcase::last_card_found(),
                                        FRAME_SIZE.lock().ok().and_then(|s| *s),
                                    )
                                    && let Ok(mut m) = CARD_RECTS.lock()
                                {
                                    m.insert(rider, RectF { x0: r[0] * w, y0: r[1] * h, x1: r[2] * w, y1: r[3] * h });
                                }
                            }
                            // (only as it's fully shown: not fading in at the list's end)
                            if c.visible && c.cxform.mult[3] >= 0.99 {
                                remember_card(lib, player, child, c, &here.xf, start, out);
                            }
                        }
                        continue;
                    }
                    // (a portrait of the rider list: drawn as last shown while the menu's loading it into another card)
                    if !here.mask
                        && c.name == "mcPortraitContainer"
                        && ssx_core::rider_showcase::enabled()
                        && !in_grown_cell(c)
                        && in_list_card(player, c)
                    {
                        let start = out.len();
                        let xf = here.xf.mul(&c.matrix);
                        self.instance(lib, player, child, here, out);
                        let alpha = (here.cx.mult[3] * c.cxform.mult[3]).clamp(0.0, 1.0);
                        keep_portrait(lib, player, child, &xf, alpha, start, out);
                        continue;
                    }
                    self.instance(lib, player, child, here, out);
                }
                while let Some((_, m)) = masks.pop() {
                    self.pop_mask(lib, player, m, here, out);
                }
                if !here.mask && player.hud_extra >= 0.5 && is_side_dimmer(lib, player, inst) {
                    self.side_dimmer_edges(lib, player, inst, &matrix, &here, out);
                }
                if !here.mask && inst.name == "mcItem" && ssx_core::rider_showcase::enabled() {
                    favourite_star(lib, player, id, &here, out);
                }
            }
            InstanceKind::Shape | InstanceKind::Morph | InstanceKind::Bitmap => {
                let Some(mesh) = self.character_mesh(lib, inst, &here.xf) else { return };
                let mesh =
                    if !inh.mask && in_narrow_popup_band(player, inst) { Arc::new(faded_band(&mesh)) } else { mesh };
                match &inh.slice {
                    Some((sl, base, to_grid)) => {
                        let sliced = Inherited { xf: *base, ..here };
                        let shape = to_grid.mul(&matrix);
                        let opaque = OPAQUE_CARD.with(|c| c.get());
                        for layer in &mesh.layers {
                            let m = Arc::new(sl.slice(&layer.mesh, &shape));
                            let item = DrawItem {
                                mesh: m,
                                paint: if opaque { opaque_paint(&layer.paint) } else { layer.paint.clone() },
                                transform: *base,
                                cxform: if opaque { opaque_cx(sliced.cx, &layer.paint) } else { sliced.cx },
                                blend: sliced.blend,
                            };
                            out.push(Command::Draw(item));
                        }
                    }
                    None if OPAQUE_CARD.with(|c| c.get()) => {
                        for l in &mesh.layers {
                            out.push(Command::Draw(DrawItem {
                                mesh: l.mesh.clone(),
                                paint: opaque_paint(&l.paint),
                                transform: here.xf,
                                cxform: opaque_cx(here.cx, &l.paint),
                                blend: here.blend,
                            }));
                        }
                    }
                    None => emit(out, &mesh, &here),
                }
            }
            InstanceKind::EditText(t) => {
                let param = inst.filter.filter(|_| !here.mask && !self.no_filters).map(|f| TextFieldParam::new(&f));
                // TODO(guess): the line buffer's flags 0x04 / 0x20 (+0x34, they turn these tests off) are taken as clear; settle: their setters (not among the writers of +0x34 / the view's +0x64).
                let g = crate::text::GUTTER;
                let tr = RectF { x0: t.bounds.x0 + g, y0: t.bounds.y0 + g, x1: t.bounds.x1 - g, y1: t.bounds.y1 - g };
                let limit = tr.height() + 20.0;
                let boxes = &t.layout.line_boxes;
                let visible = boxes
                    .iter()
                    .enumerate()
                    .take_while(|&(i, &(top, h))| if i == 0 { top <= limit } else { top + h <= limit })
                    .count();
                let mut clip = boxes.first().is_some_and(|&(top, h)| h > 0.0 && top <= limit && limit < top + h);
                let mut ended = u32::MAX;
                let mut shown = Vec::with_capacity(t.layout.glyphs.len());
                for gl in &t.layout.glyphs {
                    if gl.line as usize >= visible || gl.line == ended {
                        continue;
                    }
                    let right = gl.x + gl.ink_right;
                    if right <= tr.x0 {
                        continue;
                    }
                    if gl.x >= tr.x1 {
                        ended = gl.line;
                        continue;
                    }
                    clip |= (gl.x < tr.x0 && tr.x0 < right) || (gl.x < tr.x1 && tr.x1 < right);
                    shown.push(gl);
                }
                let clip = clip && !here.mask;
                let b = tr;
                let rect = clip.then(|| {
                    let mesh = Mesh {
                        vertices: vec![[b.x0, b.y0], [b.x1, b.y0], [b.x1, b.y1], [b.x0, b.y1]],
                        indices: vec![0, 1, 2, 0, 2, 3],
                        paint: Vec::new(),
                    };
                    DrawItem {
                        mesh: Arc::new(mesh),
                        paint: Paint::Solid([1.0; 4]),
                        transform: here.xf,
                        cxform: here.cx,
                        blend: BlendMode::Normal,
                    }
                });
                if let Some(r) = &rect {
                    out.extend([Command::MaskBegin, Command::Draw(r.clone()), Command::MaskEnd]);
                }
                let mut layers = TextLayers::default();
                for g in shown {
                    let size = g.scale * crate::text::em_units(&g.font);
                    let glyph = TextGlyph { font: &g.font, index: g.index, size, at: (g.x, g.y), color: g.color };
                    self.text_glyph(&glyph, &here.xf, &here, param.as_ref(), &mut layers);
                }
                layers.emit(out);
                if let Some(r) = rect {
                    out.extend([Command::MaskPopBegin, Command::Draw(r), Command::MaskPopEnd]);
                }
            }
            InstanceKind::StaticText => self.static_text(lib, inst, here, out),
        }
    }

    fn pop_mask(&mut self, lib: &Library, player: &Player, mask: InstanceId, here: Inherited, out: &mut Vec<Command>) {
        out.push(Command::MaskPopBegin);
        self.instance(lib, player, mask, Inherited { mask: true, ..here }, out);
        out.push(Command::MaskPopEnd);
    }
}

fn emit(out: &mut Vec<Command>, mesh: &ShapeMesh, here: &Inherited) {
    for layer in &mesh.layers {
        out.push(Command::Draw(DrawItem {
            mesh: layer.mesh.clone(),
            paint: layer.paint.clone(),
            transform: here.xf,
            cxform: here.cx,
            blend: here.blend,
        }));
    }
}

impl Renderer {
    fn tessellated(
        &mut self,
        key: MeshKey,
        what: impl FnOnce() -> String,
        make: impl FnOnce(&mut Budget) -> Result<ShapeMesh, OverBudget>,
    ) -> Option<Arc<ShapeMesh>> {
        if let Some(c) = self.meshes.get_mut(&key) {
            c.frame = self.frame;
            return Some(c.mesh.clone());
        }
        if self.frame_work.reached(&FRAME_TESSELLATION) {
            self.tess_stats.deferred += 1;
            return None;
        }
        let mut budget = Budget::shape();
        let made = make(&mut budget);
        self.frame_work.add(budget.used());
        self.tess_stats.shape_peak.peak(budget.used());
        self.tessellated += 1;
        let mesh = match made {
            Ok(m) => m,
            Err(e) => {
                self.tess_stats.refused += 1;
                if self.refused.len() < REFUSED_KEPT {
                    self.refused.push(format!("{}: {e}", what()));
                }
                ShapeMesh::default()
            }
        };
        let vertices: u64 = mesh.layers.iter().map(|l| l.mesh.vertices.len() as u64).sum();
        if self.cached_vertices + vertices > CACHE_VERTICES {
            let (frame, before) = (self.frame, self.meshes.len());
            self.meshes.retain(|_, c| c.frame == frame);
            self.tess_stats.evicted += (before - self.meshes.len()) as u64;
            self.cached_vertices = self.meshes.values().map(|c| c.vertices).sum();
        }
        let mesh = Arc::new(mesh);
        if self.cached_vertices + vertices <= CACHE_VERTICES {
            self.meshes.insert(key, Cached { mesh: mesh.clone(), vertices, frame: self.frame, owner: None });
            self.cached_vertices += vertices;
            self.tess_stats.cached_vertices_peak = self.tess_stats.cached_vertices_peak.max(self.cached_vertices);
        }
        Some(mesh)
    }

    /// The rider showcase's grown card: its inner frame (a notched outline, the notch for the level number, which
    /// now sits below it) drawn as a plain rectangle, in the frame's own colour.
    #[allow(clippy::too_many_arguments)]
    fn square_frame(
        &mut self,
        lib: &Library,
        player: &Player,
        id: InstanceId,
        inst: &Instance,
        inh: &Inherited,
        matrix: &Mat2d,
        here: &Inherited,
        out: &mut Vec<Command>,
    ) {
        let b = player.local_bounds(lib, id);
        if b.is_empty() {
            return;
        }
        let mut r = b.transform(matrix);
        // (its top lowered: more room for the rider's name over it; centred on the card)
        r.y0 += (r.y1 - r.y0) * INNER_TOP_DROP - INNER_TOP_RAISE_PX / inh.xf.x_scale().max(1e-6);
        if let Some(g) = GROW.lock().ok().and_then(|g| *g) {
            let dx = inner_shift(g);
            r.x0 += dx;
            r.x1 += dx;
        }
        let t = 3.0 / inh.xf.x_scale().max(1e-6);
        let mut paint = None;
        let mut cx = here.cx;
        if let InstanceKind::Sprite(s) = &inst.kind {
            for &c in s.children.values() {
                let Some(child) = player.get(c) else { continue };
                if let Some(m) = self.character_mesh(lib, child, &here.xf.mul(&child.matrix)) {
                    paint = m.layers.iter().find_map(|l| match &l.paint {
                        Paint::Solid(c) => Some(*c),
                        _ => None,
                    });
                }
                if paint.is_some() {
                    cx = here.cx.concat(&child.cxform);
                    break;
                }
            }
        }
        let colour = paint.unwrap_or([0.25, 0.85, 0.6, 1.0]);
        // How bright the line is drawn now (the menu pulses it): the card's glow pulses with it.
        let shown = cx.apply(colour);
        let bright = shown[3] * shown[0].max(shown[1]).max(shown[2]);
        if let Ok(mut p) = INNER_PULSE.lock() {
            p.1 = p.1.max(bright) * 0.999 + bright * 0.001;
            p.0 = if p.1 > 1e-3 { (bright / p.1).clamp(0.0, 1.0) } else { 1.0 };
        }
        // (the Custom Event Menus mod: the line pulses on a steady beat of its own (the menu's own pulse doesn't run
        // while the card's kept lit), the card's glow with it)
        if ssx_core::rider_showcase::enabled() {
            let pulse = card_pulse();
            if let Ok(mut p) = INNER_PULSE.lock() {
                p.0 = pulse;
            }
            cx = here.cx;
            cx.mult[3] *= pulse;
        }
        // (where it is (window pixels) and its colour: the rider's view's frame on the gear steps grows from it)
        if let Ok(mut last) = INNER_LAST.lock() {
            let w = r.transform(&inh.xf);
            *last = Some(([w.x0, w.y0, w.x1, w.y1], colour));
        }
        // (the rider's view's frame shrinking back into the card: the card's own one shows once it's there)
        if frame_returning() {
            return;
        }
        let mut mesh = Mesh::default();
        for [x0, y0, x1, y1] in [
            [r.x0, r.y0, r.x1, r.y0 + t],
            [r.x0, r.y1 - t, r.x1, r.y1],
            [r.x0, r.y0, r.x0 + t, r.y1],
            [r.x1 - t, r.y0, r.x1, r.y1],
        ] {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid(colour),
            transform: inh.xf,
            cxform: cx,
            blend: here.blend,
        }));
    }

    fn character_mesh(&mut self, lib: &Library, inst: &Instance, xf: &Mat2d) -> Option<Arc<ShapeMesh>> {
        let r = inst.character?;
        let bucket = scale_bucket(xf.max_scale());
        let key = match inst.kind {
            InstanceKind::Morph => MeshKey::Morph(r, (inst.ratio * 65535.0).round() as u16, bucket),
            InstanceKind::Bitmap => MeshKey::Bitmap(r),
            _ => MeshKey::Shape(r, bucket),
        };
        let tol = bucket_tolerance(bucket);
        let what = || format!("{}: character {} at scale 2^{bucket}", lib.movie(r.movie).path, r.id);
        match lib.resolve(r)?.1 {
            Character::Shape(s) => self.tessellated(key, what, |b| {
                shape::build(lib, r.movie, &s.shape.styles, &s.shape.records, s.uses_fill_winding_rule, &tol, b)
            }),
            Character::Morph(ms) => self.tessellated(key, what, |b| {
                let (styles, records) = shape::morph_at(ms, inst.ratio);
                shape::build(lib, r.movie, &styles, &records, false, &tol, b)
            }),
            Character::Bitmap(b) => self.tessellated(key, what, |_| Ok(bitmap_quad(*b))),
            _ => None,
        }
    }

    fn glyph(&mut self, font: &Arc<Font>, index: usize, scale: f32) -> Arc<ShapeMesh> {
        let bucket = scale_bucket(scale);
        let key = MeshKey::Glyph(Arc::as_ptr(font) as usize, index, bucket);
        let tol = bucket_tolerance(bucket);
        let what = || format!("font {:?}: glyph {index} at scale 2^{bucket}", font.name);
        let mesh = self.tessellated(key, what, |b| {
            let mesh = match font.glyphs.get(index) {
                Some(g) => shape::glyph_mesh(&g.records, tol.curve, b)?,
                None => Mesh::default(),
            };
            let layers = if mesh.is_empty() {
                Vec::new()
            } else {
                vec![shape::ShapeLayer { paint: Paint::Solid([1.0; 4]), mesh: Arc::new(mesh) }]
            };
            Ok(ShapeMesh { layers })
        });
        if let Some(c) = self.meshes.get_mut(&key) {
            c.owner.get_or_insert_with(|| Arc::downgrade(font));
        }
        mesh.unwrap_or_default()
    }

    // TODO(guess): for a zero-glyph font the font library's font of that name stands in, index for index (never reached on the disc); settle: Scaleform's static text with a substituted font.
    fn static_text(&mut self, lib: &Library, inst: &Instance, here: Inherited, out: &mut Vec<Command>) {
        let Some(r) = inst.character else { return };
        let Some((_, Character::Text(t))) = lib.resolve(r) else { return };
        let param = inst.filter.filter(|_| !here.mask && !self.no_filters).map(|f| TextFieldParam::new(&f));
        let mut layers = TextLayers::default();
        let base = here.xf.mul(&Mat2d::from_swf(&t.matrix));
        let (mut font, mut color, mut height) = (None, [0.0, 0.0, 0.0, 1.0], 240.0);
        let (mut x, mut y) = (0.0f32, 0.0f32);
        for rec in &t.records {
            if let Some(fid) = rec.font_id {
                font = lib.font_for(r.movie, Some(fid), None);
            }
            if let Some(c) = rec.color {
                color = crate::geom::rgba(c);
            }
            if let Some(h) = rec.height {
                height = f32::from(h);
            }
            if let Some(dx) = rec.x_offset {
                x = f32::from(dx);
            }
            if let Some(dy) = rec.y_offset {
                y = f32::from(dy);
            }
            let Some(f) = font.clone() else { continue };
            for &(g, adv) in &rec.glyphs {
                let glyph = TextGlyph { font: &f, index: g as usize, size: height, at: (x, y), color };
                self.text_glyph(&glyph, &base, &here, param.as_ref(), &mut layers);
                x += adv as f32;
            }
        }
        layers.emit(out);
    }

    fn text_glyph(
        &mut self,
        g: &TextGlyph,
        node: &Mat2d,
        here: &Inherited,
        param: Option<&TextFieldParam>,
        layers: &mut TextLayers,
    ) {
        use crate::glyph::{CachedGlyph, height_ratio, snapped_size};
        let ratio = height_ratio(node);
        let screen = g.size * ratio;
        let shadow = param.and_then(|p| p.shadow);
        if let Some(sh) = shadow.filter(ShadowParam::drawn) {
            let ss = shadow_size(screen);
            if let CachedGlyph::Raster(r) = self.glyphs.filtered(g.font, g.index, ss, sh.filter, ratio, screen) {
                let pen = node.mul(&Mat2d::translate(g.at.0 + sh.offset.0, g.at.1 + sh.offset.1));
                layers.shadow.push(raster_draw(&r, ss, g.size, &pen, sh.color, here));
            }
        }
        if shadow.is_some_and(|s| s.filter.flags & GlyphFilter::KNOCKOUT != 0) {
            return;
        }
        let fs = snapped_size(screen);
        let pen = node.mul(&Mat2d::translate(g.at.0, g.at.1));
        let cached = match param.map(|p| p.text).filter(GlyphFilter::blurred) {
            Some(f) => self.glyphs.filtered(g.font, g.index, fs, f, ratio, screen),
            None => self.glyphs.glyph(g.font, g.index, fs),
        };
        match cached {
            CachedGlyph::Raster(r) => layers.raster.push(raster_draw(&r, fs, g.size, &pen, g.color, here)),
            CachedGlyph::Empty => {}
            CachedGlyph::Vector => {
                let k = g.size / crate::text::em_units(g.font);
                let xf = pen.mul(&Mat2d::scale(k, k));
                let mesh = self.glyph(g.font, g.index, xf.max_scale());
                for layer in &mesh.layers {
                    layers.vector.push(Command::Draw(DrawItem {
                        mesh: layer.mesh.clone(),
                        paint: Paint::Solid(g.color),
                        transform: xf,
                        cxform: here.cx,
                        blend: here.blend,
                    }));
                }
            }
        }
    }

    pub fn cached(&self) -> usize {
        self.meshes.len()
    }

    pub fn cached_vertices(&self) -> u64 {
        self.cached_vertices
    }

    pub fn tessellate_all(&mut self, lib: &Library, movie: crate::library::MovieId, scale: f32) -> (usize, usize) {
        let tol = Tolerance::for_scale(scale);
        let (mut n, mut tris) = (0, 0);
        let m = lib.movie(movie);
        let mut ids: Vec<u16> = m.chars.keys().copied().collect();
        ids.sort();
        for id in ids {
            let mut budget;
            let mesh = match &m.chars[&id] {
                Character::Shape(s) => {
                    budget = Budget::shape();
                    shape::build(
                        lib,
                        movie,
                        &s.shape.styles,
                        &s.shape.records,
                        s.uses_fill_winding_rule,
                        &tol,
                        &mut budget,
                    )
                }
                Character::Morph(ms) => {
                    let (a, ra) = shape::morph_at(ms, 0.0);
                    let (b, rb) = shape::morph_at(ms, 1.0);
                    budget = Budget::shape();
                    shape::build(lib, movie, &a, &ra, false, &tol, &mut budget).and_then(|mut mesh| {
                        budget = Budget::shape();
                        mesh.layers.extend(shape::build(lib, movie, &b, &rb, false, &tol, &mut budget)?.layers);
                        Ok(mesh)
                    })
                }
                _ => continue,
            };
            self.tess_stats.shape_peak.peak(budget.used());
            let Ok(mesh) = mesh else {
                self.tess_stats.refused += 1;
                continue;
            };
            n += 1;
            tris += mesh.triangle_count();
        }
        (n, tris)
    }
}

struct TextGlyph<'a> {
    font: &'a Arc<Font>,
    index: usize,
    size: f32,
    at: (f32, f32),
    color: [f32; 4],
}

fn raster_draw(
    r: &crate::glyph::RasterGlyph,
    class: u16,
    size: f32,
    pen: &Mat2d,
    color: [f32; 4],
    here: &Inherited,
) -> Command {
    let k = size / (f32::from(class) * 0.0625).max(1e-5) * r.scale;
    Command::Draw(DrawItem {
        mesh: r.quad.clone(),
        paint: Paint::Glyph { raster: r.raster.clone(), rect: r.rect, color },
        transform: pen.mul(&Mat2d::scale(k, k)),
        cxform: here.cx,
        blend: here.blend,
    })
}

fn bitmap_quad(b: crate::library::BitmapDef) -> ShapeMesh {
    let (w, h) = (b.size[0] as f32 * 20.0, b.size[1] as f32 * 20.0);
    let mesh = Mesh {
        vertices: vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]],
        indices: vec![0, 1, 2, 0, 2, 3],
        paint: Vec::new(),
    };
    let paint = Paint::Bitmap {
        bitmap: b,
        matrix: Mat2d::scale(20.0, 20.0).mul(&b.texel_to_pixel()),
        repeat: false,
        smooth: true,
    };
    ShapeMesh { layers: vec![shape::ShapeLayer { paint, mesh: Arc::new(mesh) }] }
}

const LEADERBOARD_MOVIES: [&str; 2] = ["components/hud/leaderboard.gfx", "components/hud/dynamicleaderboard.gfx"];

// PORT(choice): for windows wider than 16:9 — the standings components place themselves outside `RepositionIfSD`, so
// their topmost clip is shifted right by the window's extra stage width when drawn, as the HUD's `RepositionIfSD` side
// panels are moved out; the console's displays are 16:9 or 4:3.
fn is_leaderboard_movie(lib: &Library, inst: &Instance) -> bool {
    LEADERBOARD_MOVIES.contains(&lib.movie(inst.movie).path.as_str())
}

fn parent_to_root(player: &Player, inst: &Instance) -> Option<Mat2d> {
    let mut up = Mat2d::IDENTITY;
    let mut cur = inst.parent;
    let mut steps = 0;
    while let Some(c) = cur {
        let i = player.get(c)?;
        up = i.matrix.mul(&up);
        cur = i.parent;
        steps += 1;
        if steps > 64 {
            return None;
        }
    }
    Some(up)
}

fn wide_leaderboard(lib: &Library, player: &Player, inst: &Instance) -> Option<Mat2d> {
    let extra = player.hud_extra as f32;
    if extra < 0.5 {
        return None;
    }
    if inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.movie == inst.movie) {
        return None;
    }
    if !is_leaderboard_movie(lib, inst) {
        return None;
    }
    let mut cur = inst.parent;
    while let Some(c) = cur {
        let i = player.get(c)?;
        if is_leaderboard_movie(lib, i) {
            return None;
        }
        cur = i.parent;
    }
    let up = parent_to_root(player, inst)?;
    let det = up.determinant();
    if det.abs() < 1e-9 {
        return None;
    }
    let dx = extra * 20.0;
    let m = inst.matrix;
    Some(Mat2d { tx: m.tx + up.d * dx / det, ty: m.ty - up.b * dx / det, ..m })
}

fn is_side_dimmer(lib: &Library, player: &Player, inst: &Instance) -> bool {
    inst.character.as_ref().is_some_and(|c| c.id == 2)
        && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcDimmer")
        && lib.movie(inst.movie).path.to_ascii_lowercase().rsplit(['/', '\\']).next() == Some("dimmer.gfx")
}

impl Renderer {
    // PORT(choice): for windows wider than 16:9 — where the side dimmer's gradient reaches the stage's left or right
    // edge, its padded end colour is drawn on out to the window's edge (the same gradient paint over a quad beyond the
    // shape's bounds), so the dimming does not stop in a hard edge beside the 16:9 stage; the stage's own pixels are
    // unchanged; the console's displays are 16:9 or 4:3.
    fn side_dimmer_edges(
        &mut self,
        lib: &Library,
        player: &Player,
        inst: &Instance,
        matrix: &Mat2d,
        here: &Inherited,
        out: &mut Vec<Command>,
    ) {
        const STAGE_WIDTH_TWIPS: f32 = 1280.0 * 20.0;
        let Some(up) = parent_to_root(player, inst) else { return };
        let to_root = up.mul(matrix);
        let window = (-player.hud_extra as f32 * 20.0, STAGE_WIDTH_TWIPS + player.hud_extra as f32 * 20.0);
        let InstanceKind::Sprite(s) = &inst.kind else { return };
        for &child in s.children.values() {
            let Some(c) = player.get(child) else { continue };
            if !c.visible || !matches!(c.kind, InstanceKind::Shape) {
                continue;
            }
            let full = to_root.mul(&c.matrix);
            if full.b.abs() > 1e-6 || full.c.abs() > 1e-6 || full.a.abs() <= 1e-6 {
                continue;
            }
            let b = player.local_bounds(lib, child);
            if b.is_empty() {
                continue;
            }
            let xf = here.xf.mul(&c.matrix);
            let Some(mesh) = self.character_mesh(lib, c, &xf) else { continue };
            let (sx0, sx1) = (full.a * b.x0 + full.tx, full.a * b.x1 + full.tx);
            let (left, right) = (sx0.min(sx1), sx0.max(sx1));
            let to_local = |x: f32| (x - full.tx) / full.a;
            let span = |a: f32, b: f32| (to_local(a).min(to_local(b)), to_local(a).max(to_local(b)));
            let mut spans = Vec::new();
            if left <= 0.5 && right > 0.5 && window.0 < left {
                spans.push(span(window.0, left));
            }
            if right >= STAGE_WIDTH_TWIPS - 0.5 && left < STAGE_WIDTH_TWIPS - 0.5 && window.1 > right {
                spans.push(span(right, window.1));
            }
            let blend = if matches!(c.blend, BlendMode::Normal | BlendMode::Layer) { here.blend } else { c.blend };
            for layer in &mesh.layers {
                let shape::Paint::Gradient(g) = &layer.paint else { continue };
                if g.spread != 0 {
                    continue;
                }
                for &(x0, x1) in &spans {
                    let quad = Mesh {
                        vertices: vec![[x0, b.y0], [x1, b.y0], [x1, b.y1], [x0, b.y1]],
                        indices: vec![0, 1, 2, 0, 2, 3],
                        paint: Vec::new(),
                    };
                    out.push(Command::Draw(DrawItem {
                        mesh: Arc::new(quad),
                        paint: layer.paint.clone(),
                        transform: xf,
                        cxform: here.cx.concat(&c.cxform),
                        blend,
                    }));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::tests::{empty_lib, movie, po};
    use crate::library::FrameTag;

    #[test]
    fn a_padded_image_maps_the_whole_file_onto_its_target() {
        let b = crate::library::BitmapDef { image: 0, rect: [0, 0, 248, 464], size: [248, 461] };
        let mesh = bitmap_quad(b);
        let l = &mesh.layers[0];
        assert_eq!(l.mesh.vertices[2], [248.0 * 20.0, 461.0 * 20.0]);
        let Paint::Bitmap { matrix, .. } = &l.paint else { panic!("a bitmap paint") };
        let (u, v) = matrix.inverse().expect("invertible").apply(248.0 * 20.0, 461.0 * 20.0);
        assert!((u - 248.0).abs() < 1e-3 && (v - 464.0).abs() < 1e-3, "{u} {v}");
        assert_eq!(crate::library::BitmapDef::of_rect(0, [2, 3, 12, 23]).size, [10, 20]);
    }

    #[test]
    fn the_popup_bands_ends_fade_out() {
        let g = shape::GradientPaint {
            kind: shape::GradientKind::Linear,
            matrix: Mat2d::IDENTITY,
            spread: 0,
            interpolation: 0,
            stops: vec![(0.0, [0.0, 0.1, 0.1, 1.0]), (0.5, [0.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.1, 0.1, 0.65])],
        };
        let f = fade_ends(&g);
        assert_eq!((f.sample(0.0)[3], f.sample(1.0)[3]), (0.0, 0.0), "nothing at the ends");
        assert_eq!(f.sample(0.5), g.sample(0.5), "the middle as it was");
        assert!(f.sample(0.05)[3] < f.sample(0.15)[3] && f.sample(0.15)[3] < 1.0, "easing in");
    }

    #[test]
    fn clip_layers_bracket_their_depth_range() {
        let mut lib = empty_lib();
        let mut mask = po(1, Some(1), false, 0);
        mask.clip_depth = Some(3);
        mask.flags |= 0x40;
        let m = movie(
            &mut lib,
            vec![vec![
                FrameTag::Place(Arc::new(mask)),
                FrameTag::Place(Arc::new(po(2, Some(2), false, 0))),
                FrameTag::Place(Arc::new(po(5, Some(2), false, 0))),
            ]],
        );
        let mut player = Player::new();
        player.load_level(&lib, m, 0);
        let list = Renderer::new().build(&lib, &player, (1280, 720), ScaleMode::NoBorder);
        let kinds: Vec<&str> = list
            .commands
            .iter()
            .map(|c| match c {
                Command::Draw(_) => "draw",
                Command::MaskBegin => "begin",
                Command::MaskEnd => "end",
                Command::MaskPopBegin => "pop",
                Command::MaskPopEnd => "popend",
            })
            .collect();
        assert_eq!(kinds, ["begin", "draw", "end", "draw", "pop", "draw", "popend", "draw"]);
    }

    #[test]
    fn the_side_dimmer_reaches_the_window_edge_only_in_a_wider_window() {
        use crate::library::{FrameDef, MovieDef, TimelineDef};
        use ssx_formats::gfx::shape::{DefineShape, FillStyle, Gradient, Shape, ShapeRecord, Styles};
        use ssx_formats::gfx::types::{Matrix, Rect, Rgba};
        let (w, h) = (436 * 20, 728 * 20);
        let gradient = FillStyle::Gradient {
            kind: 0x10,
            matrix: Matrix { a: -0.2661, d: -0.4443, tx: 218 * 20, ..Matrix::IDENTITY },
            gradient: Gradient {
                spread: 0,
                interpolation: 0,
                records: vec![(29, Rgba { r: 0x4E, g: 0x9F, b: 0xB1, a: 0 }), (214, Rgba { r: 0, g: 0, b: 0, a: 255 })],
                focal: None,
            },
        };
        let records = vec![
            ShapeRecord::StyleChange {
                move_to: Some((0, 0)),
                fill0: None,
                fill1: Some(1),
                line: None,
                new_styles: None,
            },
            ShapeRecord::Straight { dx: w, dy: 0 },
            ShapeRecord::Straight { dx: 0, dy: h },
            ShapeRecord::Straight { dx: -w, dy: 0 },
            ShapeRecord::Straight { dx: 0, dy: -h },
        ];
        let shape = DefineShape {
            version: 3,
            id: 1,
            bounds: Rect { x_min: 0, x_max: w, y_min: 0, y_max: h },
            edge_bounds: None,
            unk_reserved: 0,
            uses_fill_winding_rule: false,
            uses_non_scaling_strokes: false,
            uses_scaling_strokes: false,
            shape: Shape { styles: Styles { fills: vec![gradient], lines: Vec::new() }, records },
        };
        let build = |tx: i32, extra: f64| {
            let mut lib = empty_lib();
            let id = lib.movies().len();
            let frame = |tags: Vec<FrameTag>| vec![FrameDef { tags, ..Default::default() }];
            let sprite =
                |sid: u16, tags| Character::Sprite(Arc::new(TimelineDef { movie: id, id: sid, frames: frame(tags) }));
            let mut chars = HashMap::new();
            chars.insert(1, Character::Shape(Arc::new(shape.clone())));
            chars.insert(2, sprite(2, vec![FrameTag::Place(Arc::new(po(1, Some(1), false, 0)))]));
            chars.insert(7, sprite(7, vec![FrameTag::Place(Arc::new(po(1, Some(2), false, tx)))]));
            let named =
                ssx_formats::gfx::place::PlaceObject { name: Some("mcDimmer".into()), ..po(1, Some(7), false, 0) };
            let m = lib.insert_movie(MovieDef {
                path: "views/dimmer.gfx".into(),
                dir: String::new(),
                version: 8,
                frame_rate: 30.0,
                stage: RectF::new(0.0, 0.0, 25600.0, 14400.0),
                background: None,
                swf_name: "Dimmer".into(),
                root: Arc::new(TimelineDef { movie: id, id: 0, frames: frame(vec![FrameTag::Place(Arc::new(named))]) }),
                chars,
                exports: HashMap::new(),
                imports: HashMap::new(),
                scaling_grids: HashMap::new(),
            });
            let mut player = Player::new();
            player.load_level(&lib, m, 0);
            player.hud_extra = extra;
            let list = Renderer::new().build(&lib, &player, (1280, 720), ScaleMode::NoBorder);
            list.commands
                .iter()
                .filter_map(|c| match c {
                    Command::Draw(d) => Some(
                        d.mesh
                            .vertices
                            .iter()
                            .map(|v| v[0])
                            .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x))),
                    ),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(build(0, 0.0).len(), 1, "16:9: the shape alone");
        let wide = build(0, 100.0);
        assert_eq!(wide.len(), 2, "wider: the shape and its left end colour");
        assert_eq!(wide[1], (-2000.0, 0.0), "from the window's edge (100 px beside the stage) to the shape");
        assert_eq!(
            build(-50 * 20, 100.0)[1],
            (-1000.0, 0.0),
            "slid 50 px out: from the window's edge to its left side"
        );
        assert_eq!(build(-300 * 20, 100.0).len(), 1, "slid past the window's edge: it covers the extra width itself");
        assert_eq!(build(-500 * 20, 100.0).len(), 1, "wholly beside the stage: unchanged");
    }
}

/// The test peak's path (the Hidden Maps mod): its drop screen's markers, top to bottom, then on down the mountain, in
/// the drop screen's places; set by the menus each frame, `None` off that screen.
static TEST_PEAK_PATH: std::sync::Mutex<Option<Vec<(f32, f32)>>> = std::sync::Mutex::new(None);
/// The frames built (the test peak's path is drawn once in each).
static RENDER_FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// The frame the path was last drawn in (once a frame).
static TEST_PEAK_PATH_DRAWN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);

/// The test peak's markers as the screen draws them: each one's place in the drop screen's places, where it's drawn on
/// the stage, its clip, and whether it's the chosen one (drawn bigger, with its pin).
#[allow(clippy::type_complexity)]
static TEST_PEAK_MARKERS: std::sync::Mutex<Vec<((f32, f32), (f32, f32), InstanceId, bool)>> =
    std::sync::Mutex::new(Vec::new());

#[allow(clippy::type_complexity)]
pub fn set_test_peak_markers(markers: Vec<((f32, f32), (f32, f32), InstanceId, bool)>) {
    if let Ok(mut m) = TEST_PEAK_MARKERS.lock() {
        *m = markers;
    }
}

pub fn set_test_peak_path(path: Option<Vec<(f32, f32)>>) {
    if let Ok(mut p) = TEST_PEAK_PATH.lock() {
        *p = path;
    }
}

/// A port mod (Hidden Maps): the test peak's drop screen's path, a swerving glowing line through its markers on down the
/// mountain to where the snow ends, pulsing as the game's peaks' drop lines do. It's drawn once a frame, as the first of
/// the markers is (under them all).
fn test_peak_path(player: &Player, id: InstanceId, inst: &Instance, inh: &Inherited, out: &mut Vec<Command>) {
    let Ok(guard) = TEST_PEAK_PATH.lock() else { return };
    let Some(points) = guard.as_ref() else { return };
    let Ok(markers) = TEST_PEAK_MARKERS.lock() else { return };
    if !markers.iter().any(|&(_, _, m, _)| m == id) {
        return;
    }
    let frame = RENDER_FRAME.load(std::sync::atomic::Ordering::Relaxed);
    if TEST_PEAK_PATH_DRAWN.swap(frame, std::sync::atomic::Ordering::Relaxed) == frame {
        return;
    }
    // (the screen puts its markers in a space of its own, scaled and moved from the drop screen's places: that's worked
    // out from the markers beside this one (each axis a scale and a shift, fitted), so the line, made in the drop
    // screen's places, is drawn through them)
    let pairs: Vec<((f32, f32), (f32, f32))> =
        markers.iter().filter(|m| !m.3 || markers.len() < 3).map(|&(p, at, _, _)| (p, at)).collect();
    let fit = |f: fn(&((f32, f32), (f32, f32))) -> (f32, f32)| {
        let n = pairs.len().max(1) as f32;
        let (sx, sy) = pairs.iter().map(f).fold((0.0, 0.0), |a, (x, y)| (a.0 + x, a.1 + y));
        let (mx, my) = (sx / n, sy / n);
        let (sxy, sxx) =
            pairs.iter().map(f).fold((0.0, 0.0), |a, (x, y)| (a.0 + (x - mx) * (y - my), a.1 + (x - mx).powi(2)));
        if pairs.len() < 2 || sxx < 1e-3 {
            return (1.0, my - mx);
        }
        let a = sxy / sxx;
        (a, my - a * mx)
    };
    let (ax, bx) = fit(|&((x, _), (u, _))| (x, u));
    let (ay, by) = fit(|&((_, y), (_, v))| (y, v));
    // (the line's shape is made in a space of its own, stretched from the stage's (wider than tall), the one its look
    // was settled in; its swerves, widths and lengths are in that space's units)
    const LOOK: (f32, f32) = (2.086, 1.417);
    let points: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x * LOOK.0, y * LOOK.1)).collect();
    // (a Catmull-Rom curve through the points, a dozen steps a span: it swerves through each)
    let mut line: Vec<(f32, f32)> = Vec::new();
    let n = points.len();
    for i in 0..n - 1 {
        let p0 = points[i.saturating_sub(1)];
        let (p1, p2) = (points[i], points[i + 1]);
        let p3 = points[(i + 2).min(n - 1)];
        for s in 0..12 {
            let t = s as f32 / 12.0;
            let (t2, t3) = (t * t, t * t * t);
            let c = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2 + (3.0 * b - a - 3.0 * c + d) * t3)
            };
            line.push((c(p0.0, p1.0, p2.0, p3.0), c(p0.1, p1.1, p2.1, p3.1)));
        }
    }
    line.push(points[n - 1]);
    // (the game's drop lines swerve: here, walked a stage pixel at a time, the curve bows out to one side between each
    // pair of points and to the other side on the next (a half wave a span, so it flows smoothly through each marker),
    // by more on some spans than others; the same each frame)
    let mut walked: Vec<(f32, f32)> = vec![line[0]];
    let mut along = vec![0.0f32];
    // (how far along each of the path's points is: the line's points start a span of 12 at each)
    let mut marks = vec![0.0f32];
    for (j, w) in line.windows(2).enumerate() {
        if j % 12 == 0 && j > 0 {
            marks.push(along.last().copied().unwrap_or(0.0));
        }
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        let l = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
        let steps = (l / 1.0).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let f = k as f32 / steps as f32;
            walked.push((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f));
            along.push(along.last().copied().unwrap_or(0.0) + l / steps as f32);
        }
    }
    marks.push(along.last().copied().unwrap_or(0.0));
    let bow = |s: f32| {
        let k = marks.iter().rposition(|&m| m <= s).unwrap_or(0).min(marks.len().saturating_sub(2));
        let (m0, m1) = (marks[k], marks.get(k + 1).copied().unwrap_or(marks[k] + 1.0));
        let u = ((s - m0) / (m1 - m0).max(1e-3)).clamp(0.0, 1.0);
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let size = 4.0 * (1.0 + 0.4 * (k as f32 * 1.7 + 0.5).sin());
        side * size * (std::f32::consts::PI * u).sin()
    };
    // (and on top, the game's lines' irregular jogs: a few waves of unlike lengths, so it never repeats; easing off right
    // at each marker, so the line still goes through it)
    let jog = |s: f32| 5.0 * ((s * 0.085 + 0.3).sin() + 0.65 * (s * 0.14 + 2.1).sin() + 0.35 * (s * 0.23 + 1.0).sin());
    let markers_end = marks.len().saturating_sub(4);
    // (smoothed a little, so no turn is a corner)
    let mut shaped: Vec<(f32, f32)> = Vec::with_capacity(walked.len());
    for i in 0..walked.len() {
        let (a, b) = (walked[i.saturating_sub(3)], walked[(i + 3).min(walked.len() - 1)]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        let near =
            marks[..=markers_end.min(marks.len() - 1)].iter().map(|m| (along[i] - m).abs()).fold(f32::MAX, f32::min);
        let k = (near / 7.0).min(1.0);
        let k = k * k * (3.0 - 2.0 * k);
        let w = (-2..=2).map(|d| bow(along[i] + d as f32) + k * jog(along[i] + d as f32)).sum::<f32>() / 5.0;
        shaped.push((walked[i].0 - dy / l * w, walked[i].1 + dx / l * w));
    }
    // (the fit gives stage places; drawn here in this clip's parent's space)
    let into = inst.parent.map_or(Some(Mat2d::IDENTITY), |p| player.world_matrix(p).inverse());
    let Some(into) = into else { return };
    let screen = |x: f32, y: f32| {
        let (x, y) = (x / LOOK.0, y / LOOK.1);
        let (u, v) = into.apply((ax * x + bx) * 20.0, (ay * y + by) * 20.0);
        inh.xf.apply(u, v)
    };
    let world: Vec<(f32, f32)> = shaped.iter().map(|&(x, y)| screen(x, y)).collect();
    // (a brush stroke: its width swells and thins along the way, and it narrows to a point at the bottom (and a little
    // at the top, where the brush goes down))
    let total = along.last().copied().unwrap_or(1.0).max(1.0);
    let width: Vec<f32> = along
        .iter()
        .map(|&s| {
            let swell = 1.0 + 0.35 * (s * 0.031 + 1.3).sin() + 0.15 * (s * 0.093 + 0.2).sin();
            let end = ((total - s) / 70.0).clamp(0.0, 1.0);
            let start = (s / 14.0).clamp(0.0, 1.0);
            let ease = |x: f32| x * x * (3.0 - 2.0 * x);
            swell * (0.08 + 0.92 * ease(end)) * (0.55 + 0.45 * ease(start))
        })
        .collect();
    let scale = {
        let (a, b) = (screen(0.0, 0.0), screen(1.0, 1.0));
        (((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)) * 0.5).sqrt().max(1e-3)
    };
    // (the seconds cut to the last thousand before they're made an f32: the whole count since 1970 is too big for one to
    // hold its fractions, so it would stand still for minutes at a time)
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| (d.as_secs() % 1000) as f32 + d.subsec_nanos() as f32 * 1e-9);
    // (and, as the game's do, it pulses: it swells to white and eases back to its colour)
    // (timed off Mount Robson's: every 1.35 s it swells for about half a second, eases back over another half, then
    // rests in its colour for the rest)
    let p = (t % 1.35) / 0.97;
    let pulse = if p < 1.0 { (std::f32::consts::PI * p).sin().powi(2) } else { 0.0 };
    // (one strip down the line, its sides `core` (times the brush's width there) and `extra` (the glow, the same all
    // along but where the brush is thinnest) out either side: no part of it laid over another, so a faint stroke is
    // evenly faint. On the inside of a bend a side goes no further out than the bend's radius, so a wide strip never
    // folds back over itself at a tight turn)
    let stroke = |core: f32, extra: f32| {
        let mut mesh = Mesh::default();
        let last = world.len().saturating_sub(1);
        let unit = |v: (f32, f32)| {
            let l = (v.0 * v.0 + v.1 * v.1).sqrt().max(1e-6);
            (v.0 / l, v.1 / l, l)
        };
        for (i, &(x, y)) in world.iter().enumerate() {
            let (a, b) = (world[i.saturating_sub(2)], world[(i + 2).min(last)]);
            let (tx, ty, _) = unit((b.0 - a.0, b.1 - a.1));
            let (nx, ny) = (-ty, tx);
            let h = core * width[i] + extra * (0.4 + width[i]).min(1.0);
            let (p, q) = (world[i.saturating_sub(4)], world[(i + 4).min(last)]);
            let (ax, ay, la) = unit((x - p.0, y - p.1));
            let (bx, by, lb) = unit((q.0 - x, q.1 - y));
            let turn = (ax * by - ay * bx).atan2(ax * bx + ay * by);
            let radius = if turn.abs() > 1e-4 { 0.5 * (la + lb) / turn.abs() } else { f32::MAX };
            let inner = (0.9 * radius).max(0.5);
            let (left, right) = if turn > 0.0 { (h.min(inner), h) } else { (h, h.min(inner)) };
            mesh.vertices.extend([[x + nx * left, y + ny * left], [x - nx * right, y - ny * right]]);
            if i > 0 {
                let v = 2 * i as u32;
                mesh.indices.extend([v - 2, v - 1, v + 1, v - 2, v + 1, v]);
            }
        }
        Arc::new(mesh)
    };
    // (the colours of the game's lines, read off a recording: Race It blue, Trick It yellow; each a glow, a body and a
    // pale core)
    let race = ssx_assets::content::data::dev_peak::race_mode();
    // (Mount Robson's line, read off screenshots at 3440 x 1440: a neon tube. A solid pale core (cyan-white for Race
    // It), as wide as the brush is there, its edge sharp; around it a glow in its colour, strongest at the core's edge
    // and gone about 7 px out. As it pulses the core goes white and the glow paler (cyan), stronger and wider)
    let (core_lo, glow_lo, core_hi, glow_hi) = if race {
        ([0.82, 0.98, 1.0], [0.3, 0.5, 0.95], [1.0, 1.0, 1.0], [0.6, 0.9, 0.95])
    } else {
        ([1.0, 0.97, 0.75], [1.0, 0.62, 0.15], [1.0, 1.0, 1.0], [1.0, 0.88, 0.55])
    };
    let lerp = |a: [f32; 3], b: [f32; 3], k: f32| {
        [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k]
    };
    let (core, glow) = (lerp(core_lo, core_hi, pulse), lerp(glow_lo, glow_hi, pulse));
    // (the core's half width, in this space's units (about 1.2 px each at 1440 high), times the brush's width)
    let core_half = 2.6;
    // (the glow: nested strips from its outer edge in, each faint enough that together its strength a distance `g`
    // past the core's edge is a bell, exp(-(g/σ)²/2); fitted to the screenshot's within a few shades)
    const LAYERS: usize = 14;
    let sigma = 4.0 + 1.5 * pulse;
    let reach = 3.0 * sigma;
    let strength = |g: f32| (-(g / sigma).powi(2) * 0.5).exp();
    let mut below = 0.0f32;
    for k in 0..LAYERS {
        let g = reach * (1.0 - k as f32 / LAYERS as f32);
        let mid = reach * (1.0 - (k as f32 + 0.5) / LAYERS as f32);
        let want = strength(mid);
        let a = (1.0 - (1.0 - want) / (1.0 - below).max(1e-4)).clamp(0.0, 1.0);
        below = want.max(below);
        // (paler near the core)
        let near = 1.0 - mid / reach;
        let c = lerp(glow, core, 0.9 * near * near);
        out.push(Command::Draw(DrawItem {
            mesh: stroke(core_half * scale, g * scale),
            paint: Paint::Solid([c[0], c[1], c[2], a]),
            transform: Mat2d::IDENTITY,
            cxform: inh.cx,
            blend: BlendMode::Normal,
        }));
    }
    // (the core, solid and sharp-edged)
    out.push(Command::Draw(DrawItem {
        mesh: stroke(core_half * scale, 0.0),
        paint: Paint::Solid([core[0], core[1], core[2], 1.0]),
        transform: Mat2d::IDENTITY,
        cxform: inh.cx,
        blend: BlendMode::Normal,
    }));
}

/// Whether the loading screen is up (set by the menus each frame).
pub static LOADING_SCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Port mod: the menus' side dimmer (dimmer.gfx: `mcDimmer`'s dark sheet down the left of the stage, under the World
// Tour and Explore panels, character 2) is not drawn: on a wide screen it shows as a strip of another shade down the
// left (v0.4.0 draws it on out to the window's edge). The full-screen dimmer beside it (character 4) is kept.
fn hidden_side_dimmer(lib: &Library, player: &Player, inst: &Instance) -> bool {
    // (not on the loading screen: there the sheet, drawn on out to the window's edges, is the black beside the picture)
    !LOADING_SCREEN.load(std::sync::atomic::Ordering::Relaxed)
        && inst.character.as_ref().is_some_and(|c| c.id == 2)
        && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcDimmer")
        && lib.movie(inst.movie).path.to_ascii_lowercase().rsplit(['/', '\\']).next() == Some("dimmer.gfx")
}

// Port mod: the front-end dock's three empty tab boxes (left of the menu options) are not drawn.
fn hidden_dock_tab(player: &Player, inst: &Instance) -> bool {
    matches!(inst.name.as_str(), "mcTabTop" | "mcTabMiddle" | "mcTabBottom")
        && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcDockFE")
}

// Port mod: the standard popup's band (`mcPanel`, as wide as the screen) and its title glow
// (`mcTitleVapor`) are drawn narrower, so prompts such as quitting sit in a centred box.
// Popups showing a loaded picture keep the full band.
const POPUP_BOX: f32 = 0.36;

fn narrow_popup(player: &Player, inst: &Instance) -> Option<Mat2d> {
    let parent = player.get(inst.parent?)?;
    let (popup, about) = match inst.name.as_str() {
        "mcPanel" if parent.name == "mcPanelContainer" => (inst.parent.and_then(|p| player.get(p)?.parent)?, 0.0),
        "mcTitleVapor" if parent.name == "mcPopup" => (inst.parent?, 640.0 * 20.0),
        _ => return None,
    };
    let popup_inst = player.get(popup)?;
    if popup_inst.name != "mcPopup" || popup_shows_picture(player, popup) {
        return None;
    }
    // The band is widened so its solid middle keeps the box's width, its fading ends outside it (`BAND_FADE`).
    let k = if inst.name == "mcPanel" { POPUP_BOX / (1.0 - 2.0 * BAND_FADE) } else { POPUP_BOX };
    let m = inst.matrix;
    Some(Mat2d { a: m.a * k, c: m.c * k, tx: about + (m.tx - about) * k, ..m })
}

// Port mod: the narrowed band's ends fade out to nothing instead of stopping in a hard edge (its gradient runs left to
// right across it, opaque at both ends: drawn screen-wide in the game, its ends were off the screen). Each fading end
// is BAND_FADE of the band's width, added outside the box so the fade stays clear of the text.
const BAND_FADE: f32 = 0.2;

fn in_narrow_popup_band(player: &Player, inst: &Instance) -> bool {
    let Some(panel) = inst.parent.and_then(|p| player.get(p)) else { return false };
    panel.name == "mcPanel" && narrow_popup(player, panel).is_some()
}

fn faded_band(mesh: &ShapeMesh) -> ShapeMesh {
    let layers = mesh
        .layers
        .iter()
        .map(|l| match &l.paint {
            Paint::Gradient(g) if g.kind == shape::GradientKind::Linear => {
                shape::ShapeLayer { paint: Paint::Gradient(Arc::new(fade_ends(g))), mesh: l.mesh.clone() }
            }
            _ => l.clone(),
        })
        .collect();
    ShapeMesh { layers }
}

/// The gradient with its alpha eased to nothing over its first and last `BAND_FADE`.
fn fade_ends(g: &shape::GradientPaint) -> shape::GradientPaint {
    // Smootherstep: flat at both ends, so neither the band's edge nor the fade's start shows as a line.
    let envelope = |t: f32| {
        let e = (t.min(1.0 - t) / BAND_FADE).clamp(0.0, 1.0);
        e * e * e * (e * (e * 6.0 - 15.0) + 10.0)
    };
    const STEPS: usize = 24;
    let mut at: Vec<f32> = (0..=STEPS).map(|i| i as f32 / STEPS as f32 * BAND_FADE).collect();
    at.extend((0..=STEPS).map(|i| 1.0 - BAND_FADE + i as f32 / STEPS as f32 * BAND_FADE));
    at.extend(g.stops.iter().map(|s| s.0));
    at.sort_by(f32::total_cmp);
    at.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    let flat = shape::GradientPaint { spread: 0, ..g.clone() };
    let stops = at
        .into_iter()
        .map(|t| {
            let mut c = flat.sample(t);
            c[3] *= envelope(t);
            (t, c)
        })
        .collect();
    shape::GradientPaint { stops, spread: 0, ..g.clone() }
}

fn popup_shows_picture(player: &Player, popup: InstanceId) -> bool {
    let Some(image) = player.child_by_name(popup, "mcPopupImage", true) else { return false };
    let mut stack = vec![image];
    while let Some(id) = stack.pop() {
        let Some(i) = player.get(id) else { continue };
        if !i.visible {
            continue;
        }
        match &i.kind {
            InstanceKind::Bitmap => return true,
            InstanceKind::Sprite(s) => stack.extend(s.children.values().copied()),
            _ => {}
        }
    }
    false
}

// Port mod (the menus' 3-D rider showcase): the highlighted rider's card in the list (a portrait holder showing its
// pictures, `mac_top5.dds` for `mac`, not one of the faded copies at the row's ends) has its portrait's place on the
// screen noted (the 3-D rider stands there), and its portrait isn't drawn while the rider shows.
// The 3-D rider showcase: the highlighted card grows downwards and sideways so the model (and the board it shows)
// has the card to itself, with the level number and XP bar moved below it and the cards either side moved apart.
// `GROW` names the card (its inner `mcCell`) and how far, in the card's own units and in window pixels; `PREV` keeps
// last frame's (the cards to its left draw before it), and the panel under the cards moves down with it.
#[derive(Clone, Copy)]
struct Grow {
    cell: InstanceId,
    outer: Option<InstanceId>,
    by: f32,
    wide: f32,
    /// The portrait's centre line and width (the card's own units); every part widens to the right of the card's
    /// left edge (`pivot`, and in window pixels `pivot_px`), by one ratio, so the card's left side stays put.
    centre: f32,
    width: f32,
    pivot: f32,
    by_px: f32,
    wide_px: f32,
    centre_px: f32,
    pivot_px: f32,
    /// The card's parts as the menu has them (the card's own units): its backing's left, right and top, the inner
    /// square's bottom, the XP bar's top and bottom.
    backer: [f32; 3],
    inner_bottom: f32,
    bar: [f32; 2],
    /// The inner square's left and right (as the menu has them).
    inner_x: [f32; 2],
}
static GROW: std::sync::Mutex<Option<Grow>> = std::sync::Mutex::new(None);
static PREV: std::sync::Mutex<Option<Grow>> = std::sync::Mutex::new(None);
const GROW_FRACTION: f32 = 0.2;
const WIDEN_FRACTION: f32 = 0.55;
/// The menu's card backing has see-through margins at its sides (this much of its width each): the card seen starts
/// and ends inside them.
const CARD_MARGIN: f32 = 0.117;
/// The rider list's highlighted card sits this many card spacings left of the header's EXPLORE box's left edge.
const PIN_NUDGE: f32 = 0.22;
/// …and then this many window pixels to the right.
const PIN_SHIFT_PX: f32 = 20.0;
/// The highlighted card's inner box's top: this many pixels higher than its lowered place.
const INNER_TOP_RAISE_PX: f32 = 14.0;
/// The rider list's plain cards' edge colour (teal).
const PLAIN_EDGE: [f32; 3] = [0.33, 0.62, 0.65];
/// The gear and perks box over the rider list: this many pixels left of the highlighted card's border.
const BIO_SHIFT_PX: f32 = 3.0;
/// The rider list's cards (and the description under the highlighted one): this many pixels lower than the menu has them.
const LIST_DROP_PX: f32 = 25.0;
/// The gear and perks box over the rider list: this many pixels lower than the menu has it.
const BIO_DROP_PX: f32 = 2.0;
/// The drop selector's event tab's title (the drop and event names): lowered this much of its own height, to sit
/// in the middle of its tab (it sat at the tab's top).
const EVENT_TITLE_DROP: f32 = 0.36;
/// The grown card's level number is drawn this much larger.
const LEVEL_SCALE: f32 = 1.2;
/// …and this many window pixels lower than midway between the inner square and the XP bar.
const LEVEL_DROP_PX: f32 = 2.0;

// The grown card's window rectangle (its highlight frame), this frame and last: the bio drawn before the cards
// goes under it.
static CARD_NOW: std::sync::Mutex<Option<RectF>> = std::sync::Mutex::new(None);
static CARD_PREV: std::sync::Mutex<Option<RectF>> = std::sync::Mutex::new(None);

/// The highlighted card's colours: its inside, its edge's line and the glow either side of it.
const CARD_FILL: [f32; 3] = [0.01, 0.28, 0.17];
const CARD_EDGE: [f32; 3] = [0.62, 1.0, 0.88];
const CARD_GLOW: [f32; 3] = [0.13, 0.95, 0.62];
/// The highlighted card's inner line's brightness last drawn, as a fraction of its brightest lately (and that).
static INNER_PULSE: std::sync::Mutex<(f32, f32)> = std::sync::Mutex::new((1.0, 0.0));
/// The highlighted card's inner line's top comes down this much of its height (room for the rider's name).
const INNER_TOP_DROP: f32 = 0.035;

/// The grown card (`inh`: its cell's): its rectangle (in the cell's units: the backing's widened left to right, its
/// top down to a little below the XP bar), filled solid, edged with a bright line with a glow inside and out (sizes
/// in window pixels). Also where the rider's bio goes under it.
/// How much wider the grown card is than the others (the card seen, the card's own units): the cards after it move
/// over by this, so the gap after it is the others'.
/// How far the grown card's inner square (and the model in it) moves over to sit centred on the card (the card's
/// own units).
fn inner_shift(g: Grow) -> f32 {
    let k = (g.width + g.wide) / g.width.max(1e-3);
    let seen_right = g.backer[1] - (g.backer[1] - g.backer[0]) * CARD_MARGIN;
    let right = g.pivot + (seen_right - g.pivot) * k;
    let [ix0, ix1] = g.inner_x.map(|x| g.pivot + (x - g.pivot) * k);
    (g.pivot + right) / 2.0 - (ix0 + ix1) / 2.0
}

fn grown_extra(g: Grow) -> f32 {
    let k = (g.width + g.wide) / g.width.max(1e-3);
    let seen = (g.backer[1] - g.backer[0]) * (1.0 - 2.0 * CARD_MARGIN);
    seen * (k - 1.0)
}

fn grown_card(inh: &Inherited, out: &mut Vec<Command>) {
    let Some(g) = GROW.lock().ok().and_then(|g| *g) else { return };
    let k = (g.width + g.wide) / g.width.max(1e-3);
    let seen_right = g.backer[1] - (g.backer[1] - g.backer[0]) * CARD_MARGIN;
    let right = g.pivot + (seen_right - g.pivot) * k;
    let bar_bottom = g.bar[1] + g.by;
    let gap = (g.bar[1] - g.bar[0]).max(1.0) * 0.9;
    let (ax, ay) = inh.xf.apply(g.pivot, g.backer[2]);
    let (bx, by) = inh.xf.apply(right, bar_bottom + gap);
    let r = RectF { x0: ax.min(bx), y0: ay.min(by), x1: ax.max(bx), y1: ay.max(by) };
    if let Ok(mut now) = CARD_NOW.lock() {
        *now = Some(r);
    }
    // (on whole pixels, so its edges are crisp)
    let r = RectF { x0: r.x0.round(), y0: r.y0.round(), x1: r.x1.round(), y1: r.y1.round() };
    let alpha = inh.cx.mult[3].clamp(0.0, 1.0);
    // A ring one pixel wide, `d` pixels outside the card's edge (inside it when negative): its four sides meeting
    // without overlapping.
    let ring = |out: &mut Vec<Command>, d: f32, colour: [f32; 3], a: f32| {
        let (x0, y0, x1, y1) = (r.x0 - d - 1.0, r.y0 - d - 1.0, r.x1 + d + 1.0, r.y1 + d + 1.0);
        if x1 - x0 < 2.0 || y1 - y0 < 2.0 {
            return;
        }
        let mut mesh = Mesh::default();
        for [qx0, qy0, qx1, qy1] in [
            [x0, y0, x1, y0 + 1.0],
            [x0, y1 - 1.0, x1, y1],
            [x0, y0 + 1.0, x0 + 1.0, y1 - 1.0],
            [x1 - 1.0, y0 + 1.0, x1, y1 - 1.0],
        ] {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([[qx0, qy0], [qx1, qy0], [qx1, qy1], [qx0, qy1]]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid([colour[0], colour[1], colour[2], a]),
            transform: Mat2d::IDENTITY,
            cxform: Cxform::IDENTITY,
            blend: BlendMode::Normal,
        }));
    };
    // The glow pulses (inside and out) with the card's inner line, as the menu pulses that (its brightness last
    // frame: the line's drawn after this).
    let pulse = INNER_PULSE.lock().map_or(1.0, |p| p.0);
    // the glow outside the edge: fading out smoothly
    let scale = ((r.y1 - r.y0).abs() / 760.0).max(0.5);
    // (spreading wider and a little brighter under the mouse; rounded round the corners, as a glow spreads)
    let hover = card_hover();
    let reach = 16.0 * scale * (1.0 + 0.8 * hover);
    let layers = 24;
    for i in (0..layers).rev() {
        let (d0, d1) = (reach * i as f32 / layers as f32, reach * (i + 1) as f32 / layers as f32);
        let t = 1.0 - (i as f32 + 0.5) / layers as f32;
        let a = (0.55 + 0.15 * hover) * t * t * alpha * pulse;
        soft_ring(out, [r.x0, r.y0, r.x1, r.y1], d0, d1, [CARD_GLOW[0], CARD_GLOW[1], CARD_GLOW[2], a]);
    }
    // the card: solid
    {
        let mut mesh = Mesh::default();
        mesh.vertices.extend([[r.x0, r.y0], [r.x1, r.y0], [r.x1, r.y1], [r.x0, r.y1]]);
        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid([CARD_FILL[0], CARD_FILL[1], CARD_FILL[2], 1.0]),
            transform: Mat2d::IDENTITY,
            cxform: Cxform::IDENTITY,
            blend: BlendMode::Normal,
        }));
    }
    // the glow inside the edge: brightening smoothly towards it
    let inner = (22.0 * scale).round().max(4.0) as i32;
    for i in (0..inner).rev() {
        let t = 1.0 - (i as f32 + 0.5) / inner as f32;
        let k = 0.75 * t * t * pulse;
        let mix: [f32; 3] = std::array::from_fn(|c| CARD_FILL[c] + (CARD_GLOW[c] - CARD_FILL[c]) * k);
        ring(out, -(i as f32) - 2.0, mix, 1.0);
    }
    // the edge's line: bright, softened either side
    let line = (2.0 * scale).round().max(2.0) as i32;
    for i in 0..line {
        ring(out, -(i as f32) - 1.0, CARD_EDGE, alpha);
    }
    ring(out, -(line as f32) - 1.0, CARD_EDGE, 0.45 * alpha);
    ring(out, 0.0, CARD_EDGE, 0.6 * alpha);
    scroll_arrows(r, alpha, scale, out);
}

/// The scroll arrows' width and height, and the room kept clear either side of one (window pixels), for a highlighted
/// card this tall.
fn arrow_size(card_h: f32) -> (f32, f32, f32) {
    let scale = (card_h / 760.0).max(0.5);
    let h = card_h * 0.045;
    let w = h * 0.62;
    let m = (14.0 * scale).round().max(5.0);
    (w, h, m)
}

/// The rider list's last scroll: which way (-1 left, 1 right) and when (the arrow that way glows, fading).
static LIST_SCROLL: std::sync::Mutex<Option<(i8, std::time::Instant)>> = std::sync::Mutex::new(None);
/// How long a scroll's arrow glow takes to fade (seconds).
const ARROW_GLOW_SECS: f32 = 0.45;
/// The arrows' brightness when not glowing.
const ARROW_REST: f32 = 0.45;

/// The arrows either side of the highlighted card's bottom (`r`, its window rectangle), just outside its edges, under
/// the cards beside it: the one the list last scrolled towards glowing, fading back.
fn scroll_arrows(r: RectF, alpha: f32, scale: f32, out: &mut Vec<Command>) {
    if alpha <= 0.001 {
        return;
    }
    let glow = LIST_SCROLL.lock().ok().and_then(|s| *s).map(|(dir, at)| {
        let t = (at.elapsed().as_secs_f32() / ARROW_GLOW_SECS).clamp(0.0, 1.0);
        (dir, 1.0 - t * t * (3.0 - 2.0 * t))
    });
    let _ = scale;
    let (w, h, m) = arrow_size((r.y1 - r.y0).abs());
    // (level with the middle of the cards either side)
    let mid = f32::from_bits(ARROW_MID_Y.load(std::sync::atomic::Ordering::Relaxed));
    let cy = if mid > r.y0 && mid < r.y1 { mid } else { (r.y0 + r.y1) * 0.5 };
    // (close beside the card's edge)
    let gap = m + w * 0.5;
    let tri = |out: &mut Vec<Command>, dir: f32, cx: f32, grow: f32, colour: [f32; 3], a: f32| {
        let (hw, hh) = (w * 0.5 * grow, h * 0.5 * grow);
        let mut mesh = Mesh::default();
        // (the tip pointing out, away from the card)
        mesh.vertices.extend([[cx + dir * hw, cy], [cx - dir * hw, cy - hh], [cx - dir * hw, cy + hh]]);
        mesh.indices.extend([0, 1, 2]);
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid([colour[0], colour[1], colour[2], a.clamp(0.0, 1.0)]),
            transform: Mat2d::IDENTITY,
            cxform: Cxform::IDENTITY,
            blend: BlendMode::Normal,
        }));
    };
    for (dir, cx) in [(-1.0f32, r.x0 - gap), (1.0, r.x1 + gap)] {
        // (the list moving one way, the arrow on the other side lights: the side the cards come in from)
        let lit = glow.filter(|g| f32::from(g.0) == -dir).map_or(0.0, |g| g.1);
        // (its glow: soft layers growing out from it)
        if lit > 0.001 {
            for i in (1..=6).rev() {
                let grow = 1.0 + i as f32 * 0.16;
                tri(out, dir, cx, grow, CARD_GLOW, 0.16 * lit * alpha * (1.0 - i as f32 / 7.0));
            }
        }
        let k = ARROW_REST + (1.0 - ARROW_REST) * lit;
        let colour: [f32; 3] = std::array::from_fn(|c| PLAIN_EDGE[c] + (CARD_EDGE[c] - PLAIN_EDGE[c]) * lit);
        tri(out, dir, cx, 1.0, colour, k * alpha);
    }
}

thread_local! {
    static OPAQUE_CARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The menu's backdrop behind the rider cards (dark), that a see-through fill is put over to draw it solid.
const CARD_BACKDROP: [f32; 3] = [0.03, 0.08, 0.09];

fn opaque_paint(p: &Paint) -> Paint {
    let solid = |c: [f32; 4]| {
        let a = c[3];
        [
            c[0] * a + CARD_BACKDROP[0] * (1.0 - a),
            c[1] * a + CARD_BACKDROP[1] * (1.0 - a),
            c[2] * a + CARD_BACKDROP[2] * (1.0 - a),
            1.0,
        ]
    };
    match p {
        Paint::Solid(c) => Paint::Solid(solid(*c)),
        Paint::Gradient(g) => {
            let mut g = (**g).clone();
            for stop in &mut g.stops {
                stop.1 = solid(stop.1);
            }
            Paint::Gradient(Arc::new(g))
        }
        other => other.clone(),
    }
}

/// A colour transform drawing a layer solid: its own fade off; a picture's partly see-through pixels made solid, its
/// clear ones (outside the card's shape) left clear.
fn opaque_cx(cx: Cxform, paint: &Paint) -> Cxform {
    let mut cx = cx;
    cx.mult[3] = if matches!(paint, Paint::Bitmap { .. }) { 64.0 } else { 1.0 };
    cx.add[3] = 0.0;
    cx
}

/// The rider list's highlighted card's parts as the menu colours them once its card is fully highlighted (its inner
/// cell on its last highlight frame): its name and level green. The menu tweens them there from the plain card's blue
/// over a few frames as a card's highlighted; the highlighted card shows them so from the start.
static LIT_LOOK: std::sync::Mutex<Vec<(String, Cxform)>> = std::sync::Mutex::new(Vec::new());
/// The highlighted card's inner cell's frame once it's fully highlighted.
const LIT_FRAME: u16 = 37;

/// A part of the highlighted card (or a part of a part): its colours as the fully highlighted card has them, while the
/// card's still on its way there.
fn lit_look(player: &Player, id: InstanceId, inst: &Instance) -> Option<Cxform> {
    if ghosting() {
        return None;
    }
    let g = GROW.lock().ok().and_then(|g| *g)?;
    // (its names from the card's inner cell down: the key the look's kept by)
    let mut key = Vec::new();
    let (mut at, mut i) = (id, inst);
    for _ in 0..4 {
        key.push(if i.name.is_empty() { format!("#{}", i.depth) } else { i.name.clone() });
        let parent = i.parent?;
        if parent == g.cell {
            break;
        }
        at = parent;
        i = player.get(at)?;
    }
    if i.parent != Some(g.cell) {
        return None;
    }
    let InstanceKind::Sprite(s) = &player.get(g.cell)?.kind else { return None };
    let key = key.join("/");
    let mut look = LIT_LOOK.lock().ok()?;
    if s.current_frame == LIT_FRAME {
        match look.iter_mut().find(|e| e.0 == key) {
            Some(e) => e.1 = inst.cxform,
            None => look.push((key, inst.cxform)),
        }
        None
    } else {
        look.iter().find(|e| e.0 == key).map(|e| e.1)
    }
}

/// The drop selector's event tab's title (`mcIndicatorRoot.mcTitle` of dropselector.gfx, on the globe and over the
/// equipment screen): lowered to the middle of its tab.
fn centred_event_title(lib: &Library, player: &Player, id: InstanceId, inst: &Instance, matrix: Mat2d) -> Mat2d {
    if inst.name != "mcTitle"
        || !inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcIndicatorRoot")
        || !lib.movie(inst.movie).path.to_ascii_lowercase().ends_with("dropselector.gfx")
    {
        return matrix;
    }
    // (not when the tab shows another line under the title: its red "<gear> Required", which the title's own place
    // leaves room for)
    let second_line = inst.parent.and_then(|p| player.get(p)).is_some_and(|root| {
        let InstanceKind::Sprite(s) = &root.kind else { return false };
        let mut stack: Vec<(InstanceId, usize)> = s.children.values().filter(|&&c| c != id).map(|&c| (c, 0)).collect();
        while let Some((k, depth)) = stack.pop() {
            let Some(i) = player.get(k) else { continue };
            if !i.visible || i.cxform.mult[3] <= 0.0 {
                continue;
            }
            match &i.kind {
                InstanceKind::EditText(t) if !t.layout.glyphs.is_empty() => return true,
                InstanceKind::Sprite(s) if depth < 3 => stack.extend(s.children.values().map(|&c| (c, depth + 1))),
                _ => {}
            }
        }
        false
    });
    if second_line {
        return matrix;
    }
    let b = player.local_bounds(lib, id);
    if b.is_empty() {
        return matrix;
    }
    let h = b.transform(&matrix);
    Mat2d::translate(0.0, (h.y1 - h.y0) * EVENT_TITLE_DROP).mul(&matrix)
}

/// The equipment screen's gear steps, the chosen rider in 3-D (the 3D Rider Select mod): its card gone but for its level
/// and XP bar (under the rider's view), the screen's options in a column right of the view. Worked out each frame from
/// where the menu has them: the parts moved (window pixels, and the XP bar's widening) and the parts not drawn.
struct GearLayout {
    moved: Vec<(InstanceId, f32, f32, f32, f32)>,
    hidden: Vec<InstanceId>,
}
static GEAR: std::sync::Mutex<Option<GearLayout>> = std::sync::Mutex::new(None);
/// The rider's view: this wide (a fraction of the card's height), a gap (the same) between it and the column right of
/// it.
const GEAR_VIEW_WIDTH: f32 = 0.82;
const GEAR_GAP: f32 = 0.035;
/// The rider's view (and its frame), level and XP bar: this many pixels lower than worked out.
const GEAR_DROP_PX: f32 = 10.0;
/// The Survival box: this many pixels right of the options' column, and up from just under them.
const SURVIVAL_RIGHT_PX: f32 = 3.0;
const SURVIVAL_UP_PX: f32 = 5.0;

/// The rider's view's thin blue frame (the 3D Rider Select mod), as laid out this frame: where (window pixels) and how
/// shown (the screen's fading).
static GEAR_FRAME: std::sync::Mutex<Option<([f32; 4], f32)>> = std::sync::Mutex::new(None);

fn set_gear_frame(rect: [f32; 4], alpha: f32) {
    if let Ok(mut f) = GEAR_FRAME.lock() {
        *f = Some((rect, alpha));
    }
}

/// How shown an instance is: its own and its ancestors' alpha.
fn shown_alpha(player: &Player, id: InstanceId) -> f32 {
    let mut a = 1.0;
    let mut at = Some(id);
    while let Some(i) = at.and_then(|k| player.get(k)) {
        if !i.visible {
            return 0.0;
        }
        a *= i.cxform.mult[3];
        at = i.parent;
    }
    a.clamp(0.0, 1.0)
}

/// The highlighted card's inner frame as last drawn on the rider step: where (window pixels) and its colour.
static INNER_LAST: std::sync::Mutex<Option<([f32; 4], [f32; 4])>> = std::sync::Mutex::new(None);
/// The rider's view's frame as drawn: where (window pixels), and when.
static FRAME_NOW: std::sync::Mutex<Option<([f32; 4], std::time::Instant)>> = std::sync::Mutex::new(None);

/// The rider's view's frame on the gear steps (the 3D Rider Select mod): the highlighted card's inner frame, pulsing and
/// glowing as it does there, grown with the rider from the card to the view (and back to the card on the way back to
/// the rider step), as the showcase moves the rider's view.
/// The loading screen up: the rider's view's frame let go (not carried on from where it was afterwards).
fn loading_overlays_off() {
    if let Ok(mut f) = FRAME_NOW.lock() {
        *f = None;
    }
    if let Ok(mut m) = FRAME_MOVE.lock() {
        *m = None;
    }
    if let Ok(mut a) = FRAME_ALPHA.lock() {
        *a = 0.0;
    }
    if let Ok(mut k) = KEPT_SINCE.lock() {
        *k = None;
    }
}

fn gear_frame(out: &mut Vec<Command>) {
    let target = GEAR_FRAME.lock().ok().and_then(|f| *f);
    let inner = INNER_LAST.lock().ok().and_then(|f| *f);
    let Ok(mut state) = FRAME_NOW.lock() else { return };
    let now = std::time::Instant::now();
    let dt = (*state).map_or(0.0, |(_, at)| now.duration_since(at).as_secs_f32()).min(0.1);
    // (the showcase's own moving of the view: a long way in one eased move)
    let mut mv = FRAME_MOVE.lock().ok().and_then(|m| *m);
    let mut ease = |from: [f32; 4], to: [f32; 4]| -> [f32; 4] {
        ssx_core::rider_showcase::view_step(from, to, dt, 40.0, 9.0, &mut mv)
    };
    let back = ssx_core::rider_showcase::enabled() && !ssx_core::rider_showcase::highlight_portrait().is_empty();
    let (rect, alpha) = match (target, (*state).map(|s| s.0), inner) {
        (Some((to, a)), Some(cur), _) => (ease(cur, to), a),
        (Some((_, a)), None, Some((from, _))) => (from, a),
        (Some((to, a)), None, None) => (to, a),
        (None, Some(cur), Some((to, _))) if back => {
            let r = ease(cur, to);
            if mv.is_none() && r.iter().zip(to).all(|(a, b)| (a - b).abs() < 2.0) {
                *state = None;
                if let Ok(mut m) = FRAME_MOVE.lock() {
                    *m = None;
                }
                return;
            }
            (r, 1.0)
        }
        // (none for a moment (the screen changing: the full list opening or closing): kept where it was, and still
        // drawn (no blink), so the frame carries on from there)
        (None, Some(cur), _) => {
            let mut kept = KEPT_SINCE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let since = *kept.get_or_insert(now);
            if now.duration_since(since).as_secs_f32() > 1.0 {
                *kept = None;
                *state = None;
                if let Ok(mut a) = FRAME_ALPHA.lock() {
                    *a = 0.0;
                }
                return;
            }
            drop(kept);
            *state = Some((cur, now));
            // (fading as it would (the screen may be going away))
            let a = FRAME_ALPHA.lock().map_or(0.0, |mut held| {
                *held = (*held - dt * FRAME_FADE_RATE).max(0.0);
                *held
            });
            draw_frame(out, cur, a, inner, now);
            return;
        }
        _ => {
            *state = None;
            mv = None;
            if let Ok(mut m) = FRAME_MOVE.lock() {
                *m = mv;
            }
            return;
        }
    };
    if let Ok(mut m) = FRAME_MOVE.lock() {
        *m = mv;
    }
    if let Ok(mut kept) = KEPT_SINCE.lock() {
        *kept = None;
    }
    *state = Some((rect, now));
    // (the screens' fading (the full list's opening or closing) doesn't blink it: it fades out no quicker than this)
    let alpha = match FRAME_ALPHA.lock() {
        Ok(mut held) => {
            let a = alpha.max(*held - dt * FRAME_FADE_RATE).clamp(0.0, 1.0);
            *held = a;
            a
        }
        Err(_) => alpha,
    };
    draw_frame(out, rect, alpha, inner, now);
}

/// How quickly the rider's view's frame may fade out (per second).
const FRAME_FADE_RATE: f32 = 1.25;
/// The rider's view's frame's alpha as last drawn.
static FRAME_ALPHA: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);

/// The rider's view's frame drawn: its line and glow, pulsing.
/// A card of the rider list under the mouse is drawn this much larger (about its middle).
const HOVER_GROW: f32 = 1.04;

/// The rider list's card under the mouse, in the drawing's pixels (as last drawn).
fn hovered_card_px() -> Option<RectF> {
    if !ssx_core::rider_showcase::enabled() || ssx_core::rider_showcase::gear_view() {
        return None;
    }
    let [x0, y0, x1, y1] = ssx_core::rider_showcase::hovered_card()?;
    let (w, h) = FRAME_SIZE.lock().ok().and_then(|s| *s)?;
    Some(RectF { x0: x0 * w, y0: y0 * h, x1: x1 * w, y1: y1 * h })
}

/// The drawing's size this frame (pixels).
static FRAME_SIZE: std::sync::Mutex<Option<(f32, f32)>> = std::sync::Mutex::new(None);

/// The blue glow round the rider list's card under the mouse (`rect`, the drawing's pixels): a bright line and a
/// soft glow outside it.
fn hover_glow(out: &mut Vec<Command>, rect: [f32; 4], alpha: f32) {
    let [x0, y0, x1, y1] = rect;
    if alpha <= 0.0 || x1 - x0 < 4.0 || y1 - y0 < 4.0 {
        return;
    }
    let ring = |out: &mut Vec<Command>, d: f32, w: f32, c: [f32; 4]| {
        let (a0, b0, a1, b1) = (x0 - d - w, y0 - d - w, x1 + d + w, y1 + d + w);
        let (i0, j0, i1, j1) = (x0 - d, y0 - d, x1 + d, y1 + d);
        let mut mesh = Mesh::default();
        for [qx0, qy0, qx1, qy1] in [[a0, b0, a1, j0], [a0, j1, a1, b1], [a0, j0, i0, j1], [i1, j0, a1, j1]] {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([[qx0, qy0], [qx1, qy0], [qx1, qy1], [qx0, qy1]]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid(c),
            transform: Mat2d::IDENTITY,
            cxform: Cxform::IDENTITY,
            blend: BlendMode::Normal,
        }));
    };
    let glow = 12;
    for i in (0..glow).rev() {
        let f = 1.0 - (i as f32 + 0.5) / glow as f32;
        ring(out, 1.0 + i as f32, 1.0, [0.2, 0.55, 1.0, 0.55 * f * f * alpha]);
    }
    // (the line over the frame's own edge)
    ring(out, -1.5, 3.0, [0.6, 0.85, 1.0, alpha]);
}

fn draw_frame(out: &mut Vec<Command>, rect: [f32; 4], alpha: f32, inner: Option<([f32; 4], [f32; 4])>, now: std::time::Instant) {
    let [x0, y0, x1, y1] = rect;
    if alpha <= 0.0 || x1 - x0 < 4.0 || y1 - y0 < 4.0 {
        return;
    }
    // (its pulse: the card's inner line's, which the menu pulses; on the gear steps (no card) the same slow beat)
    let t = now.duration_since(*PULSE_START.get_or_init(std::time::Instant::now)).as_secs_f32();
    let pulse = 0.7 + 0.3 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.6).sin());
    let colour = inner.map_or([0.25, 0.85, 0.6, 1.0], |i| i.1);
    let ring = |out: &mut Vec<Command>, d: f32, w: f32, c: [f32; 4]| {
        let (a0, b0, a1, b1) = (x0 - d - w, y0 - d - w, x1 + d + w, y1 + d + w);
        let (i0, j0, i1, j1) = (x0 - d, y0 - d, x1 + d, y1 + d);
        let mut mesh = Mesh::default();
        for [qx0, qy0, qx1, qy1] in [[a0, b0, a1, j0], [a0, j1, a1, b1], [a0, j0, i0, j1], [i1, j0, a1, j1]] {
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend([[qx0, qy0], [qx1, qy0], [qx1, qy1], [qx0, qy1]]);
            mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        out.push(Command::Draw(DrawItem {
            mesh: Arc::new(mesh),
            paint: Paint::Solid(c),
            transform: Mat2d::IDENTITY,
            cxform: Cxform::IDENTITY,
            blend: BlendMode::Normal,
        }));
    };
    // the glow outside it: fading out smoothly, pulsing
    let glow = 14;
    for i in (0..glow).rev() {
        let f = 1.0 - (i as f32 + 0.5) / glow as f32;
        let a = 0.5 * f * f * alpha * pulse;
        ring(out, 3.0 + i as f32, 1.0, [CARD_GLOW[0], CARD_GLOW[1], CARD_GLOW[2], a]);
    }
    // the line: the card's inner frame's (three pixels), its brightness pulsing
    ring(out, 0.0, 3.0, [colour[0], colour[1], colour[2], colour[3] * alpha * pulse]);
}

static PULSE_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// The frame's long move under way (where from, how long it's been going).
static FRAME_MOVE: std::sync::Mutex<Option<([f32; 4], f32)>> = std::sync::Mutex::new(None);
/// Since when the frame has been kept, not drawn (the screen changing).
static KEPT_SINCE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// The full list's grids: the window rect each one's cells are drawn within (none but over the list's width).
static GRID_MASKS: std::sync::Mutex<Vec<(InstanceId, [f32; 4])>> = std::sync::Mutex::new(Vec::new());
/// The full list's grids as last laid out: their cells' places (window x, as the list has them) and how far behind
/// they're drawn (a scroll easing in), and when.
#[allow(clippy::type_complexity)]
static GRID_SCROLL: std::sync::Mutex<Vec<(InstanceId, Vec<(InstanceId, f32)>, f32, std::time::Instant)>> =
    std::sync::Mutex::new(Vec::new());
/// How quickly a scroll of the full list eases in (per second: about a fifth of a second).
const GRID_SCROLL_RATE: f32 = 14.0;
/// Drawing a grid inside its mask (so it isn't masked again).
static GRID_MASKING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn full_grid_mask(id: InstanceId) -> Option<[f32; 4]> {
    if !ssx_core::rider_showcase::enabled() || !ssx_core::rider_showcase::full_list_view() {
        return None;
    }
    GRID_MASKS.lock().ok()?.iter().find(|g| g.0 == id).map(|g| g.1)
}

/// Back from the gear steps, the rider list's other cards (and the gear and perks box) fade in over this long
/// (seconds).
const LIST_FADE_IN_SECS: f32 = 1.0;
/// The gear and perks box's fade (back from the gear steps, with the list's other cards): its alpha's bits.
static BIO_FADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);
static BIO_FADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The menus' full-screen dimmer (dimmer.gfx's sheet under `mcDimmer`, not the side one).
fn full_dimmer(lib: &Library, player: &Player, inst: &Instance) -> bool {
    inst.character.as_ref().is_some_and(|c| c.id != 2)
        && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcDimmer")
        && lib.movie(inst.movie).path.to_ascii_lowercase().rsplit(['/', '\\']).next() == Some("dimmer.gfx")
}
static DIMMER_KEEPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The dimmer's draws as last drawn, when, and (once it's gone) since when it's fading out.
#[allow(clippy::type_complexity)]
static DIMMER_LAST: std::sync::Mutex<Option<(Vec<Command>, std::time::Instant, Option<std::time::Instant>)>> =
    std::sync::Mutex::new(None);
/// How long the dimmer takes to fade out once gone (seconds).
const DIMMER_FADE_SECS: f32 = 0.6;

/// The dimmer gone this frame (drawn last frame, not this one): its last draws again, fading out.
fn dimmer_fade(out: &mut Vec<Command>, frame_start: std::time::Instant) {
    let Ok(mut d) = DIMMER_LAST.lock() else { return };
    let Some((draws, seen, gone)) = d.as_mut() else { return };
    if *seen >= frame_start {
        return;
    }
    let since = *gone.get_or_insert_with(std::time::Instant::now);
    let p = since.elapsed().as_secs_f32() / DIMMER_FADE_SECS;
    if p >= 1.0 {
        *d = None;
        return;
    }
    let f = 1.0 - p * p * (3.0 - 2.0 * p);
    for c in draws.iter() {
        match c {
            Command::Draw(item) => {
                let mut item = item.clone();
                item.cxform.mult[3] *= f;
                out.push(Command::Draw(item));
            }
            other => out.push(other.clone()),
        }
    }
}

/// When the full list (View More) was last up (the gear steps' menu comes on again after it).
static LEFT_FULL_LIST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// The rider's view's frame is shrinking back into the highlighted card (the card's own inner frame waits).
fn frame_returning() -> bool {
    FRAME_NOW.lock().is_ok_and(|f| f.is_some()) && GEAR_FRAME.lock().is_ok_and(|g| g.is_none())
}

fn gear_layout(lib: &Library, player: &Player, stage: &Mat2d) {
    static ROOT: std::sync::Mutex<Option<InstanceId>> = std::sync::Mutex::new(None);
    if let Ok(mut f) = GEAR_FRAME.lock() {
        *f = None;
    }
    let layout = (|| {
        if !ssx_core::rider_showcase::enabled() {
            return None;
        }
        // (off the gear steps (on the way back to the character select, say), the gear steps' menu (Start Run and
        // the options) and Survival box not shown: the menu slides it out from its own place, off to the left of where it's seen)
        if !ssx_core::rider_showcase::gear_view() {
            let root = ROOT.lock().ok().and_then(|r| *r)?;
            let i = player.get(root).filter(|i| i.name == "mcEquipmentRoot")?;
            let InstanceKind::Sprite(s) = &i.kind else { return None };
            // (and the Survival box, the same)
            let hidden: Vec<InstanceId> = s
                .children
                .values()
                .copied()
                .filter(|&k| player.get(k).is_some_and(|k| k.name == "mcSlots" || k.name == "mcSurvival"))
                .collect();
            return (!hidden.is_empty()).then_some(GearLayout { moved: Vec::new(), hidden });
        }
        if ssx_core::rider_showcase::full_list_view() {
            if let Ok(mut t) = LEFT_FULL_LIST.lock() {
                *t = Some(std::time::Instant::now());
            }
            return full_list_layout(lib, player, stage);
        }
        let mut root = ROOT.lock().ok()?;
        if root.is_none_or(|r| player.get(r).is_none_or(|i| i.name != "mcEquipmentRoot")) {
            *root = None;
            let mut stack: Vec<(InstanceId, usize)> = player.levels.values().map(|&r| (r, 0)).collect();
            while let Some((id, depth)) = stack.pop() {
                let Some(i) = player.get(id) else { continue };
                if i.name == "mcEquipmentRoot" {
                    *root = Some(id);
                    break;
                }
                if depth < 12
                    && let InstanceKind::Sprite(s) = &i.kind
                {
                    stack.extend(s.children.values().map(|&c| (c, depth + 1)));
                }
            }
        }
        let root = (*root)?;
        let child = |of: InstanceId, name: &str| match player.get(of).map(|i| &i.kind) {
            Some(InstanceKind::Sprite(s)) => {
                s.children.values().copied().find(|&k| player.get(k).is_some_and(|k| k.name == name))
            }
            _ => None,
        };
        let to_window = |id: InstanceId| {
            let mut m = Mat2d::IDENTITY;
            let mut at = Some(id);
            while let Some(i) = at.and_then(|a| player.get(a)) {
                m = i.matrix.mul(&m);
                at = i.parent;
            }
            stage.mul(&m)
        };
        let win = |id: InstanceId| {
            let b = player.local_bounds(lib, id);
            (!b.is_empty()).then(|| b.transform(&to_window(id)))
        };
        let slots = child(root, "mcSlots")?;
        let advance = child(slots, "mcAdvance")?;
        let menu = child(slots, "mcCategoryMenu")?;
        let survival = child(root, "mcSurvival")?;
        let survival_backer = child(survival, "mcBacker").unwrap_or(survival);
        let cell = child(child(root, "mcSelectedRider")?, "mcCell")?;
        let level = child(cell, "mcLevel")?;
        let bar = child(cell, "mcLevelProgress")?;
        let (card, adv, men, surv, lv, xp) =
            (win(cell)?, win(advance)?, win(menu)?, win(survival_backer)?, win(level)?, win(bar)?);
        let u = card.y1 - card.y0;
        let gap = u * GEAR_GAP;
        // (the view's top level with Start Run's box (not its glow))
        let start_top = child(advance, "mcBackerContainer").and_then(|b| win(b)).map_or(adv.y0, |b| b.y0);
        // (Start Run's and the options' boxes grow a little while highlighted: the places worked out from them (the
        // view's top, the Survival box's) are kept as first found for this size of screen, so nothing moves as the
        // highlight does)
        static ANCHORS: std::sync::Mutex<Option<((i32, i32), f32, Option<f32>)>> = std::sync::Mutex::new(None);
        let key = (card.x0.round() as i32, (card.y1 - card.y0).round() as i32);
        let kept = ANCHORS.lock().ok().and_then(|a| *a).filter(|a| a.0 == key);
        let start_top = kept.map_or(start_top, |a| a.1);
        let (x0, y0) = (card.x0, start_top + GEAR_DROP_PX);
        let x1 = x0 + u * GEAR_VIEW_WIDTH;
        let column = x1 + gap;
        let bottom = surv.y1;
        let xp_h = xp.y1 - xp.y0;
        let kids = |of: InstanceId| match player.get(of).map(|i| &i.kind) {
            Some(InstanceKind::Sprite(s)) => s.children.values().copied().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let menu_dx = column - men.x0;
        let mut moved = vec![(advance, column - adv.x0, 0.0, 1.0, 1.0), (menu, menu_dx, 0.0, 1.0, 1.0)];
        let mut hidden: Vec<InstanceId> = Vec::new();
        // The options: each one's own cell (its backing, its button).
        let option_part = |clip: InstanceId, name: &str| child(child(child(clip, "mcCell")?, "mcCell")?, name);
        let mut widths: Vec<f32> = Vec::new();
        let mut option_icons: Vec<(f32, InstanceId)> = Vec::new();
        let mut options_bottom = f32::MIN;
        for clip in kids(menu) {
            let shown = player.get(clip).is_some_and(|i| i.visible);
            if let Some(b) = option_part(clip, "mcBackerContainer").and_then(|b| win(b))
                && b.x1 - b.x0 > 1.0
            {
                widths.push(b.x1 - b.x0);
                if let Some(icons) = option_part(clip, "mcSlots") {
                    option_icons.push((b.x1 - b.x0, icons));
                }
                if shown {
                    options_bottom = options_bottom.max(b.y1);
                }
            }
            // (no A button at the options' right ends)
            if let Some(btn) = option_part(clip, "mcButton") {
                hidden.push(btn);
            }
        }
        // (the Survival box just under the options (the last, Mods), not down by the view's bottom)
        let options_bottom = match kept.and_then(|a| a.2) {
            Some(b) => Some(b),
            None => (options_bottom > f32::MIN).then_some(options_bottom),
        };
        if let Ok(mut a) = ANCHORS.lock()
            && kept.is_none_or(|k| k.2.is_none())
        {
            *a = Some((key, start_top, options_bottom));
        }
        let surv_dy = options_bottom.map_or(0.0, |b| (b + gap * 0.6 - surv.y0).min(0.0)) - SURVIVAL_UP_PX;
        let surv_dx = column - surv.x0 + SURVIVAL_RIGHT_PX;
        moved.push((survival, surv_dx, surv_dy, 1.0, 1.0));
        // The selection highlight goes with the options. (While the menu comes on (back from the full list, say) it waits
        // off to the left, where the options used to be, till the menu puts it on its option: not shown till then.)
        let mut highlight_waiting = false;
        if let Some(h) = child(slots, "mcHighlight") {
            // (on any of an option's slots: the Gear and Mods options' last slots are well right of the menu's left)
            let on_options = win(h).is_none_or(|b| {
                let mid = (b.x0 + b.x1) * 0.5;
                mid > men.x0 - u * 0.25 && mid < men.x1.max(men.x0 + u * 0.6) + u * 0.5
            });
            if on_options {
                moved.push((h, menu_dx, 0.0, 1.0, 1.0));
            } else {
                highlight_waiting = true;
                hidden.push(h);
            }
        }
        // (and meanwhile, just back from the full list, the options' pictures come on bright before the menu dims those
        // not chosen: those not chosen not shown till it has)
        let from_full_list = LEFT_FULL_LIST
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_some_and(|t| t.elapsed().as_secs_f32() < 1.5);
        if highlight_waiting && from_full_list {
            let wide = option_icons.iter().map(|o| o.0).fold(0.0f32, f32::max);
            let narrow = option_icons.iter().map(|o| o.0).fold(f32::MAX, f32::min);
            for &(w, icons) in &option_icons {
                if !(wide > narrow * 1.02 && w >= wide - 0.5) {
                    hidden.push(icons);
                }
            }
        }
        // Start Run as wide as an option: as a plain option while an option is highlighted, as a highlighted one (they
        // grow a little) while it is.
        static LIT_GROW: std::sync::Mutex<f32> = std::sync::Mutex::new(1.05);
        let narrow = widths.iter().copied().fold(f32::MAX, f32::min);
        let wide = widths.iter().copied().fold(0.0f32, f32::max);
        if !widths.is_empty()
            && let Some(bk) = child(advance, "mcBackerContainer")
            && let Some(b) = win(bk)
            && b.x1 - b.x0 > 1.0
        {
            let option_lit = wide > narrow * 1.02;
            let grow = match LIT_GROW.lock() {
                Ok(mut g) => {
                    if option_lit {
                        *g = wide / narrow;
                    }
                    *g
                }
                Err(_) => 1.05,
            };
            let w = if option_lit { narrow } else { narrow * grow };
            moved.push((bk, 0.0, 0.0, w / (b.x1 - b.x0), 1.0));
        }
        if let Some(btn) = child(advance, "mcButton") {
            hidden.push(btn);
        }
        // The pop-ups (equipment panel, item info) and the hovered item's description beside the options.
        if let Some(p) = child(root, "mcSelectionPanelContainer") {
            moved.push((p, menu_dx, 0.0, 1.0, 1.0));
        }
        if let Some(p) = child(root, "mcBioPrompt") {
            moved.push((p, surv_dx, surv_dy, 1.0, 1.0));
            // (no Y "Attribute Descriptions" prompt)
            if let Some(prompt) = child(p, "mcPrompt") {
                hidden.push(prompt);
            }
        }
        // (nor the button bar under the XP bar: its Y "Attribute Descriptions" on the suits' and boards' lists; Y marks
        // favourites there)
        if let Some(bar) = child(root, "mcButtonCalloutBar") {
            hidden.push(bar);
        }
        // The blue box left of the options (where the model is now): an unnamed tall narrow sprite there.
        for k in kids(root) {
            if player.get(k).is_some_and(|i| i.name.starts_with("instance"))
                && let Some(b) = win(k)
                && b.x0 < card.x1
                && b.x1 - b.x0 < u * 0.5
                && b.y1 - b.y0 > u * 0.8
            {
                hidden.push(k);
            }
        }
        // The level left of the XP bar, on a row under the view (its bottom the Survival box's), the two centred under
        // the view.
        let lv_w = lv.x1 - lv.x0;
        let row_h = (lv.y1 - lv.y0).max(xp_h);
        let row_mid = bottom - row_h * 0.5 + GEAR_DROP_PX;
        let bar_w = (x1 - x0) * 0.6;
        // (the bar centred under the view, the level left of it)
        let bar_x0 = (x0 + x1) * 0.5 - bar_w * 0.5;
        let row_x0 = bar_x0 - gap * 0.3 - lv_w;
        moved.push((
            bar,
            bar_x0 - xp.x0,
            row_mid - (xp.y0 + xp.y1) * 0.5,
            bar_w / (xp.x1 - xp.x0).max(1.0),
            1.0,
        ));
        moved.push((level, row_x0 - lv.x0, row_mid - (lv.y0 + lv.y1) * 0.5, 1.0, 1.0));
        let view_bottom = bottom - row_h - gap * 0.3 + GEAR_DROP_PX;
        hidden.extend(kids(cell).into_iter().filter(|&k| k != level && k != bar));
        // (just back from the full list the menu's parts slide back on: the view stays where it was meanwhile)
        let mut view = [x0, y0, x1, view_bottom];
        if from_full_list
            && let Some(kept) = GEAR_VIEW_RECT.lock().ok().and_then(|v| *v)
        {
            view = kept;
        }
        ssx_core::rider_showcase::found_slot(view);
        if let Ok(mut v) = GEAR_VIEW_RECT.lock() {
            *v = Some(view);
        }
        set_gear_frame(view, shown_alpha(player, root));
        Some(GearLayout { moved, hidden })
    })();
    // (where the mouse finds the parts moved: each moved part's drawn rectangle and how far it was moved, in the
    // drawing's pixels; the parts hidden)
    let map = layout.as_ref().map(|l| {
        let moved = l
            .moved
            .iter()
            .filter_map(|&(id, dx, dy, kx, _)| {
                let b = player.local_bounds(lib, id);
                if b.is_empty() {
                    return None;
                }
                let b = b.transform(&stage.mul(&player.world_matrix(id)));
                let x1 = b.x0 + (b.x1 - b.x0) * kx;
                Some((id, RectF { x0: b.x0 + dx, y0: b.y0 + dy, x1: x1 + dx, y1: b.y1 + dy }, dx, dy, kx, b.x0))
            })
            .collect();
        PointerMap { stage: *stage, moved, hidden: l.hidden.clone() }
    });
    if let Ok(mut m) = POINTER_MAP.lock() {
        *m = map;
    }
    if let Ok(mut g) = GEAR.lock() {
        *g = layout;
    }
}

/// The gear steps' parts as the mouse finds them (the 3D Rider Select mod moves and hides some as they're drawn).
struct PointerMap {
    stage: Mat2d,
    /// Each moved part: where it's drawn (the drawing's pixels), how far it was moved, how much wider, and its left
    /// edge before.
    moved: Vec<(InstanceId, RectF, f32, f32, f32, f32)>,
    hidden: Vec<InstanceId>,
}
static POINTER_MAP: std::sync::Mutex<Option<PointerMap>> = std::sync::Mutex::new(None);

/// A point of the stage (twips) where the mouse is, as the menus have it: on a part the gear steps draw moved, the
/// point the part has there before it's moved.
pub fn pointer_place(at: (f32, f32)) -> (f32, f32) {
    let Ok(m) = POINTER_MAP.lock() else { return at };
    let Some(m) = m.as_ref() else { return at };
    let (x, y) = m.stage.apply(at.0, at.1);
    let hit = m
        .moved
        .iter()
        .filter(|(_, r, ..)| (r.x0..=r.x1).contains(&x) && (r.y0..=r.y1).contains(&y))
        .min_by(|a, b| ((a.1.x1 - a.1.x0) * (a.1.y1 - a.1.y0)).total_cmp(&((b.1.x1 - b.1.x0) * (b.1.y1 - b.1.y0))));
    let Some(&(_, _, dx, dy, kx, left)) = hit else { return at };
    let ux = left + (x - dx - left) / kx.max(1e-3);
    let uy = y - dy;
    match m.stage.inverse() {
        Some(inv) => inv.apply(ux, uy),
        None => at,
    }
}

/// A point of the stage (twips) where the mouse is, as the menus have it for one part (`id`): moved back by however
/// far the part (or the part it's in) is drawn moved on the gear steps.
pub fn pointer_place_for(player: &Player, id: InstanceId, at: (f32, f32)) -> (f32, f32) {
    let Ok(m) = POINTER_MAP.lock() else { return at };
    let Some(m) = m.as_ref() else { return at };
    let mut cur = Some(id);
    let mut n = 0;
    while let Some(i) = cur {
        if let Some(&(_, _, dx, dy, kx, left)) = m.moved.iter().find(|e| e.0 == i) {
            let (x, y) = m.stage.apply(at.0, at.1);
            let ux = left + (x - dx - left) / kx.max(1e-3);
            return match m.stage.inverse() {
                Some(inv) => inv.apply(ux, y - dy),
                None => at,
            };
        }
        cur = player.get(i).and_then(|x| x.parent);
        n += 1;
        if n > 64 {
            break;
        }
    }
    at
}

/// The entries the mouse asks about (the menus' cells), and where each was drawn: this frame's, last frame's (the
/// drawing's pixels), and the stage's place in the drawing.
static POINTER_WATCH: std::sync::Mutex<std::collections::BTreeSet<InstanceId>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());
#[allow(clippy::type_complexity)]
static POINTER_DRAWN: std::sync::Mutex<(
    std::collections::BTreeMap<InstanceId, RectF>,
    std::collections::BTreeMap<InstanceId, RectF>,
    Option<Mat2d>,
)> = std::sync::Mutex::new((std::collections::BTreeMap::new(), std::collections::BTreeMap::new(), None));

/// The entries the mouse asks about from now on.
pub fn pointer_watch(ids: impl IntoIterator<Item = InstanceId>) {
    if let Ok(mut w) = POINTER_WATCH.lock() {
        *w = ids.into_iter().collect();
    }
}

/// More entries the mouse asks about (after [`pointer_watch`], till its next call): the equipment options' slots.
pub fn pointer_watch_more(ids: impl IntoIterator<Item = InstanceId>) {
    if let Ok(mut w) = POINTER_WATCH.lock() {
        w.extend(ids);
    }
}

/// Where an entry the mouse asks about was last drawn, and where the mouse (a point of the stage, twips) is in the
/// same drawing: `None` when it wasn't seen drawn.
pub fn pointer_drawn(id: InstanceId, at: (f32, f32)) -> Option<(RectF, (f32, f32))> {
    let d = POINTER_DRAWN.lock().ok()?;
    let r = *d.1.get(&id)?;
    Some((r, d.2?.apply(at.0, at.1)))
}

/// Whether the gear steps' layout is up (parts drawn moved: the mouse goes by where they're drawn).
pub fn pointer_map_on() -> bool {
    POINTER_MAP.lock().is_ok_and(|m| m.is_some())
}

/// Whether the mouse (a point of the stage, twips) is on an entry where it was last drawn; `None` when it wasn't
/// seen drawn.
pub fn pointer_on_drawn(id: InstanceId, at: (f32, f32)) -> Option<bool> {
    let d = POINTER_DRAWN.lock().ok()?;
    let r = d.1.get(&id)?;
    let (x, y) = d.2?.apply(at.0, at.1);
    Some((r.x0..=r.x1).contains(&x) && (r.y0..=r.y1).contains(&y))
}

/// How much wider a part is drawn on the gear steps (1 when it isn't).
pub fn pointer_stretch(id: InstanceId) -> f32 {
    POINTER_MAP
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.moved.iter().find(|e| e.0 == id).map(|e| e.4)))
        .unwrap_or(1.0)
}

/// Whether a part (or one it's in) is hidden on the gear steps (not for the mouse).
pub fn pointer_hidden(player: &Player, id: InstanceId) -> bool {
    let Ok(m) = POINTER_MAP.lock() else { return false };
    let Some(m) = m.as_ref() else { return false };
    let mut at = Some(id);
    let mut n = 0;
    while let Some(i) = at {
        if m.hidden.contains(&i) {
            return true;
        }
        at = player.get(i).and_then(|x| x.parent);
        n += 1;
        if n > 64 {
            break;
        }
    }
    false
}

/// A favourite suit's item (the 3D Rider Select mod): a yellow-orange star at its picture's top-left corner. The item
/// is known by its place: its cell of the gear list (`mcGearMenu`) or of the full list (any other grid but the
/// store's).
fn favourite_star(lib: &Library, player: &Player, id: InstanceId, here: &Inherited, out: &mut Vec<Command>) {
    let (gear, full, highlighted) = ssx_core::rider_showcase::favourite_cells();
    if !gear.iter().chain(&full).any(|&f| f) {
        return;
    }
    // (the item's cell: `clipInstance_<n>` a few levels up, in its grid)
    let mut at = player.get(id).and_then(|i| i.parent);
    let mut cell = None;
    for _ in 0..6 {
        let Some(a) = at else { break };
        let Some(i) = player.get(a) else { break };
        if let Some(n) = i.name.strip_prefix("clipInstance_").and_then(|n| n.parse::<usize>().ok()) {
            cell = i.parent.map(|g| (n, a, g));
            break;
        }
        at = i.parent;
    }
    let Some((n, clip, grid)) = cell else { return };
    let grid_name = player.get(grid).map(|g| g.name.as_str()).unwrap_or("");
    let favourite = match grid_name {
        "mcStoreMenu" => false,
        "mcGearMenu" => gear.get(n).copied().unwrap_or(false),
        // (the full list's grid scrolls, its cells used again for other items: a cell's item is known by where it is
        // from the highlight (on the highlighted item), its cells in columns, the items down each column in turn)
        _ => full_list_item(lib, player, grid, clip, highlighted)
            .and_then(|i| full.get(i).copied())
            .unwrap_or(false),
    };
    if !favourite {
        return;
    }
    let b = player.local_bounds(lib, id);
    if b.is_empty() {
        return;
    }
    let w = b.transform(&here.xf);
    let r = (w.y1 - w.y0).abs().min((w.x1 - w.x0).abs()) * 0.14;
    if r < 2.0 {
        return;
    }
    let (cx, cy) = (w.x0 + r * 1.25, w.y0 + r * 1.25);
    let star = |radius: f32| {
        let mut mesh = Mesh::default();
        mesh.vertices.push([cx, cy]);
        for i in 0..10 {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let d = if i % 2 == 0 { radius } else { radius * 0.45 };
            mesh.vertices.push([cx + a.cos() * d, cy + a.sin() * d]);
        }
        for i in 0..10u32 {
            mesh.indices.extend([0, 1 + i, 1 + (i + 1) % 10]);
        }
        Arc::new(mesh)
    };
    for (radius, colour) in [(r * 1.18, [0.55, 0.22, 0.0, 0.9]), (r, [1.0, 0.62, 0.05, 1.0]), (r * 0.6, [1.0, 0.86, 0.25, 1.0])] {
        out.push(Command::Draw(DrawItem {
            mesh: star(radius),
            paint: Paint::Solid(colour),
            transform: Mat2d::IDENTITY,
            cxform: here.cx,
            blend: BlendMode::Normal,
        }));
    }
}

/// The gear view's place as last laid out on the equipment screen (window pixels): the full list's view takes it too.
static GEAR_VIEW_RECT: std::sync::Mutex<Option<[f32; 4]>> = std::sync::Mutex::new(None);

/// The equipment's full list (View More), on the gear steps: the chosen rider's view where the equipment screen has it
/// (left), the screen's contents moved right of it when they'd be under it.
fn full_list_layout(lib: &Library, player: &Player, stage: &Mat2d) -> Option<GearLayout> {
    let file = |i: &Instance| lib.movie(i.movie).path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    let mut screen = None;
    let mut stack: Vec<(InstanceId, usize)> = player.levels.values().map(|&r| (r, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(i) = player.get(id) else { continue };
        if i.name == "ssx_screen" {
            screen = Some(id);
            break;
        }
        if depth < 10
            && let InstanceKind::Sprite(s) = &i.kind
        {
            stack.extend(s.children.values().map(|&c| (c, depth + 1)));
        }
    }
    let screen = screen?;
    let to_window = |id: InstanceId| {
        let mut m = Mat2d::IDENTITY;
        let mut at = Some(id);
        while let Some(i) = at.and_then(|a| player.get(a)) {
            m = i.matrix.mul(&m);
            at = i.parent;
        }
        stage.mul(&m)
    };
    // (the screen's own movies shown under it: the full list's)
    let InstanceKind::Sprite(s) = &player.get(screen)?.kind else { return None };
    let mut parts: Vec<(InstanceId, RectF)> = Vec::new();
    for &c in s.children.values() {
        let Some(i) = player.get(c) else { continue };
        if !i.visible || file(i) == "application.gfx" {
            continue;
        }
        let b = player.local_bounds(lib, c);
        if !b.is_empty() {
            parts.push((c, b.transform(&to_window(c))));
        }
    }
    if parts.is_empty() {
        return None;
    }
    // (in the list's screen: its header (the list's left edge as seen: the grid's own bounds take in its cells
    // scrolled out of sight) and its button bar (Y's "Attribute Descriptions": Y marks favourites here))
    let find = |name: &str| {
        let mut stack: Vec<(InstanceId, usize)> = parts.iter().map(|p| (p.0, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            let Some(i) = player.get(id) else { continue };
            if i.name == name {
                return Some(id);
            }
            if depth < 6
                && let InstanceKind::Sprite(s) = &i.kind
            {
                stack.extend(s.children.values().map(|&c| (c, depth + 1)));
            }
        }
        None
    };
    let header = find("mcHeaderContainer").and_then(|h| {
        let b = player.local_bounds(lib, h);
        (!b.is_empty()).then(|| b.transform(&to_window(h)).x0)
    });
    let left = header.unwrap_or_else(|| parts.iter().map(|p| p.1.x0).fold(f32::MAX, f32::min));
    let hidden: Vec<InstanceId> = find("mcButtonCalloutBar").into_iter().collect();
    let (top, bottom) = (
        parts.iter().map(|p| p.1.y0).fold(f32::MAX, f32::min),
        parts.iter().map(|p| p.1.y1).fold(f32::MIN, f32::max),
    );
    let view = GEAR_VIEW_RECT.lock().ok().and_then(|v| *v).unwrap_or_else(|| {
        let h = (bottom - top) * 0.75;
        let w = h * 0.56;
        let y0 = top + (bottom - top - h) * 0.5;
        [left, y0, left + w, y0 + h]
    });
    let gap = (view[2] - view[0]) * 0.04;
    let dx = view[2] + gap - left;
    let mut moved: Vec<(InstanceId, f32, f32, f32, f32)> = parts.iter().map(|&(id, _)| (id, dx, 0.0, 1.0, 1.0)).collect();
    // The list's grids (the 3D Rider Select mod): their cells scrolled off the left not drawn (only those over the
    // list's width: the header's), and a scroll slid smoothly (the cells drawn where they were, easing to where the
    // list has them now).
    let header_right = find("mcHeaderContainer").and_then(|h| {
        let b = player.local_bounds(lib, h);
        (!b.is_empty()).then(|| b.transform(&to_window(h)).x1)
    });
    let mut grids: Vec<(InstanceId, Vec<(InstanceId, f32)>, [f32; 4])> = Vec::new();
    let mut stack: Vec<(InstanceId, usize)> = parts.iter().map(|p| (p.0, 0)).collect();
    while let Some((id, depth)) = stack.pop() {
        let Some(InstanceKind::Sprite(s)) = player.get(id).map(|i| &i.kind) else { continue };
        let cells: Vec<InstanceId> = s
            .children
            .values()
            .copied()
            .filter(|&k| player.get(k).is_some_and(|i| i.name.starts_with("clipInstance_")))
            .collect();
        if cells.is_empty() {
            if depth < 8 {
                stack.extend(s.children.values().map(|&c| (c, depth + 1)));
            }
            continue;
        }
        let xs: Vec<(InstanceId, f32)> = cells.iter().map(|&c| (c, to_window(c).tx)).collect();
        let b = player.local_bounds(lib, id);
        let (gy0, gy1) = if b.is_empty() { (top, bottom) } else { let w = b.transform(&to_window(id)); (w.y0, w.y1) };
        let pad = (view[2] - view[0]) * 0.02;
        let mask = [left + dx - pad, gy0 - pad * 4.0, header_right.map_or(f32::MAX / 4.0, |r| r + dx + pad), gy1 + pad * 4.0];
        grids.push((id, xs, mask));
    }
    if let (Ok(mut scroll), Ok(mut masks)) = (GRID_SCROLL.lock(), GRID_MASKS.lock()) {
        let now = std::time::Instant::now();
        masks.clear();
        let mut next = Vec::new();
        for (grid, xs, mask) in grids {
            if header.is_some() {
                masks.push((grid, mask));
            }
            let (mut lag, dt) = match scroll.iter().find(|g| g.0 == grid) {
                Some((_, last, lag, at)) => {
                    // (a scroll: most of the cells moved by the same step)
                    let steps: Vec<f32> =
                        xs.iter().filter_map(|(c, x)| last.iter().find(|l| l.0 == *c).map(|l| x - l.1)).collect();
                    let mut lag = *lag;
                    if let Some(&step) = steps.iter().filter(|d| d.abs() > 1.0).max_by_key(|&&d| {
                        steps.iter().filter(|&&e| (e - d).abs() < 1.0).count()
                    }) && steps.iter().filter(|&&e| (e - step).abs() < 1.0).count() * 2 >= steps.len()
                    {
                        lag -= step;
                    }
                    (lag, now.duration_since(*at).as_secs_f32().min(0.1))
                }
                None => (0.0, 0.0),
            };
            lag *= (-dt * GRID_SCROLL_RATE).exp();
            if lag.abs() < 0.5 {
                lag = 0.0;
            }
            if lag != 0.0 {
                moved.extend(xs.iter().map(|&(c, _)| (c, lag, 0.0, 1.0, 1.0)));
            }
            next.push((grid, xs, lag, now));
        }
        *scroll = next;
    }
    ssx_core::rider_showcase::found_slot(view);
    set_gear_frame(view, parts.first().map_or(1.0, |p| shown_alpha(player, p.0)));
    Some(GearLayout { moved, hidden })
}

/// The full list's item a cell of its grid shows: worked out from where the cell is from the grid's highlight (on
/// the highlighted item, `highlighted` in the list).
fn full_list_item(lib: &Library, player: &Player, grid: InstanceId, clip: InstanceId, highlighted: Option<usize>) -> Option<usize> {
    let highlighted = i64::try_from(highlighted?).ok()?;
    let InstanceKind::Sprite(g) = &player.get(grid)?.kind else { return None };
    let named = |name: &str| g.children.values().copied().find(|&k| player.get(k).is_some_and(|i| i.name == name));
    // (a child's centre in the grid's space)
    let centre = |k: InstanceId| {
        let i = player.get(k)?;
        let b = player.local_bounds(lib, k);
        if b.is_empty() {
            return None;
        }
        let b = b.transform(&i.matrix);
        Some(((b.x0 + b.x1) * 0.5, (b.y0 + b.y1) * 0.5))
    };
    let hl = centre(named("mcHighlight")?)?;
    let c0 = centre(named("clipInstance_0")?)?;
    let c1 = centre(named("clipInstance_1")?)?;
    let row_pitch = (c1.1 - c0.1).abs();
    // (a column's height: the cells under the first one's, in its column)
    let column: Vec<(f32, f32)> = g
        .children
        .values()
        .filter(|&&k| player.get(k).is_some_and(|i| i.name.starts_with("clipInstance_")))
        .filter_map(|&k| centre(k))
        .collect();
    let rows = column.iter().filter(|c| (c.0 - c0.0).abs() < row_pitch * 0.3).count().max(1) as i64;
    let col_pitch = column
        .iter()
        .map(|c| (c.0 - c0.0).abs())
        .filter(|&d| d > row_pitch * 0.3)
        .fold(f32::MAX, f32::min);
    if row_pitch < 1.0 || !col_pitch.is_finite() || col_pitch == f32::MAX {
        return None;
    }
    let c = centre(clip)?;
    let dcol = ((c.0 - hl.0) / col_pitch).round() as i64;
    let drow = ((c.1 - hl.1) / row_pitch).round() as i64;
    usize::try_from(highlighted + dcol * rows + drow).ok()
}

fn gear_hidden(id: InstanceId) -> bool {
    GEAR.lock().is_ok_and(|g| g.as_ref().is_some_and(|g| g.hidden.contains(&id)))
}

fn gear_moved(lib: &Library, player: &Player, id: InstanceId, parent: &Mat2d, matrix: Mat2d) -> Mat2d {
    let Some((_, dx, dy, kx, ky)) =
        GEAR.lock().ok().and_then(|g| g.as_ref()?.moved.iter().find(|m| m.0 == id).copied())
    else {
        return matrix;
    };
    let (sx, sy) = (parent.a.abs().max(1e-6), parent.d.abs().max(1e-6));
    let m = if (kx - 1.0).abs() > 1e-4 || (ky - 1.0).abs() > 1e-4 {
        let b = player.local_bounds(lib, id);
        if b.is_empty() {
            matrix
        } else {
            let at = b.transform(&matrix);
            stretched(matrix, at.x0, at.y0, kx, ky)
        }
    } else {
        matrix
    };
    Mat2d::translate(dx / sx, dy / sy).mul(&m)
}

/// Drawing a plain card's part inside its straight cut (so it isn't cut again).
static PLAIN_CUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A plain card of the rider list: its backing's bounds, where its straight bottom goes (a little below its XP bar)
/// and its XP bar's bounds, in its cell's space.
fn plain_card_cut(lib: &Library, player: &Player, id: InstanceId, inst: &Instance) -> Option<(RectF, f32, RectF)> {
    let cell = player.get(inst.parent?)?;
    let InstanceKind::Sprite(s) = &cell.kind else { return None };
    let part = |name: &str| {
        s.children.values().find_map(|&k| {
            let i = player.get(k).filter(|i| i.name == name)?;
            let b = player.local_bounds(lib, k);
            (!b.is_empty()).then(|| b.transform(&i.matrix))
        })
    };
    let bar = part("mcLevelProgress")?;
    let b = match part("mcBacker") {
        Some(b) => b,
        None => {
            let b = player.local_bounds(lib, id);
            if b.is_empty() {
                return None;
            }
            b.transform(&inst.matrix)
        }
    };
    // (just under the bar, above where the frame's sides start to bend in to the pointed bottom)
    let cut = bar.y1 + (bar.y1 - bar.y0).max(1.0) * 0.15;
    (cut < b.y1).then_some((b, cut, bar))
}

fn in_grown_cell(inst: &Instance) -> bool {
    !ghosting() && GROW.lock().ok().and_then(|g| *g).is_some_and(|g| inst.parent == Some(g.cell))
}

/// The rider list's cards also drawn as they were (plain cards) sliding on from their last places: the newly
/// highlighted one under the highlighted card, or off the list's end: their clips, how far from their places (menu
/// units), and how faded.
static GHOST: std::sync::Mutex<Vec<(String, f32, f32)>> = std::sync::Mutex::new(Vec::new());
/// The rider list's highlighted card's clip: the copies are drawn just under it.
static GHOST_TOP: std::sync::Mutex<Option<InstanceId>> = std::sync::Mutex::new(None);
/// Each rider's card as last drawn as a plain card of the list: the menu's place (its transform), the card's own,
/// and its draws. A copy of a card is these, moved and faded.
static PLAIN_CARDS: std::sync::Mutex<Vec<(String, Mat2d, Mat2d, Vec<Command>)>> = std::sync::Mutex::new(Vec::new());

/// The copies of plain cards sliding on (under the highlighted card, or off the list's end): each rider's card as last
/// drawn plain, at its copy's place in the menu (`x`), faded.
fn draw_card_copies(menu_xf: &Mat2d, out: &mut Vec<Command>) {
    // (the blank cards: the riders not seen yet, in their places till their cards are)
    for [x0, y0, x1, y1, a] in BLANKS.lock().map(|b| b.clone()).unwrap_or_default() {
        let corners = [menu_xf.apply(x0, y0), menu_xf.apply(x1, y0), menu_xf.apply(x1, y1), menu_xf.apply(x0, y1)];
        let xs = corners.map(|c| c.0);
        let ys = corners.map(|c| c.1);
        let r = RectF {
            x0: xs.iter().copied().fold(f32::MAX, f32::min).round(),
            y0: ys.iter().copied().fold(f32::MAX, f32::min).round(),
            x1: xs.iter().copied().fold(f32::MIN, f32::max).round(),
            y1: ys.iter().copied().fold(f32::MIN, f32::max).round(),
        };
        let quad = |out: &mut Vec<Command>, q: [f32; 4], colour: [f32; 3], alpha: f32| {
            let mut mesh = Mesh::default();
            mesh.vertices.extend([[q[0], q[1]], [q[2], q[1]], [q[2], q[3]], [q[0], q[3]]]);
            mesh.indices.extend([0, 1, 2, 0, 2, 3]);
            out.push(Command::Draw(DrawItem {
                mesh: Arc::new(mesh),
                paint: Paint::Solid([colour[0], colour[1], colour[2], alpha]),
                transform: Mat2d::IDENTITY,
                cxform: Cxform::IDENTITY,
                blend: BlendMode::Normal,
            }));
        };
        quad(out, [r.x0, r.y0, r.x1, r.y1], BLANK_FILL, 0.85 * a);
        for q in [
            [r.x0, r.y0, r.x1, r.y0 + 1.0],
            [r.x0, r.y1 - 1.0, r.x1, r.y1],
            [r.x0, r.y0 + 1.0, r.x0 + 1.0, r.y1 - 1.0],
            [r.x1 - 1.0, r.y0 + 1.0, r.x1, r.y1 - 1.0],
        ] {
            quad(out, q, PLAIN_EDGE, 0.8 * a);
        }
    }
    let ghosts = GHOST.lock().map(|g| g.clone()).unwrap_or_default();
    let Ok(cards) = PLAIN_CARDS.lock() else { return };
    let rects = CARD_RECTS.lock().map(|m| m.clone()).unwrap_or_default();
    for (rider, x, alpha) in ghosts {
        let Some((_, was_menu, was_card, draws)) = cards.iter().find(|c| c.0 == rider) else { continue };
        let Some(back) = was_menu.mul(was_card).inverse() else { continue };
        let to = menu_xf.mul(&Mat2d { tx: x, ..*was_card }).mul(&back);
        // (the copy's card where the mouse finds it, as the card itself would be; under the mouse, its glow)
        let mut glow = None;
        if alpha > 0.02
            && let Some(r) = rects.get(&rider)
        {
            let w = r.transform(&to);
            ssx_core::rider_showcase::found_card([w.x0, w.y0, w.x1, w.y1], false);
            ssx_core::rider_showcase::name_last_card(&rider);
            let mid = (w.x0 + w.x1) * 0.5;
            if hovered_card_px().is_some_and(|h| (h.x0..=h.x1).contains(&mid)) {
                glow = Some([w.x0, w.y0, w.x1, w.y1]);
            }
        }
        for c in draws {
            let mut c = c.clone();
            if let Command::Draw(d) = &mut c {
                d.transform = to.mul(&d.transform);
                d.cxform.mult[3] *= alpha;
                d.cxform.add[3] *= alpha;
            }
            out.push(c);
        }
        if let Some(g) = glow {
            hover_glow(out, g, alpha.clamp(0.0, 1.0));
        }
    }
}

/// A plain card of the rider list just drawn (`out[start..]`): kept as its rider's.
fn remember_card(lib: &Library, player: &Player, id: InstanceId, card: &Instance, menu_xf: &Mat2d, start: usize, out: &[Command]) {
    // (the card's rider as the list's layout knows it (the menu's own item, its pictures maybe not in yet), else by its
    // pictures)
    let known = CARD_RIDERS.lock().ok().and_then(|m| m.as_ref().and_then(|h| h.get(&id).cloned()));
    let rider = match known {
        Some((r, true)) => r,
        // (a card with no pictures of its rider ("NA") never kept)
        Some((_, false)) => return,
        None => match card_portrait_name(lib, player, id) {
            Some(r) => r,
            None => return,
        },
    };
    if out.len() <= start {
        return;
    }
    let draws = out[start..].to_vec();
    let Ok(mut cards) = PLAIN_CARDS.lock() else { return };
    match cards.iter_mut().find(|c| c.0 == rider) {
        Some(c) => {
            c.1 = *menu_xf;
            c.2 = card.matrix;
            c.3 = draws;
        }
        None => cards.push((rider, *menu_xf, card.matrix, draws)),
    }
}
/// Each rider's card's frame as last drawn (the drawing's pixels), with the card kept for its copies (`PLAIN_CARDS`).
static CARD_RECTS: std::sync::Mutex<std::collections::BTreeMap<String, RectF>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());
/// The rider list's cards as last laid out: each card's rider, and whether its pictures are in.
static CARD_RIDERS: std::sync::Mutex<Option<HashMap<InstanceId, (String, bool)>>> = std::sync::Mutex::new(None);

/// The rider list's blank cards (riders not seen yet): their rectangles in the menu's units, and how faded.
static BLANKS: std::sync::Mutex<Vec<[f32; 5]>> = std::sync::Mutex::new(Vec::new());
/// A blank card's inside.
const BLANK_FILL: [f32; 3] = [0.05, 0.16, 0.18];
/// The rider list's plain cards' middle, top to bottom (window pixels): the scroll arrows are level with it.
static ARROW_MID_Y: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// That plain copy is being drawn now (none of the highlighted card's changes apply to it).
static GHOSTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ghosting() -> bool {
    GHOSTING.load(std::sync::atomic::Ordering::Relaxed)
}

fn grow_begin_frame() {
    if let (Ok(mut now), Ok(mut prev)) = (CARD_NOW.lock(), CARD_PREV.lock()) {
        *prev = now.take();
    }
    let now = GROW.lock().ok().and_then(|mut g| g.take());
    if let Ok(mut p) = PREV.lock() {
        *p = now;
    }
}

fn grow_now_or_prev() -> Option<Grow> {
    GROW.lock().ok().and_then(|g| *g).or_else(|| PREV.lock().ok().and_then(|p| *p))
}

/// The portrait's window rectangle widened as its card is (the model's viewport).
fn grown_slot(rect: [f32; 4]) -> [f32; 4] {
    match GROW.lock().ok().and_then(|g| *g) {
        Some(g) => {
            let k = (g.width + g.wide) / g.width.max(1e-3);
            let p = g.pivot_px;
            let dx = if g.wide.abs() > 1e-6 { inner_shift(g) * g.wide_px / g.wide } else { 0.0 };
            [p + (rect[0] - p) * k + dx, rect[1], p + (rect[2] - p) * k + dx, rect[3] + g.by_px * 0.25]
        }
        None => rect,
    }
}

fn check_grow_cell(lib: &Library, player: &Player, id: InstanceId, inst: &Instance, xf: &Mat2d, cx: &Cxform) {
    // (the highlighted card's drawn so whether its 3-D rider is up yet or not: never a plain card for a frame)
    if ghosting() || GROW.lock().is_ok_and(|g| g.is_some()) {
        return;
    }
    let InstanceKind::Sprite(s) = &inst.kind else { return };
    // (the highlighted card with no pictures of its rider (an "NA" card): grown all the same, as a regular rider's)
    let pictureless = pictureless_highlight(player, id);
    for &c in s.children.values() {
        let Some(child) = player.get(c) else { continue };
        if child.name != "mcPortraitContainer" || !child.visible {
            continue;
        }
        let pictured = highlighted_rider_portrait(lib, player, c, child, cx.concat(&child.cxform).mult[3]);
        if !pictured && !pictureless {
            continue;
        }
        let inner_box = s.children.values().find_map(|&k| {
            let i = player.get(k).filter(|i| i.name == "mcInnerframe")?;
            let kb = player.local_bounds(lib, k);
            (!kb.is_empty()).then(|| kb.transform(&i.matrix))
        });
        let b = player.local_bounds(lib, c);
        let b = if b.is_empty() { inner_box.unwrap_or(b) } else { b.transform(&child.matrix) };
        if b.is_empty() {
            return;
        }
        // (the card grown by its portrait's place in it as a regular rider's portrait has it: the retro riders'
        // pictures (the classic riders' package) are of other sizes, which would grow the card out of shape)
        let rider = ssx_core::rider_showcase::highlight_portrait();
        let b = portrait_fit(b, inner_box, pictured && !rider.starts_with("dlc"));
        let by = (b.y1 - b.y0) * GROW_FRACTION;
        let wide = (b.x1 - b.x0) * WIDEN_FRACTION;
        let (x0, y0) = xf.apply(b.x0, b.y0);
        let (x1, y1) = xf.apply(b.x1, b.y0 + by);
        let (x2, _) = xf.apply(b.x0 + wide, b.y0);
        // The card's left edge: its backing's.
        let part = |name: &str| {
            s.children.values().find_map(|&k| {
                let i = player.get(k).filter(|i| i.name == name)?;
                let kb = player.local_bounds(lib, k);
                (!kb.is_empty()).then(|| kb.transform(&i.matrix))
            })
        };
        let backer = part("mcBacker");
        let pivot = backer.map_or(b.x0, |r| r.x0 + (r.x1 - r.x0) * CARD_MARGIN);
        let backer = backer.map_or([b.x0, b.x1, b.y0], |r| [r.x0, r.x1, r.y0]);
        let inner = part("mcInnerframe");
        let inner_bottom = inner.map_or(b.y1, |r| r.y1);
        let inner_x = inner.map_or([b.x0, b.x1], |r| [r.x0, r.x1]);
        let bar = part("mcLevelProgress").map_or([b.y1, b.y1], |r| [r.y0, r.y1]);
        let (pivot_px, _) = xf.apply(pivot, b.y0);
        // Only a card of the moving list (not the fixed `mcSelectedRider` card the menu keeps behind it).
        let Some(outer) = inst.parent.filter(|&p| player.get(p).is_some_and(|o| o.name == "mcCell")) else { return };
        let outer = Some(outer);
        if let Ok(mut g) = GROW.lock() {
            *g = Some(Grow {
                cell: id,
                outer,
                by,
                wide,
                centre: (b.x0 + b.x1) / 2.0,
                width: b.x1 - b.x0,
                pivot,
                by_px: (y1 - y0).abs(),
                wide_px: (x2 - x0).abs(),
                centre_px: (x0 + x1) / 2.0,
                pivot_px,
                backer,
                inner_bottom,
                bar,
                inner_x,
            });
        }
        return;
    }
}

/// This card's cell (`cell`, an inner `mcCell`) is the rider list's highlighted card's, and that card has no pictures
/// of its rider (an "NA" card: its rider known only from the menu).
fn pictureless_highlight(player: &Player, cell: InstanceId) -> bool {
    let Some(top) = GHOST_TOP.lock().ok().and_then(|t| *t) else { return false };
    let clip = player.get(cell).and_then(|c| c.parent).and_then(|o| player.get(o)).and_then(|o| o.parent);
    clip == Some(top)
        && CARD_RIDERS
            .lock()
            .ok()
            .and_then(|m| m.as_ref().and_then(|h| h.get(&top).cloned()))
            .is_some_and(|(r, pictured)| !pictured && r == ssx_core::rider_showcase::highlight_portrait())
}

/// A regular rider's portrait's place on its card, from the card's inner square's (its left, top, right and bottom
/// edges' distances from the square's), as last measured: the retro riders' cards grow by it.
static PORTRAIT_FIT: std::sync::Mutex<Option<[f32; 4]>> = std::sync::Mutex::new(None);

/// The highlighted card's portrait's place (`b`, in the card's units) for growing the card: a regular rider's as
/// measured (and kept), a retro rider's (`dlc…`) as a regular rider's would be on its card.
fn portrait_fit(b: RectF, inner: Option<RectF>, regular: bool) -> RectF {
    let Some(f) = inner.filter(|f| !f.is_empty()) else { return b };
    let Ok(mut fit) = PORTRAIT_FIT.lock() else { return b };
    if regular {
        *fit = Some([b.x0 - f.x0, b.y0 - f.y0, b.x1 - f.x1, b.y1 - f.y1]);
        return b;
    }
    match *fit {
        Some([l, t, r, bo]) => RectF { x0: f.x0 + l, y0: f.y0 + t, x1: f.x1 + r, y1: f.y1 + bo },
        None => b,
    }
}

/// `matrix` (in the parent's space) stretched about `(cx, cy)`.
fn stretched(matrix: Mat2d, cx: f32, cy: f32, kx: f32, ky: f32) -> Mat2d {
    Mat2d::translate(cx, cy).mul(&Mat2d::scale(kx, ky)).mul(&Mat2d::translate(-cx, -cy)).mul(&matrix)
}

/// The highlighted rider's level for sale (the rider step's X prompt and price, in its bio's units): their move to the
/// highlighted card's bottom right, worked out with the bio's place (drawn just before them).
/// …this many window pixels right of the highlighted card's edge.
const LEVEL_PROMPT_GAP_PX: f32 = 14.0;
/// The level for sale's X button as drawn this frame (the drawing's pixels), for its glow under the mouse.
static LEVEL_BUTTON: std::sync::Mutex<Option<RectF>> = std::sync::Mutex::new(None);

/// The mouse on the rider step's highlighted card and on the level's X button: how far each one's glow has grown (0
/// to 1, easing in and out), and when they were last eased.
static HOVER_GLOWS: std::sync::Mutex<([f32; 2], Option<std::time::Instant>)> = std::sync::Mutex::new(([0.0; 2], None));
/// A glow grows in (or back out) over this long (seconds).
const HOVER_EASE_SECS: f32 = 0.15;

/// The mouse pointer on the rider step in the drawing's pixels.
fn pointer_px() -> Option<(f32, f32)> {
    let (x, y) = ssx_core::rider_showcase::pointer()?;
    let (w, h) = FRAME_SIZE.lock().ok().and_then(|s| *s)?;
    Some((x * w, y * h))
}

/// The glows under the mouse eased toward where the pointer is, and the X button's drawn (after everything).
fn hover_glows(out: &mut Vec<Command>) {
    let at = pointer_px();
    let inside = |r: Option<RectF>| {
        r.zip(at).is_some_and(|(r, (x, y))| (r.x0..=r.x1).contains(&x) && (r.y0..=r.y1).contains(&y))
    };
    let card = CARD_NOW.lock().ok().and_then(|c| *c).or_else(|| CARD_PREV.lock().ok().and_then(|c| *c));
    let button = LEVEL_BUTTON.lock().ok().and_then(|b| *b).filter(|_| ssx_core::rider_showcase::level_offer());
    let mut want = [inside(card) as u8 as f32, inside(button) as u8 as f32];
    // (a frame or two without the pointer seen there (the window not focused, say) doesn't let the glow go: only a
    // few in a row)
    static MISSED: std::sync::Mutex<[u8; 2]> = std::sync::Mutex::new([4; 2]);
    if let Ok(mut m) = MISSED.lock() {
        for i in 0..2 {
            if want[i] > 0.0 {
                m[i] = 0;
            } else if m[i] < 4 {
                m[i] += 1;
                want[i] = 1.0;
            }
        }
    }
    let Ok(mut g) = HOVER_GLOWS.lock() else { return };
    let now = std::time::Instant::now();
    let dt = g.1.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
    g.1 = Some(now);
    for i in 0..2 {
        let step = dt / HOVER_EASE_SECS;
        g.0[i] = if want[i] > g.0[i] { (g.0[i] + step).min(1.0) } else { (g.0[i] - step).max(0.0) };
    }
    let _ = out;
}

/// How far the level's X button's glow has grown under the mouse (0 to 1, eased).
fn button_hover() -> f32 {
    let k = HOVER_GLOWS.lock().map_or(0.0, |g| g.0[1]);
    k * k * (3.0 - 2.0 * k)
}

/// The level's X button's glow under the mouse: the button's own shape drawn again behind it, a little larger each
/// time and fainter, in the card's glow green (so the glow follows its edge).
static BUTTON_GLOWING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The highlighted card's pulse (the Custom Event Menus mod): its inner line's and glow's brightness, 0.7 to 1, on a
/// steady slow beat.
fn card_pulse() -> f32 {
    let t = PULSE_START.get_or_init(std::time::Instant::now).elapsed().as_secs_f32();
    0.7 + 0.3 * (0.5 + 0.5 * (t * std::f32::consts::TAU / 1.6).sin())
}

/// A band round `rect` (the drawing's pixels) from `d0` to `d1` pixels outside its edge, its corners rounded (as an
/// edge's glow spreads round a corner).
fn soft_ring(out: &mut Vec<Command>, rect: [f32; 4], d0: f32, d1: f32, c: [f32; 4]) {
    if c[3] <= 0.0 || d1 <= d0 {
        return;
    }
    let [x0, y0, x1, y1] = rect;
    const ARC: usize = 8;
    // (the outline `d` outside the rectangle: its corners quarter circles about the rectangle's own)
    let outline = |d: f32| {
        let mut pts = Vec::with_capacity(4 * (ARC + 1));
        let corners = [(x1, y0, -0.5f32), (x1, y1, 0.0), (x0, y1, 0.5), (x0, y0, 1.0)];
        for (cx, cy, start) in corners {
            for k in 0..=ARC {
                let a = (start + 0.5 * k as f32 / ARC as f32) * std::f32::consts::PI;
                pts.push([cx + d * a.cos(), cy + d * a.sin()]);
            }
        }
        pts
    };
    let (inner, outer) = (outline(d0), outline(d1));
    let n = inner.len() as u32;
    let mut mesh = Mesh::default();
    mesh.vertices.extend(inner);
    mesh.vertices.extend(outer);
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.indices.extend([i, j, n + j, i, n + j, n + i]);
    }
    out.push(Command::Draw(DrawItem {
        mesh: Arc::new(mesh),
        paint: Paint::Solid(c),
        transform: Mat2d::IDENTITY,
        cxform: Cxform::IDENTITY,
        blend: BlendMode::Normal,
    }));
}

/// How far the highlighted card's glow has grown under the mouse (0 to 1, eased).
fn card_hover() -> f32 {
    let k = HOVER_GLOWS.lock().map_or(0.0, |g| g.0[0]);
    k * k * (3.0 - 2.0 * k)
}

/// A ring `w` pixels wide, `d` pixels outside `rect` (the drawing's pixels).
fn glow_ring(out: &mut Vec<Command>, rect: [f32; 4], d: f32, w: f32, c: [f32; 4]) {
    let [x0, y0, x1, y1] = rect;
    let (a0, b0, a1, b1) = (x0 - d - w, y0 - d - w, x1 + d + w, y1 + d + w);
    let (i0, j0, i1, j1) = (x0 - d, y0 - d, x1 + d, y1 + d);
    let mut mesh = Mesh::default();
    for [qx0, qy0, qx1, qy1] in [[a0, b0, a1, j0], [a0, j1, a1, b1], [a0, j0, i0, j1], [i1, j0, a1, j1]] {
        let base = mesh.vertices.len() as u32;
        mesh.vertices.extend([[qx0, qy0], [qx1, qy0], [qx1, qy1], [qx0, qy1]]);
        mesh.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    out.push(Command::Draw(DrawItem {
        mesh: Arc::new(mesh),
        paint: Paint::Solid(c),
        transform: Mat2d::IDENTITY,
        cxform: Cxform::IDENTITY,
        blend: BlendMode::Normal,
    }));
}

static LEVEL_PROMPT_SHIFT: std::sync::Mutex<Option<(f32, f32)>> = std::sync::Mutex::new(None);

fn grown_card_part(lib: &Library, player: &Player, id: InstanceId, inst: &Instance, parent: &Mat2d, matrix: Mat2d) -> Mat2d {
    if ghosting() {
        return matrix;
    }
    if matches!(inst.name.as_str(), "mcPrompt" | "mcPrice")
        && ssx_core::rider_showcase::enabled()
        && inst.parent.and_then(|p| player.get(p)).is_some_and(|p| p.name == "mcBioPrompt")
    {
        return match LEVEL_PROMPT_SHIFT.lock().ok().and_then(|p| *p) {
            Some((x, y)) => Mat2d::translate(x, y).mul(&matrix),
            None => matrix,
        };
    }
    if inst.name == "mcSurvival" {
        let px = grow_now_or_prev().map_or(0.0, |g| g.by_px);
        if px <= 0.0 || !ssx_core::rider_showcase::enabled() || parent.d.abs() < 1e-6 {
            return matrix;
        }
        return Mat2d::translate(0.0, px / parent.d).mul(&matrix);
    }
    if inst.name == "mcBioPrompt" {
        // The rider's bio: under the grown card, its text's left edge at the card's.
        let card = CARD_NOW.lock().ok().and_then(|c| *c).or_else(|| CARD_PREV.lock().ok().and_then(|c| *c));
        let (Some(card), InstanceKind::Sprite(s)) = (card, &inst.kind) else { return matrix };
        if !ssx_core::rider_showcase::enabled() || parent.a.abs() < 1e-6 || parent.d.abs() < 1e-6 {
            return matrix;
        }
        let Some((c, child)) =
            s.children.values().find_map(|&c| player.get(c).filter(|ch| ch.name == "mcDescription").map(|ch| (c, ch)))
        else {
            return matrix;
        };
        let d = player.local_bounds(lib, c);
        if d.is_empty() {
            return matrix;
        }
        let d = d.transform(&parent.mul(&matrix).mul(&child.matrix));
        let (dx, dy) = (card.x0 - d.x0, card.y1 + 8.0 - d.y0);
        // (a level for sale: its X prompt and price right of the card, centred between its bottom and the plain cards'
        // (its own before it grew))
        let mut shift = None;
        if ssx_core::rider_showcase::level_offer() && !ssx_core::rider_showcase::gear_view() {
            let world = parent.mul(&Mat2d::translate(dx / parent.a, dy / parent.d).mul(&matrix));
            let mut u: Option<RectF> = None;
            let mut button: Option<RectF> = None;
            for name in ["mcPrompt", "mcPrice"] {
                let Some((c, ch)) =
                    s.children.values().find_map(|&c| player.get(c).filter(|ch| ch.name == name).map(|ch| (c, ch)))
                else {
                    continue;
                };
                let b = player.local_bounds(lib, c);
                if b.is_empty() || !ch.visible {
                    continue;
                }
                let b = b.transform(&world.mul(&ch.matrix));
                if name == "mcPrompt" {
                    button = Some(b);
                }
                u = Some(match u {
                    Some(a) => RectF { x0: a.x0.min(b.x0), y0: a.y0.min(b.y0), x1: a.x1.max(b.x1), y1: a.y1.max(b.y1) },
                    None => b,
                });
            }
            if let Some(u) = u
                && world.a.abs() > 1e-6
                && world.d.abs() > 1e-6
            {
                let plain_bottom = card.y1 - grow_now_or_prev().map_or(0.0, |g| g.by_px.max(0.0));
                let mid = (card.y1 + plain_bottom) * 0.5;
                let (x, y) = (card.x1 + LEVEL_PROMPT_GAP_PX - u.x0, mid - (u.y0 + u.y1) * 0.5);
                shift = Some((x / world.a, y / world.d));
                // (where it is for the mouse: a click on it is X)
                ssx_core::rider_showcase::found_prompt([u.x0 + x - 6.0, u.y0 + y - 6.0, u.x1 + x + 6.0, u.y1 + y + 6.0]);
                if let (Some(b), Ok(mut lb)) = (button, LEVEL_BUTTON.lock()) {
                    *lb = Some(RectF { x0: b.x0 + x, y0: b.y0 + y, x1: b.x1 + x, y1: b.y1 + y });
                }
            }
        }
        if let Ok(mut p) = LEVEL_PROMPT_SHIFT.lock() {
            *p = shift;
        }

        return Mat2d::translate(dx / parent.a, dy / parent.d).mul(&matrix);
    }
    let Some(g) = GROW.lock().ok().and_then(|g| *g) else { return matrix };
    if inst.parent != Some(g.cell) {
        return matrix;
    }
    let bounds = || {
        let b = player.local_bounds(lib, id);
        (!b.is_empty()).then(|| b.transform(&matrix)).filter(|b| b.x1 - b.x0 > 1e-3 && b.y1 - b.y0 > 1e-3)
    };
    let kx = (g.width + g.wide) / g.width.max(1e-3);
    let out = grown_part(inst, matrix, g, kx, &bounds);
    if inst.character.as_ref().is_some_and(|c| c.id == 44) {
        let b = player.local_bounds(lib, id);
        if !b.is_empty()
            && let Ok(mut now) = CARD_NOW.lock()
        {
            let r = b.transform(&parent.mul(&out));
            *now = Some(r);
            // (where it is for the mouse)
            ssx_core::rider_showcase::found_card([r.x0, r.y0, r.x1, r.y1], true);
        }
    }
    out
}

fn grown_part(inst: &Instance, matrix: Mat2d, g: Grow, kx: f32, bounds: &dyn Fn() -> Option<RectF>) -> Mat2d {
    // A part kept near the card's left edge keeps its place along it (its distance from the edge widened too).
    let along = |m: Mat2d| match bounds() {
        Some(b) => Mat2d::translate((kx - 1.0) * (b.x0 - g.pivot), 0.0).mul(&m),
        None => m,
    };
    let centre = g.pivot + (g.centre - g.pivot) * kx;
    match inst.name.as_str() {
        // The level number: below the model, centred on the card, a little larger.
        // (centred on the card, and between the inner square's bottom and the XP bar's top, below the model)
        "mcLevel" => match bounds() {
            Some(b) => {
                let (cx, cy) = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
                let px = if g.by_px > 1e-3 { g.by / g.by_px } else { 0.0 };
                let mid = (g.inner_bottom + g.bar[0] + g.by) / 2.0 + LEVEL_DROP_PX * px;
                Mat2d::translate(centre - cx, mid - cy).mul(&stretched(matrix, cx, cy, LEVEL_SCALE, LEVEL_SCALE))
            }
            None => Mat2d::translate(0.0, g.by).mul(&along(matrix)),
        },
        "mcTitle" => along(matrix),
        "mcLevelProgress" => Mat2d::translate(0.0, g.by).mul(&stretched(matrix, g.pivot, 0.0, kx, 1.0)),
        "mcInnerframe" | "mcCompareGlint" => {
            Mat2d::translate(inner_shift(g), 0.0).mul(&stretched(matrix, g.pivot, 0.0, kx, 1.0))
        }
        name if name == "mcBacker" || inst.character.as_ref().is_some_and(|c| c.id == 44) => match bounds() {
            Some(b) => {
                let h = b.y1 - b.y0;
                stretched(matrix, g.pivot, b.y0, kx, (h + g.by) / h)
            }
            None => stretched(matrix, g.pivot, 0.0, kx, 1.0),
        },
        _ => matrix,
    }
}

/// The rider showcase's widened card: its frames' nine-slice grids keep their whole width as fixed edges (they were
/// made only to stretch downwards), so for the grown card all but a thin band at each side (the border's own
/// thickness) stretches: the angled bottom's lines then stretch straight, with no bend where a fixed band meets the
/// stretching middle.
fn widened_grid(inst: &Instance, grid: RectF, bounds: RectF) -> RectF {
    let grown = !ghosting() && GROW.lock().ok().and_then(|g| *g).is_some_and(|g| inst.parent == Some(g.cell));
    if !grown || bounds.is_empty() {
        return grid;
    }
    let edge = (bounds.x1 - bounds.x0) * 0.05;
    RectF { x0: bounds.x0 + edge, x1: bounds.x1 - edge, ..grid }
}

/// A portrait in a card of the moving list: its card (an inner `mcCell`) sits in an outer `mcCell`.
fn in_list_card(player: &Player, inst: &Instance) -> bool {
    let parent = |i: &Instance| i.parent.and_then(|p| player.get(p));
    parent(inst).filter(|c| c.name == "mcCell").and_then(parent).is_some_and(|o| o.name == "mcCell")
}


fn highlighted_rider_portrait(lib: &Library, player: &Player, id: InstanceId, inst: &Instance, alpha: f32) -> bool {
    if inst.name != "mcPortraitContainer" || alpha < 0.6 || !ssx_core::rider_showcase::enabled() {
        return false;
    }
    let name = ssx_core::rider_showcase::highlight_portrait();
    if name.is_empty() {
        return false;
    }
    let prefix = format!("{name}_");
    let mut stack = vec![(id, 0usize)];
    while let Some((at, depth)) = stack.pop() {
        let Some(i) = player.get(at) else { continue };
        let file = lib.movie(i.movie).path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
        if file.starts_with(&prefix) && file.ends_with(".dds") {
            return true;
        }
        if depth < 4
            && let InstanceKind::Sprite(s) = &i.kind
        {
            stack.extend(s.children.values().map(|&c| (c, depth + 1)));
        }
    }
    false
}


/// The rider showcase's list: the highlighted rider's card held still, its left edge under the header's EXPLORE
/// breadcrumb's; the other riders before and after it in the list's own order (the game's, not the menu's cards':
/// the menu hands riders from card to card as it scrolls), evenly spaced, sliding a slot at a time as the highlight
/// moves, the list going round. Each rider shows on one card (the menu's least faded one), unfaded; the menu's other
/// copies are hidden. The cards' own places are moved (so the mouse finds them where they're seen); when the
/// showcase is off they go back as the menu had them.
/// The rider the list's menu has just highlighted, before the game (and so the showcase) has heard of it (the menu
/// moves its highlight as the button's pressed; the game hears of it at its next 30 Hz step): its portrait name, and
/// when.
static EARLY: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);

/// The rider the list's menu has highlighted ahead of the game, if any (for a second at most): the showcase shows
/// it till the game catches up.
pub fn rider_list_early() -> Option<String> {
    EARLY.lock().ok().and_then(|e| e.as_ref().filter(|(_, t)| t.elapsed().as_secs_f32() < 1.0).map(|(r, _)| r.clone()))
}

/// The game has caught up with the list's menu (its highlighted rider is the one the menu highlighted).
pub fn clear_rider_list_early() {
    if let Ok(mut e) = EARLY.lock() {
        *e = None;
    }
}

/// The rider list's menu (`mcRiderMenu`, a `components.RiderMenu`) as last laid out: kept for `read_rider_menu`.
static RIDER_MENU: std::sync::Mutex<Option<InstanceId>> = std::sync::Mutex::new(None);
/// The rider list's menu's cells as the menu itself has them: each card (the menu's own child, a `clipInstance`) and
/// the list item (the rider's index in the list) the menu has put in it. The menu hands riders from card to card as it
/// scrolls and loads a rider's pictures into a card a moment later (the card meanwhile shows nothing, or "NA"): by its
/// item, a card is known as its rider's before its pictures are in.
/// (and how many items the menu has: its items are the list's riders only when it has as many)
static RIDER_CELLS: std::sync::Mutex<(Vec<(InstanceId, usize)>, usize)> = std::sync::Mutex::new((Vec::new(), 0));

/// Reads the rider list's menu's cells (`read_rider_menu`'s note): before `relayout_rider_list`, each frame.
/// A port mod (the test peak): its drop screen's button bar (the one with Freeride on X) also shows Y "Switch Event"
/// (Y turns its events between Trick It and Race It), added with the bar's own `AddButton` in the style of Freeride's
/// button; taken off again when the screen isn't the test peak's.
pub fn test_peak_callout(cx: &mut crate::avm1::Ctx, show: bool) {
    use crate::avm1::value::Value;
    const LABEL: &str = TEST_PEAK_LABEL;
    static ADDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !show && !ADDED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let mut callout = Value::Object(cx.vm.global);
    for p in ["components", "ButtonCallout"] {
        callout = cx.get_member(&callout, p).unwrap_or_default();
    }
    let number = |cx: &mut crate::avm1::Ctx, obj: &Value, name: &str| {
        let v = cx.get_member(obj, name).ok()?;
        let n = cx.to_number(&v);
        n.is_finite().then_some(n)
    };
    let (Some(x), Some(y)) =
        (number(cx, &callout, "CALLOUT_FACE_BUTTON_TERTIARY"), number(cx, &callout, "CALLOUT_FACE_BUTTON_QUATERNARY"))
    else {
        return;
    };
    let objects: Vec<_> = cx
        .vm
        .display_objs
        .iter()
        .filter(|(id, _)| cx.player.get(**id).is_some_and(|i| matches!(i.kind, InstanceKind::Sprite(_))))
        .map(|(_, &o)| o)
        .collect();
    let mut added = false;
    for o in objects {
        let bar = Value::Object(o);
        let Ok(infos) = cx.get_member(&bar, "mButtonInfoArray") else { continue };
        if !matches!(infos, Value::Object(_)) || !cx.get_member(&bar, "AddButton").is_ok_and(|f| cx.is_function(&f)) {
            continue;
        }
        let n = number(cx, &infos, "length").unwrap_or(0.0).clamp(0.0, 16.0) as usize;
        let (mut style, mut ours, mut freeride) = (None, false, false);
        for i in 0..n {
            let info = cx.get_member(&infos, &i.to_string()).unwrap_or_default();
            let icon = number(cx, &info, "iconId");
            let label = cx.get_member(&info, "label").unwrap_or_default();
            let label = cx.to_string(&label).map(|s| s.to_string()).unwrap_or_default();
            if icon == Some(x) {
                freeride = true;
                style = cx.get_member(&info, "style").ok();
            }
            if icon == Some(y) && label == LABEL {
                ours = true;
            }
        }
        if show && freeride {
            wrap_callout_bar(cx, &bar, x, y);
            if ours {
                callout_order(cx, &bar, x, y);
            }
        }
        if show && freeride && !ours {
            if let Ok((f, _)) = cx.get_method(&bar, "AddButton") {
                let args = [Value::Number(y), Value::str(LABEL), style.unwrap_or_default(), Value::Undefined, Value::Undefined];
                cx.call_top(&f, bar.clone(), &args);
            }
            added = true;
        } else if !show && ours {
            if let Ok((f, _)) = cx.get_method(&bar, "RemoveButtonById") {
                cx.call_top(&f, bar.clone(), &[Value::Number(y)]);
            }
        } else if show && ours {
            added = true;
        }
    }
    ADDED.store(added, std::sync::atomic::Ordering::Relaxed);
}

/// The test peak's Y button kept steady on its drop screen: the screen takes its bar's Y button off and puts its
/// buttons back as the chosen drop changes, and a button put back shows a frame or two late (it loads first), so the Y
/// blinked as the drops were scrolled. The bar's own `AddButton` and `RemoveButtonById` are wrapped (once a bar): on the
/// test peak's drop screen, the Y isn't taken off (nor another Y put in its place), and when X (Freeride) is put back,
/// the Y goes in just after it, in the same moment.
fn wrap_callout_bar(cx: &mut crate::avm1::Ctx, bar: &crate::avm1::value::Value, x: f64, y: f64) {
    use crate::avm1::object::{Func, FuncCode};
    use crate::avm1::value::Value;
    if cx.get_member(bar, "__pa_wrapped").is_ok_and(|v| matches!(v, Value::Bool(true))) {
        return;
    }
    let (Ok((add, _)), Ok((remove, _))) = (cx.get_method(bar, "AddButton"), cx.get_method(bar, "RemoveButtonById"))
    else {
        return;
    };
    if !cx.is_function(&add) || !cx.is_function(&remove) {
        return;
    }
    let wrapped_add = cx.new_function_object(Func { code: FuncCode::Native(callout_add), scope: None, base: None });
    let wrapped_remove =
        cx.new_function_object(Func { code: FuncCode::Native(callout_remove), scope: None, base: None });
    for (k, v) in [
        ("__pa_add", add),
        ("__pa_remove", remove),
        ("__pa_x", Value::Number(x)),
        ("__pa_y", Value::Number(y)),
        ("AddButton", Value::Object(wrapped_add)),
        ("RemoveButtonById", Value::Object(wrapped_remove)),
        ("__pa_wrapped", Value::Bool(true)),
    ] {
        let _ = cx.set_member(bar, k, v);
    }
}

const TEST_PEAK_LABEL: &str = "Switch Event";

fn callout_number(cx: &mut crate::avm1::Ctx, v: &crate::avm1::value::Value) -> f64 {
    cx.to_number(v)
}

fn callout_add(
    cx: &mut crate::avm1::Ctx,
    this: crate::avm1::value::Value,
    args: &[crate::avm1::value::Value],
) -> Result<crate::avm1::value::Value, crate::avm1::Fault> {
    use crate::avm1::value::Value;
    let orig = cx.get_member(&this, "__pa_add").unwrap_or_default();
    let on = ssx_assets::content::data::dev_peak::drop_screen();
    let x = cx.get_member(&this, "__pa_x").map(|v| callout_number(cx, &v)).unwrap_or(f64::NAN);
    let y = cx.get_member(&this, "__pa_y").map(|v| callout_number(cx, &v)).unwrap_or(f64::NAN);
    let icon = args.first().map_or(f64::NAN, |v| callout_number(cx, v));
    let label = args.get(1).and_then(|v| cx.to_string(v).ok()).map(|s| s.to_string()).unwrap_or_default();
    // (another Y, not ours, on the test peak's drop screen: left out, ours stays)
    if on && icon == y && label != TEST_PEAK_LABEL {
        return Ok(Value::Undefined);
    }
    let out = cx.call(&orig, this.clone(), args, Value::Undefined)?;
    // (X put back: ours just after it, unless it's there)
    if on && icon == x && !callout_has(cx, &this, y) {
        let style = args.get(2).cloned().unwrap_or_default();
        let ours = [Value::Number(y), Value::str(TEST_PEAK_LABEL), style, Value::Undefined, Value::Undefined];
        cx.call(&orig, this, &ours, Value::Undefined)?;
    } else if on && icon == x {
        // (X put back after ours (it goes in at the end): swapped back, so Freeride stays first)
        callout_order(cx, &this, x, y);
    }
    Ok(out)
}

/// Freeride (X) kept before ours (Y) on the bar: if the screen put X back after it (it does as Y switches the event),
/// the two swapped in the bar's lists (its buttons' info and their clips) and the bar laid out again, without loading
/// either button again (no blink).
fn callout_order(cx: &mut crate::avm1::Ctx, bar: &crate::avm1::value::Value, x: f64, y: f64) {
    use crate::avm1::value::Value;
    let Ok(infos) = cx.get_member(bar, "mButtonInfoArray") else { return };
    let n = cx.get_member(&infos, "length").map(|v| cx.to_number(&v)).unwrap_or(0.0);
    let n = if n.is_finite() { n.clamp(0.0, 16.0) as usize } else { 0 };
    let at = |cx: &mut crate::avm1::Ctx, icon: f64| {
        (0..n).find(|i| {
            let info = cx.get_member(&infos, &i.to_string()).unwrap_or_default();
            cx.get_member(&info, "iconId").is_ok_and(|v| cx.to_number(&v) == icon)
        })
    };
    let (Some(xi), Some(yi)) = (at(cx, x), at(cx, y)) else { return };
    if xi < yi {
        return;
    }
    for list in ["mButtonInfoArray", "mButtonArray"] {
        let Ok(arr) = cx.get_member(bar, list) else { continue };
        if !matches!(arr, Value::Object(_)) {
            continue;
        }
        let (a, b) = (xi.to_string(), yi.to_string());
        let va = cx.get_member(&arr, &a).unwrap_or_default();
        let vb = cx.get_member(&arr, &b).unwrap_or_default();
        let _ = cx.set_member(&arr, &a, vb);
        let _ = cx.set_member(&arr, &b, va);
    }
    if let Ok((f, _)) = cx.get_method(bar, "AdjustPosition") {
        let _ = cx.call(&f, bar.clone(), &[], Value::Undefined);
    }
}

fn callout_remove(
    cx: &mut crate::avm1::Ctx,
    this: crate::avm1::value::Value,
    args: &[crate::avm1::value::Value],
) -> Result<crate::avm1::value::Value, crate::avm1::Fault> {
    use crate::avm1::value::Value;
    let orig = cx.get_member(&this, "__pa_remove").unwrap_or_default();
    let y = cx.get_member(&this, "__pa_y").map(|v| callout_number(cx, &v)).unwrap_or(f64::NAN);
    let icon = args.first().map_or(f64::NAN, |v| callout_number(cx, v));
    if ssx_assets::content::data::dev_peak::drop_screen() && icon == y {
        return Ok(Value::Undefined);
    }
    cx.call(&orig, this, args, Value::Undefined)
}

/// Whether the bar has a button of this icon.
fn callout_has(cx: &mut crate::avm1::Ctx, bar: &crate::avm1::value::Value, icon: f64) -> bool {
    let Ok(infos) = cx.get_member(bar, "mButtonInfoArray") else { return false };
    let n = cx.get_member(&infos, "length").map(|v| cx.to_number(&v)).unwrap_or(0.0);
    let n = if n.is_finite() { n.clamp(0.0, 16.0) as usize } else { 0 };
    (0..n).any(|i| {
        let info = cx.get_member(&infos, &i.to_string()).unwrap_or_default();
        cx.get_member(&info, "iconId").is_ok_and(|v| cx.to_number(&v) == icon)
    })
}

/// The rider list's slides at the game's length, or quicker (`quick`): `components.RiderMenu`'s tween length.
fn quick_slides(cx: &mut crate::avm1::Ctx, quick: bool) {
    use crate::avm1::value::Value;
    /// The game's own length, kept from before it was first changed.
    static GAME_LENGTH: std::sync::Mutex<Option<f64>> = std::sync::Mutex::new(None);
    /// A quick slide is this much of the game's.
    const QUICK: f64 = 0.35;
    let mut class = Value::Object(cx.vm.global);
    for p in ["components", "RiderMenu"] {
        class = cx.get_member(&class, p).unwrap_or_default();
    }
    if !matches!(class, Value::Object(_)) {
        return;
    }
    let name = "ANIMATE_SELECTION_TWEEN_LENGTH";
    let now = cx.get_member(&class, name).map(|v| cx.to_number(&v)).unwrap_or(f64::NAN);
    if !now.is_finite() || now <= 0.0 {
        return;
    }
    let Ok(mut game) = GAME_LENGTH.lock() else { return };
    let game = *game.get_or_insert(now);
    let want = if quick { game * QUICK } else { game };
    if (now - want).abs() > 1e-6 {
        let _ = cx.set_member(&class, name, Value::Number(want));
    }
}

pub fn read_rider_menu(cx: &mut crate::avm1::Ctx) {
    use crate::avm1::value::Value;
    let mut cells = Vec::new();
    let mut items = 0;
    let menu = RIDER_MENU.lock().ok().and_then(|m| *m);
    if let Some(menu) = menu.filter(|&m| cx.player.get(m).is_some_and(|i| i.name == "mcRiderMenu")) {
        let obj = Value::Object(cx.object_for(menu));
        let number = |cx: &mut crate::avm1::Ctx, name: &str| {
            let v = cx.get_member(&obj, name).ok()?;
            let n = cx.to_number(&v);
            n.is_finite().then_some(n)
        };
        // (the Custom Event Menus mod: the menu given a card for every rider. It makes `mMaxVisibleItems` cards (14,
        // fewer than the riders now there are more of them), so riders far from the highlighted one had no card till
        // it scrolled near them. Once per menu, while it's active and still: its most cards raised to its riders' count
        // and its cards made again as it does when its riders are published (`RefreshMenuItems`, no argument:
        // `AdjustItems(mMenuData.length)` makes that many cards on a list going round, `UpdateCellsWithData` fills
        // them). Should the menu not come out whole, it's put back as it was.)
        static RAISED: std::sync::Mutex<Option<InstanceId>> = std::sync::Mutex::new(None);
        let flag = |cx: &mut crate::avm1::Ctx, name: &str| cx.get_member(&obj, name).map(|v| cx.to_bool(&v)).unwrap_or(false);
        // (the Custom Event Menus mod: while the mouse moves the list several cards, each card's slide is quicker
        // (`components.RiderMenu.ANIMATE_SELECTION_TWEEN_LENGTH`), and the next move goes as soon as one's done)
        ssx_core::rider_showcase::set_menu_animating(flag(cx, "mIsAnimating"));
        quick_slides(cx, ssx_core::rider_showcase::quick_scroll());
        let tried = RAISED.lock().ok().and_then(|r| *r) == Some(menu);
        if !tried
            && let (Some(max), Some(total)) = (number(cx, "mMaxVisibleItems"), number(cx, "mNumTotalItems"))
            && total >= 1.0
            && flag(cx, "mIsActive")
            && !flag(cx, "mIsAnimating")
            && flag(cx, "mIsCircular")
        {
            if let Ok(mut r) = RAISED.lock() {
                *r = Some(menu);
            }
            if max < total
                && total <= 64.0
                && let Ok((refresh, _)) = cx.get_method(&obj, "RefreshMenuItems")
                && cx.is_function(&refresh)
                && cx.set_member(&obj, "mMaxVisibleItems", Value::Number(total)).is_ok()
            {
                cx.call_top(&refresh, obj.clone(), &[]);
                let whole = number(cx, "mFirstVisibleItemIndex").is_some()
                    && number(cx, "mNumVisibleItems").is_some_and(|v| v >= 1.0)
                    && number(cx, "mNumTotalItems") == Some(total);
                if !whole && cx.set_member(&obj, "mMaxVisibleItems", Value::Number(max)).is_ok() {
                    cx.call_top(&refresh, obj.clone(), &[]);
                }
            }
        }
        let first = number(cx, "mFirstVisibleItemIndex");
        let total = number(cx, "mNumTotalItems");
        let visible = number(cx, "mNumVisibleItems");
        let circular = cx.get_member(&obj, "mIsCircular").map(|v| cx.to_bool(&v)).unwrap_or(false);
        if let (Some(first), Some(total), Some(visible)) = (first, total, visible)
            && total >= 1.0
            && let Ok((get, _)) = cx.get_method(&obj, "GetCellByIndex")
            && cx.is_function(&get)
        {
            let (first, total, visible) = (first as i64, total.min(1024.0) as i64, visible.clamp(0.0, 128.0) as i64);
            items = total as usize;
            for i in 0..visible {
                let item = if circular { (first + i).rem_euclid(total) } else { first + i };
                if item < 0 || item >= total {
                    continue;
                }
                let cell = cx.call_top(&get, obj.clone(), &[Value::Number(i as f64)]);
                let Some(mut c) = cx.display_of(&cell) else { continue };
                // (the menu's own child: the card)
                let mut steps = 0;
                while let Some(parent) = cx.player.get(c).and_then(|k| k.parent)
                    && parent != menu
                    && steps < 8
                {
                    c = parent;
                    steps += 1;
                }
                if cx.player.get(c).is_some_and(|k| k.parent == Some(menu)) {
                    cells.push((c, item as usize));
                }
            }
        }
    }
    if let Ok(mut c) = RIDER_CELLS.lock() {
        *c = (cells, items);
    }
}

/// The rider list's menu's cells as last read: how many cards it has, and how many items (for the log).
pub fn rider_menu_counts() -> (usize, usize) {
    RIDER_CELLS.lock().map(|c| (c.0.len(), c.1)).unwrap_or((0, 0))
}

pub fn relayout_rider_list(lib: &Library, player: &mut Player, viewport: (u32, u32), mode: ScaleMode) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    /// A card as the menu had it: its place, whether it showed, its fade (and its outer cell's).
    #[derive(Clone, Copy)]
    struct Own {
        x: f32,
        y: f32,
        /// Its scale (the cards either side of the highlighted one are drawn smaller when they would not all fit).
        abcd: [f32; 4],
        visible: bool,
        alpha: f32,
        cell: Option<(InstanceId, f32)>,
    }
    struct State {
        menu: Option<InstanceId>,
        bio: Option<InstanceId>,
        /// The header's first breadcrumb (EXPLORE): the highlighted card's left edge lines up under its box's.
        crumb: Option<InstanceId>,
        own: HashMap<InstanceId, Own>,
        /// Each rider's place as drawn now (sliding), and its slot (from the highlighted card's) last frame.
        now: HashMap<String, f32>,
        slot: HashMap<String, i64>,
        at: Option<std::time::Instant>,
        /// The riders whose cards the menu showed highlighted last frame, and the rider the menu has just highlighted
        /// before the showcase knows (with the showcase's highlighted rider then, and when).
        lit: std::collections::HashSet<String>,
        /// The gear and perks box (`mcBioTop`) moved to line up with the highlighted card: its own place.
        bio_own: Option<(InstanceId, f32, f32)>,
        /// Plain copies of cards sliding on from their last places (the newly highlighted card's): its rider, where
        /// it's drawn now, where from, where to, and whether it fades as it goes (off the list's end).
        ghosts: Vec<(String, f32, f32, f32, bool)>,
        /// Last frame was past the rider step (the gear steps); and when the rider step came back from there (its
        /// other cards fading in as the rider's view moves back into the highlighted card).
        was_gear: Option<std::time::Instant>,
        back_at: Option<std::time::Instant>,
        /// How far the whole list (but the highlighted card) is drawn from its places, easing to none: a step along
        /// the list moves every card by one place together (none lagging the others).
        shift: f32,
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    let Ok(mut guard) = STATE.lock() else { return };
    let st = guard.get_or_insert_with(|| State {
        menu: None,
        bio: None,
        crumb: None,
        own: HashMap::new(),
        now: HashMap::new(),
        slot: HashMap::new(),
        at: None,
        lit: std::collections::HashSet::new(),
        bio_own: None,
        ghosts: Vec::new(),
        was_gear: None,
        back_at: None,
        shift: 0.0,
    });
    if let Ok(mut g) = GHOST.lock() {
        g.clear();
    }
    if let Ok(mut t) = GHOST_TOP.lock() {
        *t = None;
    }
    let dt = st.at.replace(std::time::Instant::now()).map_or(0.0, |t| t.elapsed().as_secs_f32().min(0.1));
    let mut highlighted = ssx_core::rider_showcase::highlight_portrait();
    let order = ssx_core::rider_showcase::list_order();
    let on = ssx_core::rider_showcase::enabled() && !highlighted.is_empty() && order.len() >= 2;
    let gear = ssx_core::rider_showcase::enabled() && ssx_core::rider_showcase::gear_view();
    // (the gear steps seen last: kept through the menu's handing round of its instances on the way back)
    if gear {
        st.was_gear = Some(std::time::Instant::now());
    }
    if st.was_gear.is_some_and(|t| t.elapsed().as_secs_f32() > 3.0) {
        st.was_gear = None;
    }
    let from_gear = st.was_gear.is_some();
    if !from_gear && st.back_at.is_none() {
        BIO_FADE.store(1.0f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    let restore = |st: &mut State, player: &mut Player| {
        for (&c, own) in &st.own {
            if let Some(i) = player.get_mut(c) {
                i.matrix.tx = own.x;
                i.matrix.ty = own.y;
                [i.matrix.a, i.matrix.b, i.matrix.c, i.matrix.d] = own.abcd;
                i.visible = own.visible;
                i.cxform.mult[3] = own.alpha;
            }
            if let Some((cell, alpha)) = own.cell
                && let Some(i) = player.get_mut(cell)
            {
                i.cxform.mult[3] = alpha;
            }
        }
        st.own.clear();
        st.now.clear();
        st.slot.clear();
        st.ghosts.clear();
        if let Some((b, x, y)) = st.bio_own.take()
            && let Some(i) = player.get_mut(b)
        {
            i.matrix.tx = x;
            i.matrix.ty = y;
        }
    };

    // The menu (`mcRiderMenu`) and the box over it: kept from the last frame while they're there.
    if st.menu.is_none_or(|m| player.get(m).is_none_or(|i| i.name != "mcRiderMenu")) {
        st.menu = None;
        st.bio = None;
        st.crumb = None;
        st.own.clear();
        st.now.clear();
        st.slot.clear();
        st.lit.clear();
        st.bio_own = None;
        clear_rider_list_early();
        if !on && !gear {
            return;
        }
        let mut stack: Vec<(InstanceId, usize)> = player.levels.values().map(|&r| (r, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            let Some(i) = player.get(id) else { continue };
            if i.name == "mcRiderMenu" {
                st.menu = Some(id);
            }
            if i.name == "mcBioTop" {
                st.bio = Some(id);
            }
            if i.name == "mcCrumb0" && player.get(i.parent.unwrap_or(id)).is_some_and(|p| p.name == "mcBreadcrumb") {
                st.crumb = Some(id);
            }
            if st.menu.is_some() && st.bio.is_some() && st.crumb.is_some() {
                break;
            }
            if depth < 14
                && let InstanceKind::Sprite(s) = &i.kind
            {
                stack.extend(s.children.values().map(|&c| (c, depth + 1)));
            }
        }
    }
    if let Ok(mut m) = RIDER_MENU.lock() {
        *m = st.menu;
    }
    let Some(menu) = st.menu else { return };
    if on && !gear && st.was_gear.take().is_some() {
        st.back_at = Some(std::time::Instant::now());
    }
    // (back from the gear steps: the other cards fading in as the rider's view moves back, and on a little after)
    let fade_in = st.back_at.map_or(1.0, |t| {
        let p = t.elapsed().as_secs_f32() / LIST_FADE_IN_SECS;
        let e = p.clamp(0.0, 1.0);
        e * e * (3.0 - 2.0 * e)
    });
    if fade_in >= 1.0 {
        st.back_at = None;
    }
    // (the gear and perks box fades in with the list)
    let bio = if st.was_gear.is_some() { 0.0 } else { fade_in };
    BIO_FADE.store(bio.to_bits(), std::sync::atomic::Ordering::Relaxed);
    if !on {
        restore(st, player);
        if let Ok(mut m) = CARD_RIDERS.lock() {
            *m = None;
        }
        // (on the way back from the gear steps, before the showcase has the list again: its cards not shown yet (no
        // blink of the whole list before it fades in))
        if st.was_gear.is_some()
            && !gear
            && let Some(InstanceKind::Sprite(s)) = player.get(menu).map(|m| &m.kind)
        {
            let clips: Vec<InstanceId> = s.children.values().copied().collect();
            for c in clips {
                if let Some(i) = player.get_mut(c)
                    && i.name.starts_with("clipInstance")
                {
                    i.visible = false;
                }
            }
        }
        // (past the rider step, and on the way there and back: the list's cards gone at once, the chosen rider's 3-D
        // view moving between its card's place and its own)
        if gear
            && let Some(InstanceKind::Sprite(s)) = player.get(menu).map(|m| &m.kind)
        {
            let clips: Vec<InstanceId> = s.children.values().copied().collect();
            for c in clips {
                if let Some(i) = player.get_mut(c)
                    && i.name.starts_with("clipInstance")
                {
                    i.visible = false;
                }
            }
        }
        return;
    }
    let clips: Vec<InstanceId> = match player.get(menu).map(|m| &m.kind) {
        Some(InstanceKind::Sprite(s)) => s
            .children
            .values()
            .copied()
            .filter(|&c| player.get(c).is_some_and(|i| i.name.starts_with("clipInstance")))
            .collect(),
        _ => return,
    };

    // Each card: its rider, how faded the menu has it, and its left edge (its backing's, in the menu's units, from
    // the card's own place).
    struct Card {
        clip: InstanceId,
        rider: Option<String>,
        alpha: f32,
        /// The card's backing's left and right edges and its top (menu units, from the card's own place, at the
        /// menu's own scale).
        edges: Option<(f32, f32, f32, f32)>,
        /// Its rider known from the menu's own cells (its pictures maybe not in yet), not from its pictures.
        by_item: bool,
        /// Its highlight frames are up; and where its clip, outer and inner cells are in their timelines.
        lit44: bool,
        frames: [u16; 3],
        inner: Option<InstanceId>,
    }
    let child = |player: &Player, of: InstanceId, name: &str| match player.get(of).map(|i| &i.kind) {
        Some(InstanceKind::Sprite(s)) => {
            s.children.values().copied().find(|&k| player.get(k).is_some_and(|k| k.name == name))
        }
        _ => None,
    };
    let cells = RIDER_CELLS.lock().ok().filter(|c| c.1 == order.len()).map(|c| c.0.clone()).unwrap_or_default();
    // (the menu's items trusted as the list's riders when, of its cards showing pictures, most show the rider its item
    // says)
    let trust_items = {
        let (mut agree, mut differ) = (0, 0);
        for &(clip, item) in &cells {
            let portrait = child(player, clip, "mcCell")
                .and_then(|o| child(player, o, "mcCell"))
                .and_then(|n| child(player, n, "mcPortraitContainer"));
            let Some(name) = portrait.and_then(|p| card_portrait_name(lib, player, p)) else { continue };
            if order.get(item) == Some(&name) {
                agree += 1;
            } else {
                differ += 1;
            }
        }
        agree > 0 && agree >= differ * 2
    };
    let mut cards = Vec::new();
    for &c in &clips {
        let Some(i) = player.get(c) else { continue };
        let outer = child(player, c, "mcCell");
        let inner = outer.and_then(|o| child(player, o, "mcCell"));
        let portrait = inner.and_then(|n| child(player, n, "mcPortraitContainer"));
        if !st.own.contains_key(&c) {
            let own = Own {
                x: i.matrix.tx,
                y: i.matrix.ty,
                abcd: [i.matrix.a, i.matrix.b, i.matrix.c, i.matrix.d],
                visible: i.visible,
                alpha: i.cxform.mult[3],
                cell: outer.and_then(|o| player.get(o).map(|oi| (o, oi.cxform.mult[3]))),
            };
            st.own.insert(c, own);
        }
        let alpha = [Some(c), outer, inner, portrait]
            .into_iter()
            .flatten()
            .map(|k| player.get(k).map_or(1.0, |k| k.cxform.mult[3]))
            .product::<f32>();
        let edges = inner.and_then(|n| {
            let bk = child(player, n, "mcBacker")?;
            let kb = player.local_bounds(lib, bk);
            if kb.is_empty() {
                return None;
            }
            let mut m = player.get(bk)?.matrix;
            for k in [inner, outer].into_iter().flatten() {
                m = player.get(k)?.matrix.mul(&m);
            }
            let [a, b, cc, d] = st.own.get(&c).map(|o| o.abcd)?;
            let m = Mat2d { a, b, c: cc, d, tx: 0.0, ty: 0.0 }.mul(&m);
            let b = kb.transform(&m);
            Some((b.x0, b.x1, b.y0, b.y1))
        });
        let pictured = portrait.and_then(|p| card_portrait_name(lib, player, p)).filter(|r| order.contains(r));
        // (a regular rider's card: its portrait's place on it kept for the retro riders' cards, should one of theirs
        // be the first highlighted)
        if let (Some(r), Some(p), Some(n)) = (pictured.as_deref(), portrait, inner)
            && !r.starts_with("dlc")
            && PORTRAIT_FIT.lock().is_ok_and(|f| f.is_none())
            && let Some(f) = child(player, n, "mcInnerframe")
        {
            let pb = player.local_bounds(lib, p);
            let fb = player.local_bounds(lib, f);
            if let (Some(pi), Some(fi)) = (player.get(p), player.get(f))
                && !pb.is_empty()
                && !fb.is_empty()
            {
                let _ = portrait_fit(pb.transform(&pi.matrix), Some(fb.transform(&fi.matrix)), true);
            }
        }
        // (the card's rider: as the menu has put it there (its item), when the menu's items are known to be the list's
        // riders; else by its pictures. Its pictures may be another rider's still (the menu loads a card's pictures
        // only as it comes into its own view): such a card, or one with none yet, counts as having no pictures)
        let item_rider = cells.iter().find(|(k, _)| *k == c).and_then(|e| order.get(e.1).cloned());
        let (rider, by_item) = match (trust_items, item_rider) {
            (true, Some(r)) => {
                let ok = pictured.as_deref() == Some(r.as_str());
                (Some(r), !ok)
            }
            (_, item) => match pictured {
                Some(p) => (Some(p), false),
                None => (item, true),
            },
        };
        let lit44 = inner.is_some_and(|n| match player.get(n).map(|i| &i.kind) {
            Some(InstanceKind::Sprite(s)) => s.children.values().any(|&k| {
                player.get(k).is_some_and(|k| {
                    k.visible && k.cxform.mult[3] > 0.5 && k.character.as_ref().is_some_and(|ch| ch.id == 44)
                }) && !player.local_bounds(lib, k).is_empty()
            }),
            _ => false,
        });
        let frame = |k: Option<InstanceId>| match k.and_then(|k| player.get(k)).map(|i| &i.kind) {
            Some(InstanceKind::Sprite(s)) => s.current_frame,
            _ => 0,
        };
        let frames = [frame(Some(c)), frame(outer), frame(inner)];
        cards.push(Card { clip: c, rider, alpha, edges, by_item, lit44, frames, inner });
    }

    // Each rider of the list on one card: the least faded the menu has for it.
    let mut shown: HashMap<&str, usize> = HashMap::new();
    for (k, card) in cards.iter().enumerate() {
        let Some(r) = card.rider.as_deref() else { continue };
        // (a card showing the rider's pictures before one only waiting for them; then the least faded)
        let better = shown
            .get(r)
            .is_none_or(|&o| (!card.by_item, card.alpha) > (!cards[o].by_item, cards[o].alpha));
        if better && order.iter().any(|o| o == r) {
            shown.insert(r, k);
        }
    }
    // The menu highlights a card (on its own, as the button's pressed) a frame or so before the showcase learns of it
    // (from the game's side): the card it has just lit is the highlighted one from now, so it's never seen lit (green)
    // anywhere but in the highlighted place.
    // (a card is lit when its cells stand elsewhere in their timelines than most cards' do, or when its highlight
    // frames are up while most cards' aren't)
    let ks: Vec<usize> = shown.values().copied().collect();
    let with44 = ks.iter().filter(|&&k| cards[k].lit44).count();
    let use44 = with44 > 0 && with44 * 2 < ks.len();
    let lit_now: std::collections::HashSet<String> = shown
        .iter()
        .filter(|&(_, &k)| use44 && cards[k].lit44)
        .map(|(r, _)| (*r).to_string())
        .collect();
    let newly: Vec<String> = lit_now.difference(&st.lit).cloned().collect();
    let first_look = st.lit.is_empty() && st.slot.is_empty();
    // (only the card next to the highlighted one, either side: one step of the list)
    let at = |r: &str| order.iter().position(|o| o == r);
    let next_to = |a: &str, b: &str| match (at(a), at(b)) {
        (Some(i), Some(j)) => {
            let n = order.len();
            (i + 1) % n == j || (j + 1) % n == i
        }
        _ => false,
    };
    if newly.len() == 1
        && newly[0] != highlighted
        && next_to(&newly[0], &highlighted)
        && !first_look
        && lit_now.len() * 2 < ks.len().max(2)
    {
        if let Ok(mut e) = EARLY.lock() {
            *e = Some((newly[0].clone(), std::time::Instant::now()));
        }
    }
    st.lit = lit_now;
    if let Ok(mut m) = CARD_RIDERS.lock() {
        *m = Some(shown.iter().map(|(r, &k)| (cards[k].clip, ((*r).to_string(), !cards[k].by_item))).collect());
    }
    if let Some(r) = rider_list_early().filter(|r| shown.contains_key(r.as_str())) {
        highlighted = r;
        ssx_core::rider_showcase::set_highlight_portrait(&highlighted);
    }
    let list: Vec<(&String, usize)> =
        order.iter().filter_map(|r| shown.get(r.as_str()).map(|&k| (r, k))).collect();
    let n = list.len();
    let Some(h) = list.iter().position(|(r, _)| **r == highlighted) else { return };
    if n < 2 {
        return;
    }

    // The slots: the menu's own spacing (between its cards' places); the highlighted card's left edge under the
    // EXPLORE breadcrumb's box's (else the box over the list, else where the menu's first card was); as many riders
    // before it (in the list's order) as fit back to the window's left edge, the rest after it, round the list.
    let mut xs: Vec<f32> = st.own.values().map(|o| o.x).collect();
    xs.sort_by(f32::total_cmp);
    let mut gaps: Vec<f32> = xs.windows(2).map(|w| w[1] - w[0]).filter(|g| *g > 1e-3).collect();
    gaps.sort_by(f32::total_cmp);
    let Some(&spacing) = gaps.get(gaps.len() / 2) else { return };
    let root = |player: &Player, id: InstanceId| {
        let mut m = Mat2d::IDENTITY;
        let mut at = Some(id);
        while let Some(i) = at.and_then(|a| player.get(a)) {
            m = i.matrix.mul(&m);
            at = i.parent;
        }
        m
    };
    let Some(top) = player.levels.values().next().and_then(|r| player.get(*r)) else { return };
    let stage = stage_transform(lib.movie(top.movie).stage, viewport, mode);
    let Some(to_menu) = stage.mul(&root(player, menu)).inverse() else { return };
    let Some(menu_from_root) = root(player, menu).inverse() else { return };
    let (window_left, _) = to_menu.apply(0.0, viewport.1 as f32 / 2.0);
    // the breadcrumb's box: its parts but its glow (`mcVapor`, wider than the box), shown ones only
    let crumb_left = st.crumb.and_then(|crumb| {
        let InstanceKind::Sprite(s) = &player.get(crumb)?.kind else { return None };
        let crumb_root = root(player, crumb);
        s.children
            .values()
            .filter_map(|&k| {
                let i = player.get(k)?;
                if i.name == "mcVapor" || !i.visible || i.cxform.mult[3] < 0.05 {
                    return None;
                }
                let b = player.local_bounds(lib, k);
                (!b.is_empty()).then(|| b.transform(&crumb_root.mul(&i.matrix)))
            })
            .map(|b| menu_from_root.apply(b.x0, b.y0).0)
            .reduce(f32::min)
    });
    let bio_left = st.bio.and_then(|bio| {
        let b = player.local_bounds(lib, bio);
        (!b.is_empty()).then(|| {
            let w = b.transform(&root(player, bio));
            menu_from_root.apply(w.x0, w.y0).0
        })
    });
    let left = cards[list[h].1].edges.map_or(0.0, |e| e.0);
    // (a little left of the breadcrumb's box: a fifth of a card's spacing)
    let first = match crumb_left.or(bio_left) {
        Some(x) => x - left - spacing * PIN_NUDGE + PIN_SHIFT_PX * to_menu.a.abs(),
        None => xs.first().copied().unwrap_or(0.0),
    };
    // The gear and perks box over the list: its left edge lined up with the highlighted card's.
    // (to the highlighted card's edge as drawn: its glowing border, in window pixels, as last drawn)
    let drawn = CARD_NOW.lock().ok().and_then(|c| *c).or_else(|| CARD_PREV.lock().ok().and_then(|c| *c));
    if let Some(bio) = st.bio
        && let Some(card) = drawn
    {
        let card_x = card.x0 - BIO_SHIFT_PX;
        let bb = player.local_bounds(lib, bio);
        let parent = player.get(bio).and_then(|i| i.parent).map(|p| stage.mul(&root(player, p)));
        let (parent_a, parent_d) = parent.map_or((1.0, 1.0), |m| (m.a, m.d));
        if !bb.is_empty() && parent_a.abs() > 1e-6 && parent_d.abs() > 1e-6 {
            let w = bb.transform(&stage.mul(&root(player, bio)));
            let d = (card_x - w.x0) / parent_a;
            if let Some(i) = player.get_mut(bio) {
                let (_, _, own_y) = *st.bio_own.get_or_insert((bio, i.matrix.tx, i.matrix.ty));
                i.matrix.tx += d;
                i.matrix.ty = own_y + BIO_DROP_PX / parent_d.abs();
            }
        }
    }
    // The other riders either side of the highlighted card, at the menu's own size and spacing: as many before it (in
    // the list's order) as reach back past the window's left edge, the rest after it, on past the right edge, round the
    // list. Every gap the same, the highlighted card's either side too (as it's drawn, grown).
    let growth = grow_now_or_prev().map_or(0.0, grown_extra);
    let all = order.len();
    let (hl_left, hl_w, hl_top, hl_bottom) =
        cards[list[h].1].edges.map_or((0.0, spacing, 0.0, 0.0), |e| (e.0, e.1 - e.0, e.2, e.3));
    // (the cards seen: inside their backings' clear margins; the gap between two of them)
    let seen_l = hl_left + CARD_MARGIN * hl_w;
    let seen_r = hl_left + (1.0 - CARD_MARGIN) * hl_w;
    let gap = spacing - (seen_r - seen_l);
    // (the highlighted card's edges as drawn (its grown frame, last frame), in the menu's units)
    // (and room either side of it for the scroll arrows)
    let (hx0, hx1, room) = match drawn {
        Some(r) if r.x1 > r.x0 => {
            let (a, _) = to_menu.apply(r.x0, r.y0);
            let (b, _) = to_menu.apply(r.x1, r.y0);
            let (w, _, m) = arrow_size((r.y1 - r.y0).abs());
            (a.min(b), a.max(b), (w + 2.0 * m) * to_menu.a.abs())
        }
        _ => (first + seen_l, first + seen_r + growth, 0.0),
    };
    let (hx0, hx1) = (hx0 - room, hx1 + room);
    // (a card beside the highlighted one shows only once it's clear of the arrows: none seen behind them as it slides
    // out from under the highlighted card, or in under it)
    let zone_alpha = |x: f32| {
        let dist = (hx0 - (x + seen_r)).max((x + seen_l) - hx1);
        (dist / gap.max(1e-3)).clamp(0.0, 1.0)
    };

    let pitch = spacing;
    let others = all.saturating_sub(1);
    let before = (((hx0 - gap - window_left) / pitch).ceil().max(0.0) as usize).min(others);
    let after = others - before;
    // (a card's place (its own, as `first` is the highlighted one's) by its slot)
    let place = |slot: i64| -> f32 {
        if slot >= 1 {
            hx1 + gap + (slot - 1) as f32 * pitch - seen_l
        } else if slot <= -1 {
            hx0 - gap + (slot + 1) as f32 * pitch - seen_r
        } else {
            first
        }
    };

    let ease = (dt * 14.0).min(1.0);
    let mut row: Vec<(i64, String, Option<InstanceId>, f32, f32)> = Vec::with_capacity(all);
    // (places by the whole list's order: a rider the menu has no card for this frame (the menu has fewer cards than
    // the list has riders, and hands them round as it scrolls) is drawn as its card was last seen)
    let order_at = |r: &str| order.iter().position(|o| o == r).unwrap_or(0);
    let h_at = order_at(&highlighted);
    // (a step along the list: the arrow that way glows)
    let mut step = 0i8;
    {
        static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        if let Ok(mut last) = LAST.lock() {
            if let Some(prev) = last.as_deref().filter(|p| **p != *highlighted) {
                let p = order_at(prev);
                let dir = if (p + 1) % all == h_at {
                    1
                } else if (h_at + 1) % all == p {
                    -1
                } else {
                    0
                };
                step = dir;
                if dir != 0
                    && let Ok(mut s) = LIST_SCROLL.lock()
                {
                    *s = Some((dir, std::time::Instant::now()));
                }
            }
            *last = Some(highlighted.clone());
        }
    }
    // (the whole list stepped one place: every card's place moves a place, drawn where it was and easing there
    // together)
    if step != 0 {
        let by = f32::from(step) * pitch;
        for v in st.now.values_mut() {
            *v -= by;
        }
        st.shift = (st.shift + by).clamp(-3.0 * pitch, 3.0 * pitch);
    }
    st.shift *= 1.0 - ease;
    if st.shift.abs() < pitch * 0.002 {
        st.shift = 0.0;
    }
    let slot_of = |rider: &str| {
        let r = (order_at(rider) + all - h_at) % all;
        if r > after { r as i64 - all as i64 } else { r as i64 }
    };
    // (the cards drawn close up, no gaps: a rider with neither a card of the menu's nor a card last seen (one not seen
    // yet) takes no place; the others close up after it, each side)
    let plain: std::collections::HashSet<String> =
        PLAIN_CARDS.lock().map(|c| c.iter().map(|e| e.0.clone()).collect()).unwrap_or_default();
    // (a card with no pictures of its rider (an "NA" card) is never shown but as the highlighted one: its rider shows
    // as its card was last seen with them, else takes no place)
    let pictured_card = |rider: &str| shown.get(rider).is_some_and(|&k| !cards[k].by_item);
    let drawable = |rider: &str| rider == highlighted || pictured_card(rider) || plain.contains(rider);
    // (every rider keeps its place, in the list's order: one with nothing to show yet is drawn as a blank card)
    let (right_n, left_n) = (after as i64, before as i64);
    let mut blanks: Vec<(f32, f32)> = Vec::new();
    for rider in order.iter() {
        let slot = slot_of(rider);
        let card = shown.get(rider.as_str()).copied().filter(|&k| slot == 0 || !cards[k].by_item);
        let target = place(slot);
        let was = st.slot.insert(rider.clone(), slot);
        let now = st.now.entry(rider.clone()).or_insert(target);
        // The highlighted card stands still (put in its place at once). The others slide to theirs; one going round
        // the end of the list starts a slot beyond its new place (off that end) and slides in.
        // (the card coming in from the next place either side: a plain copy of it slides on from there, under the
        // highlighted one)
        if slot == 0 && (was == Some(1) || was == Some(-1)) {
            let from = *now + st.shift;
            st.ghosts.push((rider.clone(), from, from, first, false));
        }
        match was {
            _ if slot == 0 => *now = target,
            None => *now = target,
            // (one going round the end of the list: at its new place at once, sliding on with the rest)
            Some(w) if (w - slot).abs() > 1 => *now = target,
            Some(_) => *now += (target - *now) * ease,
        }
        let x = if slot == 0 { *now } else { *now + st.shift };
        // (a card coming round either end of the list fades in as it slides on: anything past the list's last place
        // on the right, or its first on the left)
        let last = place(right_n.max(1));
        let first_left = place(-left_n.max(1));
        let alpha = if slot >= 1 {
            (1.0 - (x - last).max(0.0) / pitch.max(1e-3)).clamp(0.0, 1.0)
        } else if slot <= -1 {
            (1.0 - (first_left - x).max(0.0) / pitch.max(1e-3)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let alpha = if slot != 0 { alpha * fade_in } else { alpha };
        let alpha = if slot != 0 { alpha * zone_alpha(x) } else { alpha };
        if !drawable(rider) {
            blanks.push((x, alpha));
        }
        let alpha = if drawable(rider) { alpha } else { 0.0 };
        row.push((slot, rider.clone(), card.map(|c| cards[c].clip), x, alpha));
    }
    st.ghosts.retain_mut(|(_, x, _, to, _)| {
        *x += (*to - *x) * ease;
        (*to - *x).abs() > spacing * 0.01
    });
    if let Ok(mut g) = GHOST.lock() {
        for (rider, x, from, to, fade) in &st.ghosts {
            // (both kinds fade as they go: off the list's end, gone halfway (clear of the card sliding into the end
            // place behind it); under the highlighted card as it goes)
            let gone = (*x - *from).abs() / (*to - *from).abs().max(1e-3);
            let alpha = (1.0 - gone * if *fade { 2.0 } else { 1.0 }).clamp(0.0, 1.0) * zone_alpha(*x);
            g.push(((*rider).clone(), *x, alpha));
        }
        // (the riders the menu has no card for this frame: their cards as last seen, in their places)
        for (slot, rider, clip, x, alpha) in &row {
            if clip.is_none() && *slot != 0 && *alpha > 0.001 {
                g.push((rider.clone(), *x, *alpha));
            }
        }
    }
    // (the blank cards, and the plain cards' height for the scroll arrows: a card's top and bottom in the menu's units)
    {
        let own_y = st.own.get(&cards[list[h].1].clip).map_or(0.0, |o| o.y);
        let y0 = own_y + LIST_DROP_PX * to_menu.d.abs() + hl_top;
        let y1 = own_y + LIST_DROP_PX * to_menu.d.abs() + hl_bottom;
        if let Ok(mut b) = BLANKS.lock() {
            b.clear();
            b.extend(blanks.iter().filter(|e| e.1 > 0.001).map(|&(x, a)| [x + seen_l, y0, x + seen_r, y1, a]));
        }
        if let Some(from_menu) = to_menu.inverse() {
            let (_, mid) = from_menu.apply(first, (y0 + y1) * 0.5);
            ARROW_MID_Y.store(mid.to_bits(), std::sync::atomic::Ordering::Relaxed);
        }
    }
    // The highlighted card drawn over the others (a card leaving it slides out from behind it, never seen over it):
    // it takes the topmost of the cards' depths (swapped with the card there).
    // The highlighted card fully highlighted at once (the menu tweens a newly highlighted card's name, level and XP bar
    // from the plain card's blue to green over a few frames): its inner cell put on its last highlight frame.
    {
        let c = &cards[list[h].1];
        if let Some(inner) = c.inner
            && c.lit44
            && c.frames[2] < LIT_FRAME
        {
            player.goto_frame(lib, inner, LIT_FRAME, false);
        }
    }
    let top_card = cards[list[h].1].clip;
    if let Ok(mut t) = GHOST_TOP.lock() {
        *t = Some(top_card);
    }
    if let Some(InstanceKind::Sprite(sp)) = player.get_mut(menu).map(|m| &mut m.kind) {
        let depths: Vec<(i32, InstanceId)> =
            sp.children.iter().filter(|(_, c)| clips.contains(*c)).map(|(&d, &c)| (d, c)).collect();
        if let (Some(&(top, holder)), Some(&(mine, _))) =
            (depths.iter().max_by_key(|e| e.0), depths.iter().find(|e| e.1 == top_card))
            && holder != top_card
        {
            sp.children.insert(top, top_card);
            sp.children.insert(mine, holder);
            if let Some(i) = player.get_mut(top_card) {
                i.depth = top;
            }
            if let Some(i) = player.get_mut(holder) {
                i.depth = mine;
            }
        }
    }
    let placed: Vec<(InstanceId, f32, f32)> = row.iter().filter_map(|c| c.2.map(|clip| (clip, c.3, c.4))).collect();
    let chosen: Vec<InstanceId> = placed.iter().map(|p| p.0).collect();
    let drop = LIST_DROP_PX * to_menu.d.abs();
    for (clip, x, alpha) in placed {
        let own = st.own.get(&clip).map(|o| (o.y, o.abcd));
        if let Some(i) = player.get_mut(clip) {
            i.matrix.tx = x;
            if let Some((y, abcd)) = own {
                i.matrix.ty = y + drop;
                [i.matrix.a, i.matrix.b, i.matrix.c, i.matrix.d] = abcd;
            }
            i.visible = true;
            i.cxform.mult[3] = alpha;
            i.transformed_by_script = true;
        }
        if let Some(cell) = st.own.get(&clip).and_then(|o| o.cell)
            && let Some(i) = player.get_mut(cell.0)
        {
            i.cxform.mult[3] = 1.0;
        }
    }
    for card in &cards {
        if !chosen.contains(&card.clip)
            && let Some(i) = player.get_mut(card.clip)
        {
            i.visible = false;
        }
    }
}

/// Each rider's portrait as last drawn in the rider list (its draws, and where): as the menu moves a rider to another of
/// its cards it loads the rider's pictures into that card, which shows nothing until they're in (a blink); meanwhile
/// the card shows them as they were.
/// (each with the alpha it was drawn at: the card's fading as it was)
#[allow(clippy::type_complexity)]
static PORTRAITS: std::sync::Mutex<Vec<(String, Mat2d, Vec<Command>, f32)>> = std::sync::Mutex::new(Vec::new());

/// `alpha`: how faded the card has the portrait now (the list fading in, say): a portrait drawn as last shown is drawn
/// as faded, and a faded one counts as shown (not as still loading).
fn keep_portrait(
    lib: &Library,
    player: &Player,
    id: InstanceId,
    xf: &Mat2d,
    alpha: f32,
    start: usize,
    out: &mut Vec<Command>,
) {
    let Some(rider) = card_portrait_name(lib, player, id) else { return };
    if alpha <= 0.001 {
        return;
    }
    let shown = out[start..].iter().any(|c| {
        matches!(c, Command::Draw(d) if matches!(d.paint, Paint::Bitmap { .. }) && d.cxform.mult[3] > 0.5 * alpha)
    });
    let Ok(mut cache) = PORTRAITS.lock() else { return };
    if shown {
        let draws = out[start..].to_vec();
        match cache.iter_mut().find(|e| e.0 == rider) {
            Some(e) => {
                e.1 = *xf;
                e.2 = draws;
                e.3 = alpha;
            }
            None => cache.push((rider, *xf, draws, alpha)),
        }
    } else if let Some((_, was, draws, was_alpha)) = cache.iter().find(|e| e.0 == rider)
        && let Some(back) = was.inverse()
    {
        let to = xf.mul(&back);
        let fade = (alpha / was_alpha.max(1e-3)).clamp(0.0, 1.0);
        out.truncate(start);
        for c in draws {
            let mut c = c.clone();
            if let Command::Draw(d) = &mut c {
                d.transform = to.mul(&d.transform);
                d.cxform.mult[3] *= fade;
            }
            out.push(c);
        }
    }
}

/// The rider a card's portrait shows: its pictures' name (`mac` of `mac_top5.dds`).
fn card_portrait_name(lib: &Library, player: &Player, id: InstanceId) -> Option<String> {
    let mut stack = vec![(id, 0usize)];
    while let Some((at, depth)) = stack.pop() {
        let Some(i) = player.get(at) else { continue };
        let file = lib.movie(i.movie).path.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
        if file.ends_with(".dds")
            && (file.contains("_top") || file.contains("_bottom"))
            && let Some((name, _)) = file.split_once('_')
        {
            return Some(name.to_string());
        }
        if depth < 4
            && let InstanceKind::Sprite(s) = &i.kind
        {
            stack.extend(s.children.values().map(|&c| (c, depth + 1)));
        }
    }
    None
}
