use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Error)]
pub enum DecodeError {
    #[error("{kind}: {actual} bytes, expected exactly {expected}")]
    Size { kind: &'static str, expected: usize, actual: usize },

    #[error("pose sample: reserved flag bits set ({0:#010x})")]
    ReservedFlags(u32),

    #[error("pose sample: the time {0} is not a network tick")]
    Time(f64),

    #[error("pose sample: the execution index {0} is beyond the wire's 24 bits")]
    Execution(u32),

    #[error("pose sample: the view time {0} lies beyond the wire's 32-bit lead over the time stamp")]
    ViewTime(f64),

    #[error("pose block: {0} is not finite")]
    NonFinite(&'static str),

    #[error("pose block: pose weight {0} (the packer always stores 1.0)")]
    PoseWeight(f32),

    #[error("pose block: the bytes the packer never writes (+0x24…+0x2F, +0x47) are not zero")]
    UnwrittenBytes,

    #[error("pose block: {0}")]
    Block(String),

    #[error("{0}")]
    Field(&'static str),

    #[error("{kind}: {detail}")]
    Text { kind: &'static str, detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum KeyError {
    #[error("a key is 64 hex digits, not {0} characters")]
    Hex(usize),

    #[error("{0} is a low-order point")]
    LowOrder(&'static str),

    #[error("the player key is not a point of the curve")]
    NotAPoint,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JoinError {
    #[error("{what}: {actual} bytes, expected {min}…{max}")]
    Size { what: &'static str, min: usize, max: usize, actual: usize },

    #[error("{0}: not this protocol's envelope (magic or version)")]
    Envelope(&'static str),

    #[error("{0}: the sealed box does not open with this key")]
    Seal(&'static str),

    #[error("{0}")]
    Field(&'static str),

    #[error("{0}")]
    Key(#[from] KeyError),

    #[error("the signature does not verify")]
    Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NameError {
    #[error("the name is empty")]
    Empty,

    #[error("the name has {actual} characters, at most {max}")]
    TooLong { max: usize, actual: usize },

    #[error("the game's fonts cannot draw {0:?} (U+{code:04X})", code = u32::from(*.0))]
    Undrawable(char),
}
