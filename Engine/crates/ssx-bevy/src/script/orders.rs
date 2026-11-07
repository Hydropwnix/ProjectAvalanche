use std::sync::Mutex;
use std::sync::mpsc::Receiver;

use bevy::prelude::*;
use ssx_content::ContentId;
use ssx_script::host::game::{CameraOrder, Orders};

use super::snapshot::{BOT_KEYS, GHOST_KEYS};
use crate::camera::chase::SpectateTarget;
use crate::ride::Ride;
use crate::shell::game_mode::GameUiRes;

#[derive(Resource, Default)]
pub(crate) struct OrderState {
    camera: Option<u64>,
    camera_refused: bool,
    fade_alpha: f32,
    fade_to: f32,
    fade_rate: f32,
    fade_colour: u32,
    pub(super) hud: HudSlot,
    pub(super) music: MusicLoads<ssx_assets::musicbox::LicensedMusic>,
    music_place: Option<MusicPlace>,
}

#[derive(Default)]
pub(super) struct HudSlot {
    shown: Option<(String, String, f32)>,
    pub(super) refused: u64,
    logged: bool,
}

impl HudSlot {
    pub(super) fn ours(&self, v: &ssx_ui::game::values::UiValues) -> bool {
        use ssx_ui::game::values::id;
        self.shown.as_ref().is_some_and(|(t, m, _)| {
            v.bool(id::HUDMESSAGE_IS_ENABLED) && v.str(id::HUDMESSAGE_TITLE) == t && v.str(id::HUDMESSAGE_MESSAGE) == m
        })
    }

    pub(super) fn game_busy(&self, v: &ssx_ui::game::values::UiValues) -> bool {
        v.bool(ssx_ui::game::values::id::HUDMESSAGE_IS_ENABLED) && !self.ours(v)
    }

    pub(super) fn show(
        &mut self,
        v: &mut ssx_ui::game::values::UiValues,
        title: String,
        text: String,
        seconds: f32,
    ) -> bool {
        use ssx_ui::game::values::id;
        if self.game_busy(v) {
            self.refused += 1;
            if !self.logged {
                self.logged = true;
                info!("scripts: a HUD message waits for no one: the game's own message is up (dropped, counted)");
            }
            return false;
        }
        v.set_str(id::HUDMESSAGE_TITLE, title.clone());
        v.set_str(id::HUDMESSAGE_MESSAGE, text.clone());
        v.set_bool(id::HUDMESSAGE_IS_ENABLED, true);
        self.shown = Some((title, text, seconds));
        true
    }

    pub(super) fn tick(&mut self, v: &mut ssx_ui::game::values::UiValues, dt: f32) {
        if self.shown.is_none() {
            return;
        }
        if !self.ours(v) {
            self.shown = None;
            return;
        }
        if let Some((_, _, left)) = self.shown.as_mut() {
            *left -= dt;
            if *left <= 0.0 {
                self.clear(v);
            }
        }
    }

    pub(super) fn clear(&mut self, v: &mut ssx_ui::game::values::UiValues) {
        if self.ours(v) {
            v.set_bool(ssx_ui::game::values::id::HUDMESSAGE_IS_ENABLED, false);
        }
        self.shown = None;
    }
}

type SongLoad<T> = Mutex<Receiver<Result<T, String>>>;

pub(super) struct MusicLoads<T> {
    wanted: Option<(String, ContentId)>,
    loading: Option<(ContentId, SongLoad<T>)>,
    pub(super) folded: u64,
}

impl<T> Default for MusicLoads<T> {
    fn default() -> Self {
        MusicLoads { wanted: None, loading: None, folded: 0 }
    }
}

impl<T> MusicLoads<T> {
    pub(super) fn order(
        &mut self,
        package: String,
        id: ContentId,
        start: impl FnOnce(&ContentId) -> Result<Receiver<Result<T, String>>, String>,
    ) {
        if self.loading.is_some() {
            if self.wanted.replace((package, id)).is_some() {
                self.folded += 1;
            }
            return;
        }
        self.start(package, id, start);
    }

    fn start(
        &mut self,
        package: String,
        id: ContentId,
        start: impl FnOnce(&ContentId) -> Result<Receiver<Result<T, String>>, String>,
    ) {
        match start(&id) {
            Ok(rx) => {
                info!("scripts: [{package}] playMusic {id}");
                self.loading = Some((id, Mutex::new(rx)));
            }
            Err(e) => warn!("scripts: [{package}] playMusic {id}: {e}"),
        }
    }

