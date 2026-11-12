mod frame;
pub mod keys;
pub(crate) mod markers;
mod session;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use ssx_content::{ContentId, Kind};

use crate::value::{ElementId, NetValue};

pub use frame::FrameReport;
pub use markers::{MarkerHitShape, MarkerView};

pub trait Ground: Send + Sync {
    fn height(&self, x: f32, y: f32, z: f32) -> Option<f32>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalState {
    pub velocity: [f32; 3],
    pub speed: f32,
    pub air: bool,
    pub grind: bool,
    pub trick: Option<String>,
    pub score: f64,
    pub health: f32,
    pub finished: bool,
}

pub const SESSION_KEYS: u64 = 1 << 48;

#[derive(Clone, Debug, PartialEq)]
pub struct RiderView {
    pub key: u64,
    pub name: String,
    pub position: [f32; 3],
    pub local: Option<LocalState>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceView {
    pub mode: String,
    pub drop: String,
    pub state: &'static str,
    pub clock_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StandingView {
    pub rider: Option<u64>,
    pub name: String,
    pub place: u32,
    pub value: f64,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RideEvent {
    Land { air_time: f32, bail: bool },
    Trick { name: String, points: f64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: &'static str,
    pub down: bool,
}

/// One rider's career progress as the profile holds it: `xp` is the rider's earnings (the save's `MONEYEARNED`),
/// which the game turns into the rider's level.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRiderView {
    pub key: u32,
    pub name: String,
    pub display_name: String,
    pub xp: i64,
    /// The level as the game shows it (1 and up).
    pub level: i64,
}

/// The single-player profile the front end holds; `None` in a network session or before the front end is up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileView {
    pub credits: i64,
    pub infinite_credits: bool,
    /// The gear that never wears out or runs down on a drop, one bit each (`GEAR_ALL` for every piece).
    pub unlimited_gear: i64,
    /// The lowest rarity the shops offer (0 = every rarity).
    pub shop_min_rarity: i64,
    /// The highest level a rider can reach (as the game shows it).
    pub max_level: i64,
    pub riders: Vec<ProfileRiderView>,
}

/// A change a mod asked for to the single-player profile; the game applies it and saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileOrder {
    Credits {
        package: String,
        credits: i64,
    },
    RiderXp {
        package: String,
        rider: u32,
        xp: i64,
    },
    /// `level` as the game shows it (1 and up).
    RiderLevel {
        package: String,
        rider: u32,
        level: i64,
    },
    InfiniteCredits {
        package: String,
        on: bool,
    },
    ShopMinRarity {
        package: String,
        rarity: i64,
    },
    UnlimitedGear {
        package: String,
        mask: i64,
    },
    /// The highest level a rider can reach, as the game shows it (the game's own when at most its count).
    MaxLevel {
        package: String,
        level: i64,
    },
}

/// Every piece of gear `setUnlimitedGear` knows: 1 wingsuit (wear and flight time), 2 ice axe, 4 armour, 8 head lamp,
/// 16 pulse goggles, 32 oxygen tank.
pub const GEAR_ALL: i64 = 63;

/// The most credits or XP a script may set (the game keeps both within a signed 32-bit value).
pub const PROFILE_MAX: i64 = i32::MAX as i64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelView {
    pub id: u64,
    pub drop: String,
    pub budget: MarkerBudget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkerBudget {
    pub markers: usize,
    pub vertices: u64,
    pub triangles: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelMarker {
    pub node: String,
    pub id: ContentId,
    pub position: [f32; 3],
    pub heading: f32,
    pub size: f32,
    pub colour: u32,
    pub hit: MarkerHitShape,
    pub sound: Option<ContentId>,
}

pub const MAX_LEVEL_MARKERS: usize = 4096;
pub const MAX_LEVEL_MARKER_ID: usize = 128;

#[derive(Clone, Default)]
pub struct GameFrame {
    pub now: Duration,
    pub level: Option<LevelView>,
    pub level_markers: Option<Arc<Vec<LevelMarker>>>,
    pub ground: Option<Arc<dyn Ground>>,
    pub riders: Vec<RiderView>,
    pub instance: Option<InstanceView>,
    pub standings: Vec<StandingView>,
    pub events: Vec<RideEvent>,
    pub keys: Vec<KeyEvent>,
    pub typing: bool,
    pub hud_busy: bool,
    pub profile: Option<ProfileView>,
    /// A port addition for mods: the window and the part of the draw stage it shows; `None` before the game knows.
    pub screen: Option<ScreenView>,
    /// A port addition for mods (`getCursor`): the mouse cursor in stage units; `None` outside the window.
    pub cursor: Option<(f32, f32)>,
    /// A port addition for mods (`isMultiplayer`): a network session is up, or the server browser is open.
    pub multiplayer: bool,
    /// A port addition for mods (`isMainMenu`): the port's hub or the game's main menu is the screen up.
    pub main_menu: bool,
    /// A port addition for mods (`getMainMenu`): on the port's hub, its menu panel `[x, y, width, height]` in stage
    /// units.
    pub main_menu_box: Option<[f32; 4]>,
}

/// A port addition for mods (`getScreen`): the window's size in pixels and what it covers in the units of the 1280 x
/// 720 stage the scripts draw on: past the stage on an ultrawide window (left below 0, right past 1280), a part of it
/// on a 4:3 or 16:10 one (its sides cropped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenView {
    pub width: u32,
    pub height: u32,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Default for ScreenView {
    fn default() -> Self {
        ScreenView { width: 1280, height: 720, left: 0.0, top: 0.0, right: 1280.0, bottom: 720.0 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentMeta {
    pub source: String,
    pub layer: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkerMeta {
    pub size: f32,
    pub colour: u32,
    pub hit: MarkerHitShape,
    pub sound: Option<ContentId>,
    pub vertices: u64,
    pub triangles: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub ids: BTreeMap<ContentId, ContentMeta>,
    pub markers: BTreeMap<ContentId, MarkerMeta>,
}

impl Catalog {
    pub fn kind(&self, kind: Kind) -> impl Iterator<Item = &ContentId> {
        self.ids.keys().filter(move |id| id.kind() == kind)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Draw {
    Text { text: String, x: f32, y: f32, colour: u32, size: f32, align: Align, shadow: bool },
    Rect { x: f32, y: f32, w: f32, h: f32, colour: u32 },
    Image { x: f32, y: f32, w: f32, h: f32, texture: ContentId, colour: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SoundOrder {
    pub package: String,
    pub id: ContentId,
    pub position: Option<[f32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CameraOrder {
    Rider(u64),
    Own,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FadeOrder {
    pub fade_in: bool,
    pub seconds: f32,
    pub colour: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HudOrder {
    pub package: String,
    pub title: String,
    pub text: String,
    pub seconds: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Orders {
    pub draws: Option<Vec<(String, Draw)>>,
    pub sounds: Vec<SoundOrder>,
    pub music: Option<(String, ContentId)>,
    pub camera: Option<CameraOrder>,
    pub fade: Option<FadeOrder>,
    pub hud: Vec<HudOrder>,
    pub preload: Vec<ContentId>,
    pub profile: Vec<ProfileOrder>,
    /// A running package holds the menu keys (arrows, Enter, Backspace): the game does not see them this frame.
    pub capture: bool,
    /// A port addition for mods (`musicPlayer`): play, pause, resume or stop the music, in a ride or the menus.
    pub player: Vec<PlayerOrder>,
}

/// A port addition for mods: what `musicPlayer` asks of the game's music.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerOrder {
    /// Play this song now: in a ride the level's music gives way to it (as `playMusic`), in the menus it takes over
    /// from the menus' music (as RiderNet's music menu plays a song).
    Play { package: String, id: ContentId },
    /// Hold the music where it is (true) or carry on (false).
    Pause(bool),
    /// Stop the music that plays.
    Stop,
    /// The licensed music's level, 0 to 1 (`musicVolume`).
    Volume(f32),
    /// Move the playing song (a package's) to this many seconds in (`musicPlayer("seek", seconds)`).
    Seek(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameCaps {
    pub draw_calls: usize,
    pub draw_calls_package: usize,
    pub draw_glyphs: usize,
    pub draw_pixels: u64,
    pub text_bytes: usize,
    pub markers_package: usize,
    pub binds_package: usize,
    pub sounds_frame: usize,
    pub sounds_frame_package: usize,
    pub hud_every: Duration,
    pub music_every: Duration,
    pub preload_frame: usize,
    pub server_markers: usize,
    pub frame_bound: Duration,
}

// TODO(guess): the draw caps (1024 calls a frame, 512 per package, 2048 glyphs, 16 stages of pixels), 256 markers and
// 64 key binds per package, 32 sounds a frame (8 per package), one HUD message and one `playMusic` a second per package
// and the frame bound of 20 ms are port choices, not the game's (it has no mod scripts). The draw caps keep what the
// UI's render list spends on the scripts' layer near 2 ms a frame at worst: measured with eight packages drawing
// 4 088 rects moving every frame (about 0.8 µs of the UI's CPU each) and 4 088 four-glyph texts changing every frame
// (about 0.7 µs a glyph), development machine; a full scoreboard is a few hundred calls. Settle: measure real HUD and
// gamemode packages (M8) on the target machines.
pub const GAME_CAPS: GameCaps = GameCaps {
    draw_calls: 1024,
    draw_calls_package: 512,
    draw_glyphs: 2048,
    draw_pixels: 1280 * 720 * 16,
    text_bytes: 256,
    markers_package: 256,
    binds_package: 64,
    sounds_frame: 32,
    sounds_frame_package: 8,
    hud_every: Duration::from_secs(1),
    music_every: Duration::from_secs(1),
    preload_frame: 64,
    frame_bound: Duration::from_millis(20),
    server_markers: 1024,
};

#[derive(Clone)]
pub(crate) struct Bind {
    pub package: String,
    pub key: &'static str,
    pub state: &'static str,
    pub callback: super::events::Callback,
}

#[derive(Default)]
pub struct GameState {
    pub(crate) frame: GameFrame,
    pub(crate) riders: BTreeMap<u64, ElementId>,
    pub(crate) markers: markers::Markers,
    pub(crate) binds: Vec<Bind>,
    pub(crate) catalog: Arc<Catalog>,
    pub(crate) orders: Orders,
    pub(crate) drawing: Option<DrawList>,
    pub(crate) hud_at: BTreeMap<String, Duration>,
    pub(crate) music_at: BTreeMap<String, Duration>,
    pub(crate) skipped: u64,
    pub(crate) bound: Option<Duration>,
    pub(crate) last_position: Option<[f32; 3]>,
    pub(crate) made: BTreeMap<u64, ElementId>,
    pub(crate) session: BTreeMap<u16, (ElementId, ElementId)>,
    pub(crate) capture: std::collections::BTreeSet<String>,
}

#[derive(Default)]
pub(crate) struct DrawList {
    pub items: Vec<(String, Draw)>,
    pub per_package: BTreeMap<String, usize>,
    /// A port addition for mods (`drawOnTop`): the packages whose draws this frame go after the others'.
    pub on_top: std::collections::BTreeSet<String>,
    pub glyphs: usize,
    pub pixels: f64,
}

impl GameState {
    pub(crate) fn forget_session(&mut self, gone: &[ElementId]) {
        self.session.retain(|_, (r, p)| !gone.contains(r) && !gone.contains(p));
        self.riders.retain(|_, e| !gone.contains(e));
        self.made.retain(|_, e| !gone.contains(e));
    }

    pub(crate) fn rider_element(&self, key: u64) -> Option<ElementId> {
        self.riders.get(&key).copied()
    }

    pub(crate) fn rider_key(&self, element: ElementId) -> Option<u64> {
        self.riders.iter().find(|(_, e)| **e == element).map(|(k, _)| *k)
    }

    pub(crate) fn rider_view(&self, key: u64) -> Option<&RiderView> {
        self.frame.riders.iter().find(|r| r.key == key)
    }
}

pub(crate) fn vec3_value(p: [f32; 3]) -> NetValue {
    let n = |s: &str, v: f32| (NetValue::String(s.as_bytes().to_vec()), NetValue::Number(f64::from(v)));
    NetValue::Table(vec![n("x", p[0]), n("y", p[1]), n("z", p[2])])
}

pub(crate) fn record(fields: Vec<(&str, Option<NetValue>)>) -> NetValue {
    NetValue::Table(
        fields.into_iter().filter_map(|(k, v)| v.map(|v| (NetValue::String(k.as_bytes().to_vec()), v))).collect(),
    )
}

pub(crate) fn text(s: &str) -> NetValue {
    NetValue::String(s.as_bytes().to_vec())
}
