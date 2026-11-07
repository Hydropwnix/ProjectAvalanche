pub const PROTOCOL_VERSION: u32 = 12;

pub const API_LEVEL: u32 = 0;

pub const BUILD_ID: &str = concat!("project-avalanche/", env!("CARGO_PKG_VERSION"));

pub const DEFAULT_PORT: u16 = 27450;

pub const PROTOCOL_MAGIC: u32 = u32::from_be_bytes(*b"SSXA");

pub const TICK_HZ: u32 = 30;

pub const MAX_MESSAGE_SIZE: usize = crate::control::ClientControl::MAX_SIZE;
const _: () = assert!(crate::chat::MAX_SERVER_CHAT_SIZE <= MAX_MESSAGE_SIZE);
const _: () = assert!(crate::RELAYED_POSE_SIZE <= MAX_MESSAGE_SIZE);
const _: () = assert!(crate::control::MAX_CONTROL_SIZE <= MAX_MESSAGE_SIZE);
const _: () = assert!(crate::control::ClientControl::MAX_SIZE <= MAX_MESSAGE_SIZE);
const _: () = assert!(crate::chat::MAX_CLIENT_CHAT_SIZE <= MAX_MESSAGE_SIZE);
const _: () = assert!(MAX_MESSAGE_SIZE <= 1024);

pub const fn netcode_protocol_id() -> u64 {
    ((PROTOCOL_MAGIC as u64) << 32) | PROTOCOL_VERSION as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_id_carries_magic_and_version() {
        assert_eq!(netcode_protocol_id() >> 32, u64::from(PROTOCOL_MAGIC));
        assert_eq!(netcode_protocol_id() & 0xFFFF_FFFF, u64::from(PROTOCOL_VERSION));
        assert_eq!(&PROTOCOL_MAGIC.to_be_bytes(), b"SSXA");
    }
}