    pub(super) fn poll(
        &mut self,
        start: impl FnOnce(&ContentId) -> Result<Receiver<Result<T, String>>, String>,
    ) -> Option<(ContentId, Result<T, String>)> {
        let (id, rx) = self.loading.as_ref()?;
        let got = match rx.lock().ok().map(|r| r.try_recv()) {
            Some(Ok(g)) => g,
            Some(Err(std::sync::mpsc::TryRecvError::Empty)) => return None,
            _ => Err("the loader ended without a song".to_string()),
        };
        let id = id.clone();
        self.loading = None;
        match self.wanted.take() {
            Some((package, next)) => {
                self.start(package, next, start);
                None
            }
            None => Some((id, got)),
        }
    }

    pub(super) fn clear(&mut self) {
        (self.wanted, self.loading) = (None, None);
    }

    #[cfg(test)]
    pub(super) fn in_flight(&self) -> bool {
        self.loading.is_some()
    }
}

impl OrderState {
    pub(super) fn clear(&mut self, values: &mut ssx_ui::game::values::UiValues) {
        (self.camera, self.camera_refused) = (None, false);
        (self.fade_alpha, self.fade_to, self.fade_rate) = (0.0, 0.0, 0.0);
        self.hud.clear(values);
        self.music.clear();
    }

    pub(super) fn fade_colour(&self) -> Option<[f32; 4]> {
        let [r, g, b, a] = super::draw::colour(self.fade_colour);
        (self.fade_alpha > 0.0).then_some([r, g, b, a * self.fade_alpha])
    }

    #[cfg(test)]
    pub(super) fn idle(&self) -> bool {
        self.camera.is_none() && self.fade_alpha == 0.0 && self.fade_to == 0.0 && !self.music.in_flight()
    }
}

pub(super) struct MusicPlace {
    pub big_root: std::path::PathBuf,
    pub scope: Option<ssx_assets::content::audio::AudioScope>,
}

impl OrderState {
    pub(super) fn take(
        &mut self,
        o: Orders,
        values: &mut ssx_ui::game::values::UiValues,
        glyphs: Option<&std::collections::BTreeSet<char>>,
        place: MusicPlace,
    ) {
        let Orders { camera, fade, hud, music, .. } = o;
        self.music_place = Some(place);
        match camera {
            Some(CameraOrder::Own) => self.camera = None,
            Some(CameraOrder::Rider(k)) => {
                self.camera = Some(k);
                self.camera_refused = false;
            }
            None => {}
        }
        if let Some(f) = fade {
            self.fade_to = if f.fade_in { 0.0 } else { 1.0 };
            self.fade_rate = if f.seconds > 0.0 { 1.0 / f.seconds } else { f32::INFINITY };
            if !f.fade_in {
                self.fade_colour = f.colour;
            }
        }
        if let Some(h) = hud.into_iter().last() {
            let shown = |t: &str| ssx_ui::game::fe::ava::shown(t, glyphs);
            if self.hud.show(values, shown(&h.title), shown(&h.text), h.seconds) {
                info!("scripts: [{}] a HUD message for {:.1} s", h.package, h.seconds);
            }
        }
        if let Some((package, id)) = music {
            let place = self.music_place.as_ref();
            self.music.order(package, id, |id| spawn_load(place, id));
        }
    }
}

fn spawn_load(
    place: Option<&MusicPlace>,
    id: &ContentId,
) -> Result<Receiver<Result<ssx_assets::musicbox::LicensedMusic, String>>, String> {
    let place = place.ok_or("no level music")?;
    let (big, scope, id) = (place.big_root.clone(), place.scope.clone(), id.clone());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("script-music".into())
        .spawn(move || {
            let _ = tx.send(load_song(&big, &id, scope.as_ref()));
        })
        .map_err(|e| format!("no loader thread ({e})"))?;
    Ok(rx)
}

