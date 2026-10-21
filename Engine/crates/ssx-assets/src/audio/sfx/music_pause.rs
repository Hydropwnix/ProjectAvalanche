use crate::musicbox;
use crate::sfx_bus::Graph;

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
    pub log: Vec<String>,
}

impl MusicPause {
    fn pause(&mut self, g: &mut Graph, secs: f32) {
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

    fn resume(&mut self, g: &mut Graph, secs: f32) {
        g.fade_by_name(MASTER, 1.0, MASTER_IN);
        if musicbox::paused() {
            musicbox::set_paused(false);
            self.log.push("UnpauseMusic".into());
        }
        if musicbox::playing() {
            g.fade_by_name(SUBMIX, SUBMIX_PLAYING, secs);
        }
        self.log.push(format!("resume: {MASTER} in over {MASTER_IN} s"));
        (self.pausing, self.paused) = (false, false);
    }

    pub fn pause_event(&mut self, g: &mut Graph, tutorial: bool) {
        if !self.cinematic && !tutorial {
            self.pause(g, FADE);
        }
        self.tutorial = tutorial;
    }

    pub fn unpause_event(&mut self, g: &mut Graph) {
        if self.paused && !self.cinematic {
            self.resume(g, FADE);
        }
        self.tutorial = false;
    }

    pub fn pause_menu(&mut self, g: &mut Graph, opened: bool, closed: bool) {
        if !self.tutorial {
            return;
        }
        if opened {
            self.pause(g, FADE);
        } else if closed && self.paused {
            self.resume(g, FADE);
        }
    }

    pub fn state_msg(&mut self, g: &mut Graph, state: i32, restart: bool) {
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

    pub fn update(&mut self, g: &mut Graph) {
        let playing = musicbox::playing();
        if !self.pausing {
            if playing && !g.fading(SUBMIX) && !self.level_set {
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
        } else if !g.fading(SUBMIX) {
            self.pause(g, 0.0);
        }
    }
}
