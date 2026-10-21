#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpeechPause {
    pub paused: bool,
    pub tutorial: bool,
}

impl SpeechPause {
    pub fn pause_event(&mut self, tutorial: bool) -> Option<bool> {
        self.tutorial = tutorial;
        if tutorial {
            return None;
        }
        self.paused = true;
        Some(true)
    }

    pub fn unpause_event(&mut self) -> Option<bool> {
        self.tutorial = false;
        std::mem::take(&mut self.paused).then_some(false)
    }

    pub fn pause_menu(&mut self, opened: bool, closed: bool) -> Option<bool> {
        if !self.tutorial {
            return None;
        }
        if opened {
            self.paused = true;
            return Some(true);
        }
        (closed && std::mem::take(&mut self.paused)).then_some(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pause_menu_and_a_tutorial_popup() {
        let mut p = SpeechPause::default();
        assert_eq!(p.pause_menu(true, false), None);
        assert_eq!(p.pause_event(false), Some(true));
        assert_eq!(p.pause_menu(false, true), None);
        assert_eq!(p.unpause_event(), Some(false));
        assert_eq!(p.unpause_event(), None);
        assert_eq!(p.pause_event(true), None);
        assert_eq!(p.pause_menu(false, false), None);
        assert_eq!(p.pause_menu(true, false), Some(true));
        assert_eq!(p.pause_menu(false, true), Some(false));
        assert_eq!(p.unpause_event(), None);
        assert_eq!(p.pause_event(true), None);
        assert_eq!(p.pause_menu(true, false), Some(true));
        assert_eq!(p.pause_menu(false, false), None);
        assert!(p.paused);
        assert_eq!(p.unpause_event(), Some(false));
    }
}
