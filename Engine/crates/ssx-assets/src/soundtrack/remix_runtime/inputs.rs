#[derive(Clone, Debug)]
pub struct GameInputs {
    pub(super) events: Vec<&'static str>,
    pub carve: f32,
    pub land_intensity: f32,
    pub tunnel_size: i32,
    pub pitch: f32,
    pub ms_in_air: i32,
    pub ms_to_land: i32,
    pub oxygen: f32,
    pub temperature: f32,
    pub health: f32,
    pub race_event_type: i32,
    pub enabled: bool,
    pub(super) rewind_latch: bool,
    pub armed: bool,
    pub(super) dead_flag: bool,
    pub(super) ended_flag: bool,
    pub(super) death_latch: bool,
    pub(super) death_timer: f32,
    pub reset: bool,
}

impl Default for GameInputs {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            carve: 0.01,
            land_intensity: 0.0,
            tunnel_size: 0,
            pitch: 1.0,
            ms_in_air: 0,
            ms_to_land: 0,
            oxygen: 1.0,
            temperature: 1.0,
            health: 1.0,
            race_event_type: -1,
            enabled: false,
            rewind_latch: false,
            armed: false,
            dead_flag: false,
            ended_flag: false,
            death_latch: false,
            death_timer: 0.0,
            reset: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameInputs {
    pub ms_in_air: i32,
    pub ms_to_land: i32,
    pub oxygen: f32,
    pub temperature: f32,
    pub health: f32,
    pub rewinding: bool,
    pub dead_by_death_plane: bool,
    pub ended_by_death: bool,
    pub dt_ms: f32,
}

pub(super) const EVENT_ORDER: [&str; 26] = [
    "Jump",
    "Land",
    "Wingsuit Open",
    "Wingsuit Close",
    "Time Dilation Start",
    "Time Dilation Stop",
    "Boost Start",
    "Boost Stop",
    "Tumble Start",
    "Tumble Stop",
    "Awesome Mode Start",
    "Awesome Mode Stop",
    "Super Awesome Mode Start",
    "Super Awesome Mode Stop",
    "Awesome Land Start",
    "Awesome Land Stop",
    "Grinding Start",
    "Grinding Stop",
    "Tunnel Start",
    "Tunnel Stop",
    "Player Death Start",
    "Player Death Stop",
    "Rewind Start",
    "Rewind Stop",
    "Music Ducker Start",
    "Music Ducker Stop",
];

impl GameInputs {
    pub fn pending_events(&self) -> &[&'static str] {
        &self.events
    }

    pub(super) fn flag(&mut self, e: &'static str) {
        if !self.events.contains(&e) {
            self.events.push(e);
        }
    }

    pub fn music_ducker(&mut self, on: bool) {
        self.flag(if on { "Music Ducker Start" } else { "Music Ducker Stop" });
    }

    pub fn post(&mut self, name: &str, value: f32) {
        if name == "RaceEventType" {
            self.race_event_type = value as i32;
            return;
        }
        if !self.enabled {
            return;
        }
        let ev = match name {
            "UpdateCarve" => {
                self.carve = value;
                return;
            }
            "UpdatePitch" => {
                self.pitch = value;
                return;
            }
            "Land" => {
                self.land_intensity = value;
                "Land"
            }
            "Jump" => "Jump",
            "WingsuitDeployed" => "Wingsuit Open",
            "WingsuitRetracted" => "Wingsuit Close",
            "TimeDilationStart" => "Time Dilation Start",
            "TimeDilationStop" => "Time Dilation Stop",
            "BoostStart" => "Boost Start",
            "BoostStop" => "Boost Stop",
            "TumbleStart" => "Tumble Start",
            "TumbleStop" => "Tumble Stop",
            "AwesomeModeStart" => "Awesome Mode Start",
            "AwesomeModeStop" => "Awesome Mode Stop",
            "SuperAwesomeModeStart" => "Super Awesome Mode Start",
            "SuperAwesomeModeStop" => "Super Awesome Mode Stop",
            "AwesomeLandStart" => "Awesome Land Start",
            "AwesomeLandStop" => "Awesome Land Stop",
            "GrindingStart" => "Grinding Start",
            "GrindingStop" => "Grinding Stop",
            "TunnelStart" => {
                self.tunnel_size = value as i32;
                "Tunnel Start"
            }
            "TunnelStop" => "Tunnel Stop",
            "PlayerDeath" => "Player Death Start",
            _ => return,
        };
        self.flag(ev);
    }

    pub fn frame(&mut self, f: &FrameInputs) {
        if self.enabled {
            (self.ms_in_air, self.ms_to_land) = (f.ms_in_air, f.ms_to_land);
            (self.oxygen, self.temperature, self.health) = (f.oxygen, f.temperature, f.health);
            if f.rewinding && !self.rewind_latch {
                self.flag("Rewind Start");
                self.rewind_latch = true;
            } else if !f.rewinding && self.rewind_latch {
                self.flag("Rewind Stop");
                self.rewind_latch = false;
            }
        }
        (self.dead_flag, self.ended_flag) = (f.dead_by_death_plane, f.ended_by_death);
    }

    pub fn events_frame(&mut self, now_ms: f64) {
        let dead = self.dead_flag || self.ended_flag;
        if dead && self.armed {
            if !self.death_latch {
                self.post("PlayerDeath", 0.0);
                self.death_latch = true;
                self.death_timer = (now_ms + 2000.0) as f32;
            }
        } else if !dead && self.death_latch {
            (self.death_timer, self.death_latch) = (0.0, false);
        }
        if self.death_timer > 0.0 && self.death_latch && self.ended_flag && f64::from(self.death_timer) < now_ms {
            self.enabled = false;
            self.reset = true;
            (self.death_timer, self.death_latch) = (0.0, false);
        }
    }
}
