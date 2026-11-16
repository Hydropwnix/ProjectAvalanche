//! The menus' 3-D rider showcase (a port addition for the 3D Rider Select mod: `setRiderShowcase`). On the
//! equipment screen's rider step the highlighted rider stands in 3-D in its card (the `mcSelectedRider` card), in
//! place of its portrait: in its last suit and gear (as its run would dress it), doing the level start's idle. The
//! other cards keep their portraits; the portrait stays until the rider has loaded.
//!
//! The riders load on threads of their own (the menus keep running) as soon as the rider step opens: the
//! highlighted one first, then outwards, all of them, so each is there by the time it's highlighted. They stay
//! loaded while the menus run (a rider whose suit or gear changed loads again), and through a run too, hidden, so
//! they're there at once back in the menus. Anywhere else in the menus they load ahead (out of sight), so they're
//! there when the rider step opens.

use std::sync::Mutex;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use bevy::camera::Viewport;
use bevy::ecs::system::SystemParam;
use bevy::mesh::skinning::SkinnedMeshInverseBindposes;
use bevy::prelude::{
    Assets, Camera, Camera3d, ClearColorConfig, Commands, Component, Entity, Image, Mesh, Msaa, Name,
    PerspectiveProjection, Projection, Quat, Query, Res, ResMut, Resource, Time, Transform, UVec2, Vec3, Visibility,
    Window, With, default, info, warn, ButtonInput, Gamepad, GamepadButton, MouseButton, Vec2, Vec4,
};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::core_pipeline::tonemapping::DebandDither;
use bevy::render::storage::ShaderBuffer;
use bevy::render::view::Tonemapping;
use bevy::time::Real;
use bevy::window::PrimaryWindow;
use ssx_ui::game::fe::equipment::showcase::{self as sc, ShowcaseRider};

use super::{GameUiRes, Shell, Stage};
use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::rider::material::{CharacterLightUniform, CharacterMaterial};
use crate::rider::skin::{self as rider_skin, LoadedRider, RiderRig};

/// At most this many riders let go are kept (to take back at once).
const KEPT_RIDERS: usize = 12;
/// At most this many riders load at once.
const LOADING_AT_ONCE: usize = 3;
/// How fast the rider's view moves to a new place (per second: the fraction of the way left goes as 1 - e^-rate·t).
const VIEW_MOVE_RATE: f32 = 9.0;
/// Elsewhere in the menus, the riders to load ahead are worked out again this often (seconds).
const PREFETCH_EVERY_SECS: f64 = 2.0;
/// A rider's origin is its riding height (about its waist): raised so its feet stand on the card's floor.
const LIFT: f32 = 0.95;
/// The animations count time in the game's ticks (60 a second).
const TICKS_PER_SECOND: f32 = 60.0;
/// The riders' frame values (what a run sets every step): ambient occlusion standing in the open, glowing apparel
/// fully lit.
const FRAME_AO: f32 = 0.6;
const FRAME_GLOW: f32 = 1.0;
/// The camera: where it looks (the rider's middle), how far away, its field of view (the card's height takes in the
/// rider and its board).
const LOOK_AT: Vec3 = Vec3::new(-0.12, 0.98, 0.0);
const DISTANCE: f32 = 3.7;
/// A suit's big-head / big-hands perk: the head's or the hands' size (the game's big-head scale).
const BIG_PART_SCALE: f32 = 2.0;
/// Turning the rider: the right stick's rate at full tilt, a mouse drag's per pixel (after a few pixels' slop, so
/// a click stays a click), and how far it tips towards or away from the camera.
const STICK_RADIANS_PER_SECOND: f32 = 3.0;
const STICK_DEAD_ZONE: f32 = 0.2;
const DRAG_RADIANS_PER_PIXEL: f32 = 0.01;
const DRAG_SLOP: f32 = 4.0;
const MAX_TILT: f32 = 1.5;
/// Zoom: the camera's distance changes this much a wheel notch, and a second at a full trigger; its limits (a
/// fraction of the default distance). Pan: metres a mouse pixel and a second of full stick (at the default zoom),
/// and how far from the rider it may go.
const ZOOM_PER_NOTCH: f32 = 1.12;
const ZOOM_PER_SECOND: f32 = 2.5;
const MIN_ZOOM: f32 = 0.3;
const MAX_ZOOM: f32 = 1.6;
const DRAG_PAN_PER_PIXEL: f32 = 0.004;
const STICK_PAN_PER_SECOND: f32 = 1.2;
const MAX_PAN: f32 = 1.2;
const FOV: f32 = 0.62;
/// The camera's order: over the menus (0), under the level's HUD (10).
const ORDER: isize = 5;

#[derive(Component)]
pub(super) struct ShowcaseCamera;

#[derive(Component)]
pub(super) struct ShowcaseRoot;

type Loaded = Result<LoadedRider, String>;

