use super::FrontEnd;

pub const LEVEL_START_IN_S: f32 = 1.0;
pub const LOOP_IN_S: f32 = 1.5;
pub const LOOP_IN_DELAY_S: f32 = 0.5;

pub const FADE_OUT: i32 = 2;
pub const FADE_IN: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fader {
    pub delay: f32,
    pub duration: f32,
    pub remaining: f32,
    pub alpha: f32,
    pub grey: f32,
    pub direction: i32,
}

impl Default for Fader {
    fn default() -> Self {
        Fader { delay: 0.0, duration: 0.0, remaining: 0.0, alpha: 0.0, grey: 1.0, direction: 2 }
    }
}

impl Fader {
    pub fn message(&mut self, kind: i32, duration: f32, delay: f32) {
        let dir = match kind {
            FADE_OUT => 1,
            FADE_IN => 0,
            _ => return,
        };
        if self.remaining <= 0.0 {
            self.remaining = duration;
        } else if dir == self.direction {
            self.remaining = self.remaining / self.duration * duration;
        } else {
            self.remaining = (1.0 - self.remaining / self.duration) * duration;
        }
        self.duration = duration;
        self.delay = delay;
        self.direction = dir;
        self.step(0.0);
    }

    pub fn step(&mut self, dt: f32) {
        if self.delay > 0.0 {
            self.delay -= dt;
        } else if self.remaining <= 0.0 {
            if self.direction == 1 {
                self.grey = 1.0;
                self.alpha = 1.0;
            } else {
                self.grey = 1.0;
                self.alpha = 0.0;
                self.direction = 2;
            }
        } else {
            self.remaining = (self.remaining - dt).max(0.0);
            let f = (self.duration - self.remaining) / self.duration;
            match self.direction {
                0 => self.alpha = 1.0 - f,
                1 => self.alpha = f,
                _ => {}
            }
            self.grey = 1.0;
        }
    }

    pub fn presentation(&self) -> [f32; 2] {
        [self.alpha, self.grey]
    }
}

impl FrontEnd {
    pub fn fade_message(&mut self, kind: i32, seconds: f32, delay: f32) {
        let what = match kind {
            FADE_OUT => "out",
            FADE_IN => "in",
            _ => "ignored",
        };
        self.note(format!("DoFadeMessage (0xBF0B3672) {{{kind}, {seconds} s, {delay} s}}: fade {what}"));
        self.fader.message(kind, seconds, delay);
    }

    pub fn screen_fade(&self) -> [f32; 2] {
        self.fader.presentation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_goes_out_and_back() {
        let mut f = Fader::default();
        assert_eq!(f.presentation(), [0.0, 1.0]);
        f.message(FADE_OUT, 0.1, 0.0);
        f.step(0.05);
        assert!((f.alpha - 0.5).abs() < 1e-5 && f.grey == 1.0);
        f.step(0.1);
        assert_eq!(f.alpha, 1.0);
        f.message(FADE_IN, 0.1, 0.0);
        f.step(0.1);
        assert_eq!(f.alpha, 0.0);
        f.step(0.0);
        assert_eq!(f.direction, 2);
    }

    #[test]
    fn a_fade_in_over_a_clear_screen_starts_white() {
        let mut f = Fader::default();
        f.message(FADE_IN, LEVEL_START_IN_S, 0.0);
        assert_eq!(f.alpha, 1.0);
        f.step(0.25);
        assert!((f.alpha - 0.75).abs() < 1e-6);
        f.step(1.0);
        assert_eq!((f.alpha, f.direction), (0.0, 0));
        f.step(1.0 / 30.0);
        assert_eq!(f.direction, 2);
        let mut g = Fader::default();
        g.message(FADE_OUT, 0.0, 0.0);
        assert_eq!(g.alpha, 1.0);
        g.message(FADE_IN, LOOP_IN_S, LOOP_IN_DELAY_S);
        g.step(0.25);
        assert_eq!(g.alpha, 1.0);
        g.step(0.25);
        assert_eq!(g.alpha, 1.0);
        g.step(0.75);
        assert!((g.alpha - 0.5).abs() < 1e-5, "{}", g.alpha);
    }

    #[test]
    fn a_reversal_goes_on_from_where_it_is() {
        let mut f = Fader::default();
        f.message(FADE_OUT, 1.0, 0.0);
        f.step(0.3);
        assert!((f.alpha - 0.3).abs() < 1e-6);
        f.message(FADE_IN, 2.0, 0.0);
        assert!((f.alpha - 0.3).abs() < 1e-5, "{}", f.alpha);
        f.step(0.3);
        assert!((f.alpha - 0.15).abs() < 1e-5, "{}", f.alpha);
    }
}
