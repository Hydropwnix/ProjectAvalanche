const MOVIE_FPS: f32 = 60.0;
const INTRO: (i32, i32) = (4, 25);
const OUTRO: (i32, i32) = (26, 35);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FinishMessage {
    shown: bool,
    frame: i32,
    stop: i32,
    clock: f32,
    last: i32,
    playing: bool,
    stopped: bool,
    intro: bool,
    hold: f32,
}

impl FinishMessage {
    fn play(&mut self, (label, stop): (i32, i32), intro: bool) {
        (self.frame, self.stop, self.clock, self.last) = (label, stop, 0.0, label);
        (self.playing, self.stopped, self.intro) = (true, false, intro);
    }

    pub fn update(&mut self, dt: f32) -> bool {
        let cleared = self.provider(dt);
        if self.shown {
            self.clock += dt * MOVIE_FPS;
            let whole = self.clock.floor();
            self.clock -= whole;
            self.frame = (self.frame + whole as i32).min(self.stop);
        }
        cleared
    }

    fn provider(&mut self, dt: f32) -> bool {
        if !self.shown {
            self.shown = true;
            self.play(INTRO, true);
            return false;
        }
        if self.playing {
            self.stopped = true;
            self.playing = self.frame != self.last;
            self.last = self.frame;
        } else {
            self.stopped = false;
        }
        if self.playing {
            return false;
        }
        if self.stopped {
            if self.intro {
                self.hold = 0.0;
                return false;
            }
            return true;
        }
        self.hold += dt;
        if self.hold >= super::FINISH_MESSAGE_HOLD_S {
            self.play(OUTRO, false);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_clears_after_its_intro_hold_and_outro() {
        let mut m = FinishMessage::default();
        let n = (1..200).find(|_| m.update(1.0 / 30.0)).expect("cleared");
        assert_eq!(n, 49, "cleared after {n} updates");
    }
}
