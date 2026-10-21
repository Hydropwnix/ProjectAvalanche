use super::objectives::Name;

pub const RACE_STARTED: usize = 0;
pub const CROSSED_FINISH_LINE: usize = 1;
pub const RACE_ENDED: usize = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PostedEvents {
    pub lists: [Vec<(Name, Option<Name>)>; 2],
    pub read: usize,
    pub flags: [bool; 3],
}

impl PostedEvents {
    pub fn post(&mut self, event: Name, entity: Option<Name>) {
        self.lists[1 - self.read].push((event, entity));
    }

    pub fn posted(&self, event: &Name, entity: Option<&Name>) -> bool {
        self.lists[self.read].iter().any(|(e, who)| {
            e.hash == event.hash
                && match (who, entity) {
                    (Some(a), Some(b)) => a.hash == b.hash,
                    _ => true,
                }
        })
    }

    pub fn current(&self) -> &[(Name, Option<Name>)] {
        &self.lists[self.read]
    }

    pub fn swap(&mut self) {
        self.lists[self.read].clear();
        self.read = 1 - self.read;
    }

    pub fn reset(&mut self) {
        self.lists.iter_mut().for_each(Vec::clear);
        self.flags = [false; 3];
    }

    pub fn global(&self, i: usize) -> bool {
        self.flags.get(i).copied().unwrap_or(false)
    }

    pub fn on_race_started(&mut self) {
        self.flags[RACE_STARTED] = true;
        self.flags[CROSSED_FINISH_LINE] = false;
    }

    pub fn on_crossed_finish_line(&mut self) {
        self.flags[CROSSED_FINISH_LINE] = true;
    }

    pub fn on_race_ended(&mut self) {
        self.flags[RACE_ENDED] = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_seen_for_the_update_after_its_post() {
        let mut e = PostedEvents::default();
        let (door, heli, cam) = (Name::new("DoorWarningLightOn"), Name::new("DropHeli"), Name::new("TheCamera"));
        e.post(door.clone(), Some(heli.clone()));
        assert!(!e.posted(&door, None), "not before the swap");
        e.swap();
        assert!(e.posted(&door, None) && e.posted(&door, Some(&heli)) && !e.posted(&door, Some(&cam)));
        e.swap();
        assert!(!e.posted(&door, None), "gone after the next swap");
        e.post(door.clone(), None);
        e.swap();
        assert!(e.posted(&door, Some(&cam)), "a null entity matches any");
        e.on_race_started();
        e.on_crossed_finish_line();
        assert!(e.global(RACE_STARTED) && e.global(CROSSED_FINISH_LINE) && !e.global(RACE_ENDED));
        e.on_race_started();
        assert!(!e.global(CROSSED_FINISH_LINE));
        e.reset();
        assert!(!e.global(RACE_STARTED) && e.current().is_empty());
    }
}
