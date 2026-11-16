use crate::musicbox;
use crate::sfx_bus::Graph;

pub trait Faders {
    fn fade_by_name(&mut self, bus: &str, to: f32, secs: f32);
    fn fading(&self, bus: &str) -> bool;
}

impl Faders for Graph {
    fn fade_by_name(&mut self, bus: &str, to: f32, secs: f32) {
        Graph::fade_by_name(self, bus, to, secs);
    }
    fn fading(&self, bus: &str) -> bool {
        Graph::fading(self, bus)
    }
}

pub const SUBMIX: &str = "MusicSubmix";
pub const MASTER: &str = "Music_Master_BUS";
pub const FADE: f32 = 0.5;
pub const MASTER_OUT: f32 = 0.5;
pub const MASTER_IN: f32 = 0.1;
pub const SUBMIX_PLAYING: f32 = 1.0;

#[derive(Clone, Debug, Default)]
pub struct MusicPause {
    pub pausing: bool,
    pub paused: bool,
    pub cinematic: bool,
    pub tutorial: bool,
    restart: bool,
    level_set: bool,
    stops: u32,
    pub log: Vec<String>,
}

impl MusicPause {
    fn pause(&mut self, g: &mut impl Faders, secs: f32) {
        self.sync();
        let mut fading = true;
        if !self.pausing {
            if musicbox::playing() && !g.fading(SUBMIX) {
                g.fade_by_name(SUBMIX, 0.0, secs);
                self.log.push(format!("pause: {SUBMIX} out over {secs} s"));
            } else {
                fading = false;
            }
        }
        if secs > 0.0 && fading {
            (self.pausing, self.paused) = (true, true);
            return;
        }
        if !musicbox::paused() {
            musicbox::set_paused(true);
            self.log.push("PauseMusic".into());
        }
        g.fade_by_name(MASTER, 0.0, MASTER_OUT);
        self.log.push(format!("pause: {MASTER} out over {MASTER_OUT} s"));
        (self.paused, self.pausing) = (true, false);
    }

    fn resume(&mut self, g: &mut impl Faders, secs: f32) {
        self.sync();
        g.fade_by_name(MASTER, 1.0, MASTER_IN);
        if musicbox::paused() {
            musicbox::set_paused(false);
            self.log.push("UnpauseMusic".into());
        }
        if musicbox::playing() && !musicbox::stopping() {
            g.fade_by_name(SUBMIX, if self.level_set { SUBMIX_PLAYING } else { 0.0 }, secs);
        }
        self.log.push(format!("resume: {MASTER} in over {MASTER_IN} s"));
        (self.pausing, self.paused) = (false, false);
    }

    pub fn pause_event(&mut self, g: &mut impl Faders, tutorial: bool) {
        if !self.cinematic && !tutorial {
            self.pause(g, FADE);
        }
        self.tutorial = tutorial;
    }

    pub fn unpause_event(&mut self, g: &mut impl Faders) {
        if self.paused && !self.cinematic {
            self.resume(g, FADE);
        }
        self.tutorial = false;
    }

    pub fn pause_menu(&mut self, g: &mut impl Faders, opened: bool, closed: bool) {
        if !self.tutorial {
            return;
        }
        if opened {
            self.pause(g, FADE);
        } else if closed && self.paused {
            self.resume(g, FADE);
        }
    }

    pub fn state_msg(&mut self, g: &mut impl Faders, state: i32, restart: bool) {
        if state == super::frontend::state::TRANSITION && !self.cinematic {
            self.cinematic = true;
            if !self.paused {
                self.pause(g, FADE);
            }
        } else if self.cinematic && state != super::frontend::state::TRANSITION {
            self.cinematic = false;
            self.resume(g, FADE);
        }
        if !self.restart && restart {
            g.fade_by_name(MASTER, 1.0, FADE);
        }
        self.restart = restart;
    }

    pub fn update(&mut self, g: &mut impl Faders) {
        self.sync();
        let playing = musicbox::playing();
        let stopping = musicbox::stopping();
        if !self.pausing {
            if playing && !stopping && !g.fading(SUBMIX) && !self.level_set {
                g.fade_by_name(SUBMIX, SUBMIX_PLAYING, 0.0);
                self.level_set = true;
            }
            return;
        }
        if !playing {
            if self.level_set {
                self.log.push("StopMusic (no song: the pause's fade outlived it)".into());
            }
            self.level_set = false;
        } else if !stopping && !g.fading(SUBMIX) {
            self.pause(g, 0.0);
        }
    }

    fn sync(&mut self) {
        let stops = musicbox::stops();
        if stops != self.stops {
            self.stops = stops;
            self.level_set = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Buses {
        level: HashMap<String, f32>,
        fading: HashMap<String, bool>,
    }

    impl Buses {
        fn settle(&mut self) {
            self.fading.clear();
        }
        fn level(&self, bus: &str) -> f32 {
            self.level.get(bus).copied().unwrap_or(1.0)
        }
    }

    impl Faders for Buses {
        fn fade_by_name(&mut self, bus: &str, to: f32, secs: f32) {
            self.level.insert(bus.to_string(), to);
            self.fading.insert(bus.to_string(), secs > 0.0);
        }
        fn fading(&self, bus: &str) -> bool {
            self.fading.get(bus).copied().unwrap_or(false)
        }
    }

    #[test]
    fn a_song_stopped_under_the_pause_leaves_the_next_its_level() {
        let mut g = Buses::default();
        let mut p = MusicPause::default();
        musicbox::set_paused(false);
        musicbox::test_state::set_playing(true);
        p.update(&mut g);
        assert_eq!(g.level(SUBMIX), SUBMIX_PLAYING);
        p.pause_event(&mut g, false);
        assert_eq!((g.level(SUBMIX), p.pausing), (0.0, true));
        g.settle();
        p.update(&mut g);
        assert!(musicbox::paused() && p.paused && !p.pausing);
        g.settle();
        musicbox::test_state::stop_music();
        p.update(&mut g);
        p.unpause_event(&mut g);
        assert!(!musicbox::paused());
        assert_eq!(g.level(SUBMIX), 0.0);
        g.settle();
        musicbox::test_state::set_playing(true);
        p.update(&mut g);
        assert_eq!(g.level(SUBMIX), SUBMIX_PLAYING, "the next song stays silent");
        assert!(!g.fading(SUBMIX));
        p.pause_event(&mut g, false);
        g.settle();
        p.update(&mut g);
        p.unpause_event(&mut g);
        assert_eq!(g.level(SUBMIX), SUBMIX_PLAYING);
        musicbox::test_state::set_playing(false);
        musicbox::set_paused(false);
    }
}