enum Slot {
    Empty,
    Loading(Mutex<Receiver<Loaded>>),
    Ready(Box<Shown>),
    Failed,
}

struct Shown {
    root: Entity,
    rig: RiderRig,
}

#[derive(Resource, Default)]
pub(super) struct ShowcaseState {
    keys: Vec<u32>,
    riders: Vec<ShowcaseRider>,
    slots: Vec<Slot>,
    camera: Option<Entity>,
    light: Option<GameLightAssets>,
    /// The riders' frame values written since the showcase opened (a run in between writes its own).
    frame_written: bool,
    /// The rider step was showing last frame.
    was_active: bool,
    /// Elsewhere in the menus: when the riders the rider step will show were last worked out (to load them ahead).
    prefetched_at: Option<f64>,
    /// In a run: the riders kept loaded but hidden (so they're there at once back in the menus).
    parked: bool,
    /// When a run started from the rider's view (it stays till the loading screen's up).
    starting_since: Option<f64>,
    /// The rider's view on the screen as drawn (fractions of the window): moving to where the menus have it.
    view: Option<[f32; 4]>,
    /// The view's long move under way (where from, how long it's been going).
    view_move: Option<([f32; 4], f32)>,
    /// On the gear steps: the chosen rider as dressed, and when that was worked out.
    chosen: Option<ShowcaseRider>,
    chosen_at: f64,
    /// The player's view of the highlighted rider: turned (about up, then across, radians), zoomed (the camera's
    /// distance, a fraction of its own) and panned (metres across and up); and which rider that is. A newly
    /// highlighted rider, and the rider step opened again, start at the default view.
    spin: Vec2,
    zoom: f32,
    pan: Vec2,
    spin_for: Option<usize>,
    /// A mouse drag on the rider: the left button turns it, the right pans.
    dragging: Option<MouseButton>,
    drag_moved: f32,
    /// Whether the view was the gear steps' (the player turns the rider only there; it starts at the default view
    /// each time they open, and back on the rider step).
    in_gear: bool,
    /// The rider the player's view is of (its key: the same rider in another suit keeps the view).
    spin_key: Option<u32>,
    /// On the gear steps: the hovered suit the chosen rider was last dressed in (None: as equipped), and the rider
    /// shown (while one in another suit loads, the last one stays).
    chosen_preview: Option<(usize, usize, (u32, u8))>,
    gear_shown: Option<usize>,
    /// The gear list's suits or boards on the chosen rider, loaded ahead (their riders' places, the hovered one's
    /// first), and what they were worked out for.
    previews: Vec<usize>,
    previews_key: String,
    /// Riders let go (another event's loadout, say), kept a while (the latest few): taken back at once when wanted
    /// again, and shown in place of the same rider in other gear till that one has loaded (the model, not the
    /// portrait, meanwhile).
    stale: Vec<(ShowcaseRider, Box<Shown>)>,
    /// How many of `riders` are the list's (those after: the chosen rider in other kinds of event's loadouts, loaded
    /// ahead).
    list_len: usize,
}

#[derive(SystemParam)]
pub(super) struct ShowcaseAssets<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    images: ResMut<'w, Assets<Image>>,
    objects: ResMut<'w, Assets<ObjectMaterial>>,
    foreground: ResMut<'w, Assets<ForegroundMaterial>>,
    characters: ResMut<'w, Assets<CharacterMaterial>>,
    bindposes: ResMut<'w, Assets<SkinnedMeshInverseBindposes>>,
    buffers: ResMut<'w, Assets<ShaderBuffer>>,
}

impl ShowcaseState {
    fn clear(&mut self, commands: &mut Commands) {
        for slot in self.slots.drain(..) {
            if let Slot::Ready(s) = slot {
                commands.entity(s.root).despawn();
            }
        }
        for (_, s) in self.stale.drain(..) {
            commands.entity(s.root).despawn();
        }
        self.list_len = 0;
        if let Some(c) = self.camera.take() {
            commands.entity(c).despawn();
        }
        self.keys.clear();
        self.riders.clear();
        self.frame_written = false;
        self.was_active = false;
        self.prefetched_at = None;
        self.parked = false;
        self.chosen = None;
        self.view = None;
    }

    /// In a run (or on its way): the riders kept, hidden, so the rider step has them at once afterwards (they'd take a
    /// moment each to load again).
    fn park(&mut self, commands: &mut Commands, cameras: &mut Query<&mut Camera>) {
        self.hide(cameras);
        self.frame_written = false;
        if self.parked {
            return;
        }
        self.parked = true;
        for root in self.roots() {
            commands.entity(root).insert(Visibility::Hidden);
        }
    }

    /// Back in the menus: the riders kept through a run shown again (they're still out of sight till highlighted),
    /// and the riders to show worked out again at once (a rider's gear may have changed).
    fn unpark(&mut self, commands: &mut Commands) {
        if !self.parked {
            return;
        }
        self.parked = false;
        self.prefetched_at = None;
        for root in self.roots() {
            commands.entity(root).insert(Visibility::Inherited);
        }
    }

