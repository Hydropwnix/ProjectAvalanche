use crate::keys::{KEY_SIZE, PlayerKey};
use crate::{DecodeError, KeyError};

pub const TICKET_SIZE: usize = 256;

const MAGIC: [u8; 4] = *b"SSXT";
const VERSION: u8 = 1;
const USED: usize = 16 + KEY_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionTicket {
    pub join_id: u64,
    pub player: PlayerKey,
}

impl SessionTicket {
    pub fn encode(&self) -> [u8; TICKET_SIZE] {
        let mut out = [0u8; TICKET_SIZE];
        out[..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[8..16].copy_from_slice(&self.join_id.to_be_bytes());
        out[16..USED].copy_from_slice(self.player.as_bytes());
        out
    }

    pub fn decode(bytes: &[u8; TICKET_SIZE]) -> Result<Self, DecodeError> {
        if bytes[..4] != MAGIC || bytes[4] != VERSION || bytes[5..8] != [0; 3] {
            return Err(DecodeError::Field("session ticket: not this protocol's ticket"));
        }
        if bytes[USED..].iter().any(|&b| b != 0) {
            return Err(DecodeError::Field("session ticket: the padding is not zero"));
        }
        let mut join_id = [0u8; 8];
        join_id.copy_from_slice(&bytes[8..16]);
        let mut key = [0u8; KEY_SIZE];
        key.copy_from_slice(&bytes[16..USED]);
        let player = PlayerKey::from_bytes(key)
            .map_err(|e: KeyError| DecodeError::Text { kind: "session ticket", detail: e.to_string() })?;
        Ok(Self { join_id: u64::from_be_bytes(join_id), player })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::PlayerSecret;

    #[test]
    fn round_trip_and_refusals() {
        let t = SessionTicket { join_id: 0x0102_0304_0506_0708, player: PlayerSecret::generate().public() };
        assert_eq!(SessionTicket::decode(&t.encode()), Ok(t));
        assert!(SessionTicket::decode(&[0u8; TICKET_SIZE]).is_err(), "a token without a ticket (zero user data)");
        for i in (0..8).chain(USED..TICKET_SIZE) {
            let mut b = t.encode();
            b[i] ^= 0x80;
            assert!(SessionTicket::decode(&b).is_err(), "byte {i}");
        }
    }
}
