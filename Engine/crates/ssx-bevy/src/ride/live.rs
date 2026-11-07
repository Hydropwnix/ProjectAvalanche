#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveRewinds {
    Event,
    Off,
    Limit(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRules {
    pub explore_ghosts: bool,
    pub rewinds: LiveRewinds,
    pub rounds: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartGate {
    #[default]
    Free,
    Hold,
    Go {
        pressed: bool,
    },
}