    fn roots(&self) -> Vec<Entity> {
        let ready = self.slots.iter().filter_map(|s| if let Slot::Ready(s) = s { Some(s.root) } else { None });
        ready.chain(self.stale.iter().map(|(_, s)| s.root)).collect()
    }

    /// Off screen: the camera off and the portrait back; the riders kept (for the next time).
    fn hide(&mut self, cameras: &mut Query<&mut Camera>) {
        if let Some(c) = self.camera
            && let Ok(mut cam) = cameras.get_mut(c)
        {
            cam.is_active = false;
        }
        self.frame_written = false;
        self.was_active = false;
        self.view = None;
    }

    /// The list's riders (and `extras` after them, loaded ahead), keeping those loaded already whose suit and gear are
    /// the same (or kept from before).
    fn take_riders(&mut self, mut riders: Vec<ShowcaseRider>, extras: Vec<ShowcaseRider>, commands: &mut Commands) {
        self.list_len = riders.len();
        for e in extras {
            if !riders.contains(&e) {
                riders.push(e);
            }
        }
        let mut old: Vec<(ShowcaseRider, Slot)> =
            std::mem::take(&mut self.riders).into_iter().zip(std::mem::take(&mut self.slots)).collect();
        let mut slots = Vec::with_capacity(riders.len());
        for r in &riders {
            let slot = match old.iter().position(|(o, s)| o == r && !matches!(s, Slot::Failed)) {
                Some(at) => old.swap_remove(at).1,
                None => match self.stale.iter().position(|(o, _)| o == r) {
                    Some(at) => Slot::Ready(self.stale.remove(at).1),
                    None => Slot::Empty,
                },
            };
            slots.push(slot);
        }
        for (r, slot) in old {
            if let Slot::Ready(s) = slot {
                self.stale.push((r, s));
            }
        }
        // (the latest few kept)
        while self.stale.len() > KEPT_RIDERS {
            let (_, s) = self.stale.remove(0);
            commands.entity(s.root).despawn();
        }
        self.riders = riders;
        self.slots = slots;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn showcase_system(
    mut state: ResMut<ShowcaseState>,
    shell: Res<Shell>,
    mut ui: ResMut<GameUiRes>,
    time: Res<Time<Real>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut assets: ShowcaseAssets,
    mut transforms: Query<&mut Transform>,
    mut cameras: Query<&mut Camera>,
    (mouse, motion, scroll, pads): (
        Res<ButtonInput<MouseButton>>,
        Res<AccumulatedMouseMotion>,
        Res<AccumulatedMouseScroll>,
        Query<&Gamepad>,
    ),
) {
    let menus = ssx_core::rider_showcase::enabled() && shell.stage == Stage::Menus;
    // (a run starting from the rider's view: everything kept as it is, the rider showing, till the loading screen
    // covers it, so the card's portrait doesn't show for a moment in between (or a few seconds at most))
    let starting = state.was_active
        && ssx_core::rider_showcase::enabled()
        && shell.stage != Stage::Level
        && !ssx_ui::render::LOADING_SCREEN.load(std::sync::atomic::Ordering::Relaxed)
        && (shell.stage != Stage::Menus || ui.fe.as_ref().is_some_and(|f| f.launch.is_some()));
    if starting {
        let now = time.elapsed_secs_f64();
        if now - *state.starting_since.get_or_insert(now) < 3.0 {
            return;
        }
    } else {
        state.starting_since = None;
    }
    // (the Custom Event Menus mod: the retro riders given a suit and a board of their own to start in, once)
    static RETRO_GIVEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if menus
        && !RETRO_GIVEN.load(std::sync::atomic::Ordering::Relaxed)
        && let Some(fe) = ui.0.fe.as_mut()
    {
        RETRO_GIVEN.store(true, std::sync::atomic::Ordering::Relaxed);
        for line in fe.profile.give_retro_defaults(&fe.career) {
            info!("{line}");
        }
    }
    // (past the rider step, on the gear steps: the chosen rider in a view of its own)
    let gear = menus && ui.fe.as_ref().is_some_and(sc::gear_view);
    let rider_step = menus && ui.fe.as_ref().is_some_and(sc::active);
    // (on the equipment screen between its steps (the rider chosen, or the way back): the rider stays in view, moving
    // between its card's place and its gear view's)
    let between = menus
        && !gear
        && !rider_step
        && state.was_active
        && state.spin_for.is_some()
        && ui.fe.as_ref().is_some_and(|fe| fe.screen == ssx_ui::game::fe::screens::id::EQUIPMENT);
    ssx_core::rider_showcase::set_gear_view(gear || between);
    let active = gear || rider_step || between;
    if !active {
        ssx_core::rider_showcase::set_portrait_hidden(false);
        ssx_core::rider_showcase::set_highlight_portrait("");
        ssx_core::rider_showcase::set_list_order(Vec::new());
        let on = ssx_core::rider_showcase::enabled();
        if !on && (state.camera.is_some() || !state.slots.is_empty()) {
            info!("showcase: off (the riders let go)");
            state.clear(&mut commands);
        } else if on && !menus {
            // (in a run: the riders kept, hidden, for the rider step afterwards)
            if !state.parked && !state.slots.is_empty() {
                info!("showcase: the riders kept through the run (hidden)");
            }
            state.park(&mut commands, &mut cameras);
        } else if state.was_active {
            state.hide(&mut cameras);
        }
        if menus {
            state.unpark(&mut commands);
        }
        // Elsewhere in the menus: the rider step's riders loaded ahead, out of sight (worked out again now and
        // then: the profile, a rider's gear may change meanwhile; those already loaded and the same are kept).
        let g = &mut ui.0;
        if menus && let Some(fe) = g.fe.as_ref() {
            let now = time.elapsed_secs_f64();
            if state.prefetched_at.is_none_or(|t| now - t >= PREFETCH_EVERY_SECS) {
                state.prefetched_at = Some(now);
                let riders = sc::prefetch(fe, &mut g.loc);
                // (and the rider last played in each other kind of event's loadout)
                let extras = sc::variants(fe, &mut g.loc, sc::last_played(fe), &riders);
                let all: Vec<ShowcaseRider> = riders.iter().chain(&extras).cloned().collect();
                if !riders.is_empty() && all != state.riders {
                    info!("showcase: {} riders loading ahead ({} in other events' loadouts)", riders.len(), extras.len());
                    state.keys.clear();
                    state.take_riders(riders, extras, &mut commands);
                }
            }
            let highlighted = 0;
            load_and_spawn(&mut state, &shell, &mut commands, &mut assets, highlighted, false, None);
        }
        return;
    }
    let dt = time.delta_secs().min(0.1);

    // The list's riders (built again when the list changes: another drop, a rider bought).
    let g = &mut ui.0;
    let Some(fe) = g.fe.as_ref() else { return };
    let gear_rider = if between {
        state.spin_for
    } else if gear {
        // (the chosen rider as now dressed, worked out a few times a second: it's dressed anew as gear is equipped)
        let now = time.elapsed_secs_f64();
        // (a hovered suit: the rider in it at once)
        let preview = fe.equipment.preview;
        if state.chosen.is_none() || now - state.chosen_at >= 0.25 || state.chosen_preview != preview {
            state.chosen_at = now;
            state.chosen_preview = preview;
            state.chosen = sc::chosen(fe, &mut g.loc);
        }
        // (the list's suits or boards on the rider, loaded ahead; those no longer near let go)
        let key = sc::previews_key(fe);
        if key != state.previews_key {
            state.previews_key = key;
            let riders = sc::previews(fe, &mut g.loc);
            let mut at = Vec::with_capacity(riders.len());
            for r in riders {
                let i = match state.riders.iter().position(|x| *x == r) {
                    Some(i) => i,
                    None => {
                        state.riders.push(r);
                        state.slots.push(Slot::Empty);
                        state.riders.len() - 1
                    }
                };
                at.push(i);
            }
            let keep: Vec<usize> = state.gear_shown.into_iter().collect();
            let chosen = state.chosen.clone();
            for old in std::mem::take(&mut state.previews) {
                if at.contains(&old) || keep.contains(&old) || chosen.as_ref() == state.riders.get(old) {
                    continue;
                }
                if let Some(slot) = state.slots.get_mut(old)
                    && let Slot::Ready(s) = std::mem::replace(slot, Slot::Empty)
                {
                    commands.entity(s.root).despawn();
                } else if let Some(slot) = state.slots.get_mut(old)
                    && matches!(slot, Slot::Loading(_))
                {
                    *slot = Slot::Empty;
                }
            }
            state.previews = at;
        }
        let Some(c) = state.chosen.clone() else { return };
        // (back on the rider step its list is worked out again)
        state.keys.clear();
        Some(match state.riders.iter().position(|r| *r == c) {
            Some(i) => i,
            None => {
                state.riders.push(c);
                state.slots.push(Slot::Empty);
                state.riders.len() - 1
            }
        })
    } else {
        state.chosen = None;
        state.previews.clear();
        state.previews_key.clear();
        None
    };
    let gear = gear || between;
    let keys = sc::keys(fe);
    let highlighted = sc::highlighted(&fe.equipment);
    // (each time the step opens, or its list changes: what each rider wears may have changed meanwhile)
    if gear_rider.is_none() && (keys != state.keys || !state.was_active) {
        let Some(show) = sc::showcase(fe, &mut g.loc) else { return };
        // (and the chosen rider in each other kind of event's loadout, loaded ahead)
        let extras = sc::variants(fe, &mut g.loc, sc::chosen_rider(fe), &show.riders);
        info!("showcase: {} riders, {} highlighted ({} in other events' loadouts)", show.riders.len(), show.highlighted, extras.len());
        state.keys = keys;
        state.take_riders(show.riders, extras, &mut commands);
    }
    let entering = !state.was_active;
    state.was_active = true;
    let n = state.riders.len();
    if n == 0 {
        return;
    }
    let highlighted = gear_rider.unwrap_or(highlighted).min(n - 1);
    // (the list's menu moves its highlight as the button's pressed, the game a step later: the menu's rider shown till
    // the game has it too)
    let highlighted = match ssx_ui::render::rider_list_early().filter(|_| !gear) {
        Some(r) if state.riders[highlighted].portrait == r => {
            ssx_ui::render::clear_rider_list_early();
            highlighted
        }
        Some(r) => state.riders.iter().position(|x| x.portrait == r).unwrap_or(highlighted),
        None => highlighted,
    };
    // (on the gear steps, the rider in another suit loading: the one shown stays till it's ready)
    let wanted = highlighted;
    let highlighted = if gear {
        let ready = |i: usize| matches!(state.slots.get(i), Some(Slot::Ready(_)));
        let (wanted_ready, shown) = (ready(wanted), state.gear_shown.filter(|&i| i < n && ready(i)));
        if wanted_ready {
            state.gear_shown = Some(wanted);
            wanted
        } else {
            shown.filter(|&i| state.riders[i].key == state.riders[wanted].key).unwrap_or(wanted)
        }
    } else {
        state.gear_shown = None;
        wanted
    };

    // The camera, drawing over the cards' band.
    if state.camera.is_none() {
        let id = commands
            .spawn((
                Name::new("rider showcase camera"),
                Camera3d::default(),
                Camera { order: ORDER, clear_color: ClearColorConfig::None, ..default() },
                Projection::Perspective(PerspectiveProjection { fov: FOV, ..default() }),
                Tonemapping::None,
                DebandDither::Disabled,
                Msaa::Off,
                Transform::from_translation(LOOK_AT + Vec3::Z * DISTANCE).looking_at(LOOK_AT, Vec3::Y),
                ShowcaseCamera,
            ))
            .id();
        state.camera = Some(id);
    }
    // In the highlighted card's portrait (where the menus drew it last), when the rider there is ready.
    // (on the gear steps the menus draw the chosen rider's view where its card was: no list to lay out)
    if gear {
        ssx_core::rider_showcase::set_highlight_portrait("");
        ssx_core::rider_showcase::set_list_order(Vec::new());
    } else {
        ssx_core::rider_showcase::set_highlight_portrait(&state.riders[highlighted].portrait);
        let list = state.list_len.min(state.riders.len());
        ssx_core::rider_showcase::set_list_order(state.riders[..list].iter().map(|r| r.portrait.clone()).collect());
    }
    // (the highlighted rider not loaded in its new gear yet: the same rider in its old gear shown meanwhile)
    let mut ready = matches!(state.slots.get(highlighted), Some(Slot::Ready(_)));
    let stale_at =
        if ready { None } else { state.stale.iter().rposition(|(r, _)| r.key == state.riders[highlighted].key) };
    ready |= stale_at.is_some();
    let slot = ssx_core::rider_showcase::slot();
    // (the highlighted card's portrait never shown: the card shows the rider in 3-D only, empty for the moment its
    // model takes to load (back from a run, say))
    ssx_core::rider_showcase::set_portrait_hidden(!gear);
    if let (Some(c), Ok(w)) = (state.camera, windows.single())
        && let Ok(mut cam) = cameras.get_mut(c)
    {
        let (ww, wh) = (w.physical_width().max(1) as f32, w.physical_height().max(1) as f32);
        // (the view moves smoothly to where the menus have it now: from the list's card to the gear view's place and
        // back; between the steps, where the menus have no place for it, it stays where it was)
        let target = slot.filter(|_| ready);
        state.view = match (state.view, target) {
            (Some(v), Some(t)) => {
                let mut mv = state.view_move;
                let n = ssx_core::rider_showcase::view_step(v, t, dt, 0.03, VIEW_MOVE_RATE, &mut mv);
                state.view_move = mv;
                Some(if mv.is_none() && n.iter().zip(&t).all(|(a, b)| (a - b).abs() < 0.0005) { t } else { n })
            }
            (None, Some(t)) => Some(t),
            // (no place for it this frame (between the steps, or the list not laid out yet on the way back): it stays)
            (Some(v), None) if ready => Some(v),
            _ => None,
        };
        if state.view.is_none() {
            state.view_move = None;
        }
        let want = state.view.and_then(|[x0, y0, x1, y1]| {
            let (px, py) = ((x0.clamp(0.0, 1.0) * ww) as u32, (y0.clamp(0.0, 1.0) * wh) as u32);
            let (qx, qy) = ((x1.clamp(0.0, 1.0) * ww) as u32, (y1.clamp(0.0, 1.0) * wh) as u32);
            (qx > px + 1 && qy > py + 1).then(|| Viewport {
                physical_position: UVec2::new(px, py),
                physical_size: UVec2::new(qx - px, qy - py),
                ..default()
            })
        });
        cam.is_active = want.is_some();
        if let Some(want) = want
            && cam.viewport.as_ref().is_none_or(|v| {
                v.physical_position != want.physical_position || v.physical_size != want.physical_size
            })
        {
            cam.viewport = Some(want);
        }
    }

    // (on the gear steps only the rider shown and the list's previews load (the hovered one first): the rider
    // step's others wait)
    let only: Option<Vec<usize>> =
        gear.then(|| std::iter::once(wanted).chain(state.previews.iter().copied()).collect());
    load_and_spawn(&mut state, &shell, &mut commands, &mut assets, wanted, entering, only.as_deref());

    // The riders' frame values (a run writes them each step; none runs here).
    if !state.frame_written
        && let Some(mut buffer) = assets.buffers.get_mut(&crate::rider::material::RIDER_FRAME)
    {
        let floats = [FRAME_AO, FRAME_GLOW, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let bytes: Vec<u8> = floats.iter().flat_map(|f| f.to_le_bytes()).collect();
        buffer.clear();
        buffer.extend_from_slice(&bytes);
        state.frame_written = true;
    }

    // The player's view: the right stick turns the rider (with LB held, it pans), the triggers zoom; with the mouse, on
    // the rider: a left drag turns it, a right drag pans, the wheel zooms.
    let key = state.riders[highlighted].key;
    if state.spin_key != Some(key) || entering || state.in_gear != gear {
        state.spin = Vec2::ZERO;
        state.zoom = 1.0;
        state.pan = Vec2::ZERO;
        state.dragging = None;
    }
    state.spin_key = Some(key);
    state.spin_for = Some(highlighted);
    state.in_gear = gear;
    // (the rider step's rider stays as it stands: turned, panned and zoomed on the gear steps only)
    let controls = gear;
    let over = windows.single().ok().and_then(|w| {
        let c = w.cursor_position()?;
        (w.width() > 0.0 && w.height() > 0.0).then(|| (c.x / w.width(), c.y / w.height()))
    });
    let on_model = over.is_some_and(|(x, y)| ssx_core::rider_showcase::over_model(x, y));
    for b in [MouseButton::Left, MouseButton::Right] {
        if controls && mouse.just_pressed(b) && on_model && state.dragging.is_none() {
            state.dragging = Some(b);
            state.drag_moved = 0.0;
            ssx_core::rider_showcase::set_dragged(false);
        }
    }
    if state.dragging.is_some_and(|b| !mouse.pressed(b)) {
        state.dragging = None;
    }
    if let Some(b) = state.dragging
        && motion.delta != Vec2::ZERO
    {
        state.drag_moved += motion.delta.length();
        if state.drag_moved > DRAG_SLOP {
            ssx_core::rider_showcase::set_dragged(true);
            if b == MouseButton::Left {
                state.spin += motion.delta * DRAG_RADIANS_PER_PIXEL;
            } else {
                let metres = DRAG_PAN_PER_PIXEL * state.zoom;
                state.pan += Vec2::new(-motion.delta.x, motion.delta.y) * metres;
            }
        }
    }
    if controls && on_model && scroll.delta.y != 0.0 {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        state.zoom *= ZOOM_PER_NOTCH.powf(-notches);
    }
    let pad = |f: &dyn Fn(&Gamepad) -> f32| pads.iter().map(f).fold(0.0f32, |a, v| if v.abs() > a.abs() { v } else { a });
    let stick = pads.iter().map(|p| p.right_stick()).fold(Vec2::ZERO, |a, s| if s.length() > a.length() { s } else { a });
    let panning = pads.iter().any(|p| p.pressed(GamepadButton::LeftTrigger));
    if controls && stick.length() > STICK_DEAD_ZONE {
        if panning {
            let zoom = state.zoom;
            state.pan += stick * STICK_PAN_PER_SECOND * zoom * dt;
        } else {
            state.spin += Vec2::new(stick.x, -stick.y) * STICK_RADIANS_PER_SECOND * dt;
        }
    }
    let zoom_in = pad(&|p| p.get(GamepadButton::RightTrigger2).unwrap_or(0.0));
    let zoom_out = pad(&|p| p.get(GamepadButton::LeftTrigger2).unwrap_or(0.0));
    let triggers = zoom_in.max(0.0) - zoom_out.max(0.0);
    if controls && triggers.abs() > 0.1 {
        state.zoom *= ZOOM_PER_SECOND.powf(-triggers * dt);
    }
    state.spin.y = state.spin.y.clamp(-MAX_TILT, MAX_TILT);
    state.zoom = state.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    state.pan = state.pan.clamp(Vec2::splat(-MAX_PAN), Vec2::splat(MAX_PAN));
    if let Some(c) = state.camera
        && let Ok(mut t) = transforms.get_mut(c)
    {
        let target = LOOK_AT + state.pan.extend(0.0);
        *t = Transform::from_translation(target + Vec3::Z * DISTANCE * state.zoom).looking_at(target, Vec3::Y);
    }
    let turn = Quat::from_rotation_x(state.spin.y) * Quat::from_rotation_y(state.spin.x);
    let stand = LOOK_AT + turn * (Vec3::new(0.0, LIFT, 0.0) - LOOK_AT);

    // The highlighted rider in its card, posed; the others (loaded, waiting) out of sight.
    for (i, slot) in state.slots.iter_mut().enumerate() {
        let Slot::Ready(s) = slot else { continue };
        let here = i == highlighted;
        if let Ok(mut t) = transforms.get_mut(s.root) {
            *t = Transform {
                translation: if here { stand } else { Vec3::new(0.0, -100.0, 0.0) },
                rotation: if here { turn } else { Quat::IDENTITY },
                scale: Vec3::ONE,
            };
        }
        if here {
            rider_skin::animate_idle(&mut s.rig, dt * TICKS_PER_SECOND, &mut transforms);
        }
    }
    // (and once the rider's loaded in its new gear, that one)
    let loaded = matches!(state.slots.get(highlighted), Some(Slot::Ready(_)));
    for (i, (_, s)) in state.stale.iter_mut().enumerate() {
        let here = stale_at == Some(i) && !loaded;
        if let Ok(mut t) = transforms.get_mut(s.root) {
            *t = Transform {
                translation: if here { stand } else { Vec3::new(0.0, -100.0, 0.0) },
                rotation: if here { turn } else { Quat::IDENTITY },
                scale: Vec3::ONE,
            };
        }
        if here {
            rider_skin::animate_idle(&mut s.rig, dt * TICKS_PER_SECOND, &mut transforms);
        }
    }
}

/// The riders not loaded yet: loading (a few at once, the highlighted one first, then outwards); those loaded: spawned
/// (out of sight, till highlighted on the rider step).
fn load_and_spawn(
    state: &mut ShowcaseState,
    shell: &Shell,
    commands: &mut Commands,
    assets: &mut ShowcaseAssets,
    highlighted: usize,
    entering: bool,
    only: Option<&[usize]>,
) {
    let n = state.riders.len();
    if n == 0 {
        return;
    }
    let highlighted = highlighted.min(n - 1);
    // Loading: every rider, the highlighted one first, then outwards (or only these, in this order).
    let order: Vec<usize> = match only {
        Some(o) => o.iter().copied().filter(|&i| i < n).collect(),
        None => {
            // (the list's riders by their distance round it; the chosen rider in other events' loadouts just after
            // the highlighted one)
            let list = state.list_len.clamp(1, n);
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by_key(|&i| {
                if i >= list {
                    1
                } else if i == highlighted {
                    0
                } else {
                    2 + i.abs_diff(highlighted).min(list.saturating_sub(i.abs_diff(highlighted)))
                }
            });
            order
        }
    };
    let mut loading = state.slots.iter().filter(|s| matches!(s, Slot::Loading(_))).count();
    for i in order {
        if loading >= LOADING_AT_ONCE {
            break;
        }
        if matches!(state.slots[i], Slot::Empty) {
            let rider = state.riders[i].clone();
            let big_root = shell.big_root.clone();
            let (tx, rx) = channel();
            let spawned = std::thread::Builder::new().name(format!("showcase {}", rider.name)).spawn(move || {
                let _ = tx.send(load(&big_root, &rider));
            });
            match spawned {
                Ok(_) => {
                    state.slots[i] = Slot::Loading(Mutex::new(rx));
                    loading += 1;
                }
                Err(e) => {
                    warn!("showcase: no loader thread: {e}");
                    state.slots[i] = Slot::Failed;
                }
            }
        }
    }

    // Finished loads: spawned.
    // (the light list and textures a level makes: empty ones if no level has run yet, or riders can't draw)
    // (in daylight: a run's lighting, a night drop's, left in the shared light list is put back to the viewer's)
    if entering {
        crate::render::lighting::local::reset_empty(&mut assets.buffers, &mut assets.images);
    } else {
        crate::render::lighting::local::ensure_empty(&mut assets.buffers, &mut assets.images);
    }
    let light = state.light.get_or_insert_with(|| GameLightAssets::viewer(&mut assets.images)).clone();
    let rider_light = CharacterLightUniform::viewer_default();
    for i in 0..n {
        let got = match &state.slots[i] {
            Slot::Loading(m) => match m.lock() {
                Ok(rx) => rx.try_recv(),
                Err(_) => Err(TryRecvError::Disconnected),
            },
            _ => continue,
        };
        match got {
            Ok(Ok(rider)) => {
                let name = state.riders[i].name.clone();
                info!("showcase: {name} loaded ({}, {:.2}s)", rider.summary, rider.load_secs);
                info!("showcase: {name}: {}", state.riders[i].note);
                let root = commands
                    .spawn((
                        Name::new(format!("showcase {name}")),
                        ShowcaseRoot,
                        Transform::from_xyz(0.0, -100.0, 0.0),
                        Visibility::default(),
                    ))
                    .id();
                let rig = rider_skin::spawn(
                    commands,
                    root,
                    rider,
                    None,
                    &mut assets.meshes,
                    &mut assets.images,
                    &light,
                    &rider_light,
                    &mut assets.objects,
                    &mut assets.foreground,
                    &mut assets.characters,
                    &mut assets.bindposes,
                );
                // Its materials read the riders' frame values (the glow), as a run's rider's do; only a rider in a
                // glowing suit glows (else its gloves' and pack's glow stays dark).
                let glows: Vec<(String, Vec3)> = rig
                    .character_materials
                    .iter()
                    .zip(&rig.character_slots)
                    .filter_map(|(h, slot)| {
                        let g = assets.characters.get(h)?.extension.params.glow.truncate();
                        (g != Vec3::ZERO).then(|| (slot.clone(), g))
                    })
                    .collect();
                info!("showcase: {name}: suit {:?}, glowing parts {glows:?}", state.riders[i].suit);
                // A glowing suit (its name ends "Glowing Suit") glows; the others' glowing parts (the gloves and pack
                // every rider has, in its colour) stay dark.
                let glowing_suit = state.riders[i].glowing;
                // A suit's big-head / big-hands perk, shown.
                let mut rig = rig;
                let big = |on: bool| if on { BIG_PART_SCALE } else { 1.0 };
                rider_skin::set_big(&mut rig, big(state.riders[i].big_head), big(state.riders[i].big_hands));
                for handle in &rig.character_materials {
                    if let Some(mut m) = assets.characters.get_mut(handle) {
                        m.extension.rider.frame.x = 1.0;
                        if !glowing_suit {
                            m.extension.params.glow = Vec4::ZERO;
                        } else if m.extension.params.glow != Vec4::ZERO
                            && let Some(img) = m.base.base_color_texture.as_ref().and_then(|t| assets.images.get(t))
                            && !has_glow_mask(img)
                        {
                            // (a part whose picture has no glow pattern (no alpha: a glowing suit's plain pants)
                            // would glow all over: it doesn't)
                            m.extension.params.glow = Vec4::ZERO;
                        }
                    }
                }
                info!("showcase: {name}: {}", if glowing_suit { "a glowing suit" } else { "no glowing suit (its gear's glow off)" });
                state.slots[i] = Slot::Ready(Box::new(Shown { root, rig }));
            }
            Ok(Err(e)) => {
                warn!("showcase: {} not loaded: {e}", state.riders[i].name);
                state.slots[i] = Slot::Failed;
            }
            Err(TryRecvError::Disconnected) => state.slots[i] = Slot::Failed,
            Err(TryRecvError::Empty) => {}
        }
    }

}

/// Whether a part's picture carries a glow pattern (in its alpha): a picture with none (block-compressed without alpha,
/// or every texel opaque) marks no glow, and would glow all over.
fn has_glow_mask(img: &Image) -> bool {
    use bevy::render::render_resource::TextureFormat as F;
    match img.texture_descriptor.format {
        F::Bc1RgbaUnorm | F::Bc1RgbaUnormSrgb => false,
        F::Rgba8Unorm | F::Rgba8UnormSrgb | F::Bgra8Unorm | F::Bgra8UnormSrgb => {
            img.data.as_ref().is_none_or(|d| d.chunks_exact(4).any(|p| p[3] < 240))
        }
        _ => true,
    }
}

/// A rider as its run would load it: the character (or its stand-in), then dressed in its suit and gear.
fn load(big_root: &std::path::Path, r: &ShowcaseRider) -> Loaded {
    let keys: Vec<u32> = r.items.iter().map(|&(_, d)| d).collect();
    let rr = ssx_assets::content::characters::ride::ride_rider(big_root, &r.name, None, &keys, false)?;
    let mut rider = rider_skin::load_rider(big_root, None, &rr.request).map_err(|e| format!("{e:#}"))?;
    let loadout = ssx_assets::character::Loadout::from_setup(&r.items, r.lamp);
    if loadout != ssx_assets::character::Loadout::default() {
        rider.dress(&loadout).map_err(|e| format!("its gear: {e:#}"))?;
    }
    Ok(rider)
}
