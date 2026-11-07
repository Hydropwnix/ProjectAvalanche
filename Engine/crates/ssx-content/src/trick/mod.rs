pub mod template;
mod toml;

#[cfg(test)]
mod tests;

pub use toml::{RawClips, RawTrick, RawTrickTable, parse_toml};

use crate::hash::ContentHash;
use crate::id::ContentId;

pub const TRICK_FILE: &str = "trick.toml";

pub const BASES: [(&str, u8, &str); 12] = [
    ("flyingsquirrel", 2, "flyingsquirrel"),
    ("indy", 3, "indy"),
    ("melon", 4, "melon"),
    ("mute", 5, "mute"),
    ("nose", 6, "nose"),
    ("rocket", 7, "rocket"),
    ("seatbelt", 8, "seatbelt"),
    ("stalefish", 9, "stalefish"),
    ("stiffy", 10, "stiffy"),
    ("tail", 11, "tail"),
    ("truckdriver", 12, "truckdriver"),
    ("wheelman", 13, "nuclear"),
];

pub const ROLES: [(&str, &str); 12] = [
    ("enter", ""),
    ("loop", "_loop"),
    ("tweak", "_tweak"),
    ("tweak_loop", "_tweak_loop"),
    ("uber", "_uber"),
    ("uber_loop", "_uber_loop"),
    ("uber_tweak", "_uber_tweak"),
    ("uber_tweak_loop", "_uber_tweak_loop"),
    ("super_uber", "_super_uber"),
    ("super_uber_loop", "_super_uber_loop"),
    ("super_uber_tweak", "_super_uber_tweak"),
    ("super_uber_tweak_loop", "_super_uber_tweak_loop"),
];

pub const MAX_ADDED_TRICKS: usize = 3;
pub const FIRST_VALUE: u8 = 32;
pub const MIN_POINTS: u16 = 1;
pub const MAX_POINTS: u16 = 2400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Input {
    UpUp,
    LeftLeft,
    RightRight,
}

pub const INPUTS: [Input; 3] = [Input::UpUp, Input::LeftLeft, Input::RightRight];

pub const DISC_INPUTS: [(&str, &str, &str); 12] = [
    ("up", "Rocket", "Rocket"),
    ("up-left", "Flying Squirrel", "Stiffy"),
    ("up-right", "Stiffy", "Flying Squirrel"),
    ("up-down", "Truckdriver", "Truckdriver"),
    ("left", "Melon", "Indy"),
    ("left-right", "Mute", "Stalefish"),
    ("left-up", "Nose", "Wheelman"),
    ("left-down", "Seatbelt", "Tail"),
    ("right", "Indy", "Melon"),
    ("right-left", "Stalefish", "Mute"),
    ("right-up", "Wheelman", "Nose"),
    ("right-down", "Tail", "Seatbelt"),
];

impl Input {
    pub fn name(self) -> &'static str {
        match self {
            Input::UpUp => "up-up",
            Input::LeftLeft => "left-left",
            Input::RightRight => "right-right",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        INPUTS.into_iter().find(|i| i.name() == s)
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn mirror(self) -> Self {
        match self {
            Input::UpUp => Input::UpUp,
            Input::LeftLeft => Input::RightRight,
            Input::RightRight => Input::LeftLeft,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hand {
    Rear,
    Front,
    Both,
}

impl Hand {
    pub fn name(self) -> &'static str {
        match self {
            Hand::Rear => "rear",
            Hand::Front => "front",
            Hand::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Hand::Rear, Hand::Front, Hand::Both].into_iter().find(|h| h.name() == s)
    }

    pub fn of_base(v: u8) -> Self {
        match v {
            3 | 9 | 11 | 13 => Hand::Rear,
            4 | 5 | 6 | 8 => Hand::Front,
            _ => Hand::Both,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrickToml {
    pub base: usize,
    pub input: Input,
    pub hand: Hand,
    pub points: u16,
    pub names: [ContentId; 3],
    pub clips: [ContentId; 12],
}

impl TrickToml {
    pub fn base_value(&self) -> u8 {
        BASES[self.base].1
    }

    pub fn value(&self) -> u8 {
        FIRST_VALUE + self.input.index() as u8
    }

    pub fn base_clip(&self, r: usize) -> String {
        format!("1_trick_air_{}{}", BASES[self.base].2, ROLES[r].1)
    }
}

pub const DOMAIN: &[u8] = b"project-avalanche/trick-sim/1\n";

pub fn sim_hash(t: &TrickToml, clips: &[ContentHash; 12]) -> ContentHash {
    let mut h = blake3::Hasher::new();
    h.update(DOMAIN);
    for x in [u64::from(t.base_value()), t.input.index() as u64, t.hand as u64, u64::from(t.points)] {
        h.update(&x.to_le_bytes());
    }
    for c in clips {
        h.update(&c.0);
    }
    ContentHash(*h.finalize().as_bytes())
}
