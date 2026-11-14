const FRAME_TIME_S: f32 = 1.0 / 30.0;
const CATCH_UP: u32 = 2;
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
        (self.frame, self.stop, self.last) = (label, stop, label);
        (self.playing, self.stopped, self.intro) = (true, false, intro);
    }

    pub fn update(&mut self, dt: f32) -> bool {
        let cleared = self.provider(dt);
        // TODO(guess): the root movie's time left over before the message's post (below one frame time) starts at 0;
        // the score manager's mirror of the widget cannot read the UI's `MovieImpl` +0x168 (it moves no frame while
        // the FE's dt is the frame time, the 30 Hz loop; a dilated dt could move one frame by an update).
        self.clock += dt.max(0.0);
        let mut frames = 0;
        while self.clock >= FRAME_TIME_S && frames <= CATCH_UP {
            self.clock -= FRAME_TIME_S;
            frames += 1;
        }
        if self.shown {
            self.frame = (self.frame + frames as i32).min(self.stop);
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
        assert_eq!(n, 63, "cleared after {n} updates");
    }

    #[test]
    fn the_root_movie_runs_at_most_three_frames_a_call() {
        let mut m = FinishMessage::default();
        m.update(0.5);
        assert_eq!(m.frame, 7);
        m.update(0.0);
        assert_eq!(m.frame, 10);
    }
}
