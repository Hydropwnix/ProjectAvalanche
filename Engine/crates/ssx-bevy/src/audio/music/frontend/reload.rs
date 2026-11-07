use std::path::Path;
use std::sync::Arc;

use bevy::log::{info, warn};
use ssx_assets::content::audio::{AudioScope, MusicDecision, apply};
use ssx_assets::musicbox::MusicBox;
use ssx_ui::game::fe::music::LicensedSong;

use super::FrontEndAudio;

pub struct FrontEndReload {
    plan: Arc<MusicDecision>,
    menu_plan: Arc<MusicDecision>,
    score: Option<anyhow::Result<ssx_assets::score::ScorePlayer>>,
    tracks: Vec<(usize, anyhow::Result<ssx_assets::track_override::TrackOverride>)>,
    pub menu_songs: Option<Vec<LicensedSong>>,
}

impl FrontEndAudio {
    pub fn prepare_reload(big_root: &Path) -> FrontEndReload {
        let content = ssx_assets::content::GameContent::of(big_root);
        let plan = content.music_plan(&AudioScope::FrontEnd);
        let menu_plan = content.music_plan(&AudioScope::MusicMenu);
        let score = plan.music.scores.get("ssx_fe").map(|t| apply::score(t).map_err(anyhow::Error::msg));
        let tracks = plan
            .music
            .tracks
            .iter()
            .filter(|((p, _), _)| p == "ssx_fe")
            .map(|((_, track), t)| (*track, apply::track_override(t).map_err(anyhow::Error::msg)))
            .collect();
        let menu_songs = MusicBox::load(big_root).ok().map(|mut mb| {
            if !menu_plan.music.is_empty() {
                mb.add_packages(&menu_plan.music);
            }
            ssx_ui::game::fe::music::licensed_songs(&mb)
        });
        FrontEndReload { plan, menu_plan, score, tracks, menu_songs }
    }

    pub fn apply_reload(&mut self, r: &mut FrontEndReload) {
        let turn = crate::audio::music::GameTurn::take();
        if let Some(music) = &self.music
            && let Ok(mut fm) = music.lock()
        {
            let fm = &mut fm.music;
            fm.clear_packages();
            match r.score.take() {
                Some(Ok(s)) => fm.set_score(s, "the package's score (reloaded)"),
                Some(Err(e)) => warn!("mod-dev: the front end's score not used: {e:#}; the project's own plays"),
                None => {}
            }
            for (track, o) in r.tracks.drain(..) {
                match o {
                    Ok(o) if fm.rt.score.is_none() => fm.set_track_override(track, o),
                    Ok(_) => {}
                    Err(e) => warn!("mod-dev: ssx_fe track {track} not replaced: {e:#}; the project's own plays"),
                }
            }
        }
        self.plan = r.plan.clone();
        self.menu_plan = r.menu_plan.clone();
        let drawing = self.menu.lock().is_ok_and(|m| m.is_some());
        if drawing {
            if let Ok(mut m) = self.menu.lock()
                && let Some((lm, _)) = m.as_mut()
            {
                lm.replace_packages(&self.plan.music, None);
            }
        } else if self.plugged {
            drop(turn);
            self.arm_menu_songs();
        }
        info!("mod-dev: the front end's music re-applied ({})", self.plan.summary(&AudioScope::FrontEnd));
    }
}
