use super::FeEnv;

pub const LEVEL: usize = 0;
pub const PAUSE_LEAVE: usize = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BusyIcon {
    pub counts: [u32; 2],
    pub saving: bool,
    pub music: u8,
}

impl BusyIcon {
    pub fn add(&mut self, env: &mut FeEnv, kind: usize) {
        self.counts[kind] += 1;
        self.apply(env);
    }

    pub fn release(&mut self, env: &mut FeEnv, kind: usize) {
        self.counts[kind] = self.counts[kind].saturating_sub(1);
        self.apply(env);
    }

    pub fn set_saving(&mut self, env: &mut FeEnv, on: bool) {
        self.saving = on;
        self.apply(env);
    }

    pub fn set_music(&mut self, env: &mut FeEnv, on: Option<bool>) {
        self.music = match on {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        self.apply(env);
    }

    pub fn choice(&self) -> (i32, Option<&'static str>) {
        if self.saving {
            (2, Some("TXT_SAVING"))
        } else if self.music == 1 {
            (1, None)
        } else if self.music != 0 || self.counts.iter().sum::<u32>() == 0 {
            (0, None)
        } else {
            (1, Some("TXT_LOADING"))
        }
    }

    fn apply(&self, env: &mut FeEnv) {
        let (form, text) = self.choice();
        env.values.set_int(0x9A, form);
        let text = text.map_or_else(String::new, |k| env.loc.text(k));
        env.values.set_str(0x9B, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_icon_follows_the_reasons_in_order() {
        let mut b = BusyIcon::default();
        assert_eq!(b.choice(), (0, None));
        b.counts[LEVEL] = 1;
        assert_eq!(b.choice(), (1, Some("TXT_LOADING")));
        b.music = 2;
        assert_eq!(b.choice(), (0, None), "the music menu's off hides it");
        b.music = 1;
        assert_eq!(b.choice(), (1, None));
        b.saving = true;
        assert_eq!(b.choice(), (2, Some("TXT_SAVING")));
        b.saving = false;
        b.music = 0;
        b.counts = [0, 1];
        assert_eq!(b.choice(), (1, Some("TXT_LOADING")));
    }
}