fn load_song(
    big_root: &std::path::Path,
    id: &ContentId,
    scope: Option<&ssx_assets::content::audio::AudioScope>,
) -> Result<ssx_assets::musicbox::LicensedMusic, String> {
    let mut mb = ssx_assets::musicbox::MusicBox::load(big_root).map_err(|e| format!("{e:#}"))?;
    let content = ssx_assets::content::GameContent::of(big_root);
    let song = match id.path().strip_prefix("licensed/").filter(|_| id.namespace() == ssx_content::BASE_NAMESPACE) {
        Some(n) => n.parse::<u32>().map_err(|_| format!("{id}: not a licensed song"))?,
        None => {
            let scope = scope.ok_or("no level music to take a package's song into")?;
            let plan = content.music_plan(scope);
            mb.add_packages(&plan.music);
            let k = plan.music.added.iter().position(|t| t.from.id == *id);
            let k = k.ok_or_else(|| format!("{id}: the level's music plan does not take it"))?;
            let k = u32::try_from(k).map_err(|_| format!("{id}: beyond the MusicBox's ids"))?;
            ssx_assets::musicbox::FIRST_PACKAGE_SONG.checked_add(k).ok_or_else(|| format!("{id}: beyond the ids"))?
        }
    };
    if mb.song(song).is_none() {
        return Err(format!("{id}: the MusicBox has no song {song}"));
    }
    Ok(ssx_assets::musicbox::LicensedMusic::song(mb, song))
}

pub(super) fn music(mut state: ResMut<OrderState>, music: Option<Res<crate::audio::music::Music>>) {
    let s = &mut *state;
    let place = s.music_place.as_ref();
    let Some((id, got)) = s.music.poll(|id| spawn_load(place, id)) else { return };
    match (got, music) {
        (Ok(song), Some(m)) => match m.play_script_song(song) {
            Ok(()) => info!("scripts: {id} plays"),
            Err(e) => warn!("scripts: playMusic {id}: {e}"),
        },
        (Ok(_), None) => warn!("scripts: playMusic {id}: the level has no music up"),
        (Err(e), _) => warn!("scripts: playMusic {id}: {e}"),
    }
}

pub(super) fn fade(mut state: ResMut<OrderState>, time: Res<Time<Real>>) {
    let s = &mut *state;
    let step = s.fade_rate * time.delta_secs();
    s.fade_alpha = if s.fade_alpha < s.fade_to {
        (s.fade_alpha + step).min(s.fade_to)
    } else {
        (s.fade_alpha - step).max(s.fade_to)
    };
}

pub(super) fn hud_timeout(mut state: ResMut<OrderState>, time: Res<Time<Real>>, mut ui: ResMut<GameUiRes>) {
    state.hud.tick(&mut ui.values, time.delta_secs());
}

const SCRIPT_SPECTATE: u16 = u16::MAX - 128;

pub(super) fn camera_target(
    mut state: ResMut<OrderState>,
    ride: Res<Ride>,
    level: Option<Res<crate::shell::game_level::LevelGame>>,
    target: Option<Res<SpectateTarget>>,
    mut commands: Commands,
) {
    let ours = target.as_ref().is_some_and(|t| t.rider.is_some_and(|r| r >= SCRIPT_SPECTATE));
    let Some(key) = state.camera else {
        if ours {
            commands.insert_resource(SpectateTarget::default());
        }
        return;
    };
    let slot = |k: usize, base: u16| u16::MAX - 1 - base - u16::try_from(k.min(63)).unwrap_or(63);
    let followed = if key >= GHOST_KEYS {
        let seconds = ride.session.event.clock.seconds;
        level
            .as_ref()
            .and_then(|l| l.script_ghost(key & 0xFFFF_FFFF, seconds))
            .map(|(k, p, v, air)| (slot(k, 64), Vec3::from_array(p), Vec3::from_array(v), air))
    } else if let Some(k) = key.checked_sub(BOT_KEYS).and_then(|k| usize::try_from(k).ok()) {
        let bot = ride.session.bots.as_ref().and_then(|r| r.bots.get(k)).filter(|b| !b.removed);
        bot.map(|b| (slot(k, 0), b.position, b.velocity, b.in_air))
    } else {
        None
    };
    let Some((rider, position, velocity, air)) = followed else {
        if !state.camera_refused {
            warn!(
                "scripts: setCameraTarget: rider key {key} is no race bot or rival ghost the camera can follow now: \
                 the player's own"
            );
            state.camera_refused = true;
        }
        if ours {
            commands.insert_resource(SpectateTarget::default());
        }
        return;
    };
    let inputs = crate::camera::chase::spectate_inputs_at(position, velocity, air, ride.back_press);
    commands.insert_resource(SpectateTarget { rider: Some(rider), inputs: Some(inputs) });
}
