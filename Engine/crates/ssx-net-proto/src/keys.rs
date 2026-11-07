use core::fmt;

use crypto_box::aead::OsRng;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::KeyError;

pub const KEY_SIZE: usize = 32;

pub fn to_hex(bytes: &[u8; KEY_SIZE]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(text: &str) -> Result<[u8; KEY_SIZE], KeyError> {
    let digits = text.as_bytes();
    if digits.len() != 2 * KEY_SIZE {
        return Err(KeyError::Hex(digits.len()));
    }
    let nibble = |d: u8| char::from(d).to_digit(16).map(|v| v as u8).ok_or(KeyError::Hex(digits.len()));
    let mut out = [0u8; KEY_SIZE];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = (nibble(digits[2 * i])? << 4) | nibble(digits[2 * i + 1])?;
    }
    Ok(out)
}

fn contributory(key: &[u8; KEY_SIZE]) -> bool {
    let probe = crypto_box::SecretKey::generate(&mut OsRng);
    let shared = x25519_dalek::x25519(probe.to_bytes(), *key);
    !bool::from(shared.ct_eq(&[0u8; KEY_SIZE]))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerKey([u8; KEY_SIZE]);

impl ServerKey {
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Result<Self, KeyError> {
        if !contributory(&bytes) {
            return Err(KeyError::LowOrder("the server key"));
        }
        Ok(Self(bytes))
    }

    pub fn from_hex(text: &str) -> Result<Self, KeyError> {
        Self::from_bytes(from_hex(text)?)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    pub(crate) fn box_key(&self) -> crypto_box::PublicKey {
        crypto_box::PublicKey::from_bytes(self.0)
    }
}

impl fmt::Debug for ServerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServerKey({})", self.to_hex())
    }
}

pub struct ServerSecret(crypto_box::SecretKey);

impl ServerSecret {
    pub fn generate() -> Self {
        Self(crypto_box::SecretKey::generate(&mut OsRng))
    }

    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Result<Self, KeyError> {
        let secret = Self(crypto_box::SecretKey::from_bytes(bytes));
        ServerKey::from_bytes(secret.0.public_key().to_bytes())?;
        Ok(secret)
    }

    pub fn to_bytes(&self) -> [u8; KEY_SIZE] {
        self.0.to_bytes()
    }

    pub fn public(&self) -> ServerKey {
        ServerKey(self.0.public_key().to_bytes())
    }

    pub(crate) fn box_secret(&self) -> &crypto_box::SecretKey {
        &self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerKey([u8; KEY_SIZE]);

impl PlayerKey {
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Result<Self, KeyError> {
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| KeyError::NotAPoint)?;
        if key.is_weak() {
            return Err(KeyError::LowOrder("the player key"));
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    pub fn id(&self) -> PlayerId {
        PlayerId(Sha256::digest(self.0).into())
    }

    pub(crate) fn verify(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        let Ok(key) = VerifyingKey::from_bytes(&self.0) else { return false };
        key.verify_strict(message, &Signature::from_bytes(signature)).is_ok()
    }
}

impl fmt::Debug for PlayerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PlayerKey({})", to_hex(&self.0))
    }
}

pub struct PlayerSecret(SigningKey);

impl PlayerSecret {
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut OsRng))
    }

    pub fn from_bytes(seed: [u8; KEY_SIZE]) -> Result<Self, KeyError> {
        let secret = Self(SigningKey::from_bytes(&seed));
        PlayerKey::from_bytes(secret.0.verifying_key().to_bytes())?;
        Ok(secret)
    }

    pub fn to_bytes(&self) -> [u8; KEY_SIZE] {
        self.0.to_bytes()
    }

    pub fn public(&self) -> PlayerKey {
        PlayerKey(self.0.verifying_key().to_bytes())
    }

    pub(crate) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PlayerId(pub [u8; KEY_SIZE]);

impl PlayerId {
    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    pub fn short(&self) -> String {
        self.to_hex()[..16].to_string()
    }
}

impl fmt::Debug for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PlayerId({})", self.short())
    }
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

pub struct ReplySecret(crypto_box::SecretKey);

impl ReplySecret {
    pub fn generate() -> Self {
        Self(crypto_box::SecretKey::generate(&mut OsRng))
    }

    pub fn public(&self) -> ReplyKey {
        ReplyKey(self.0.public_key().to_bytes())
    }

    pub(crate) fn box_secret(&self) -> &crypto_box::SecretKey {
        &self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReplyKey([u8; KEY_SIZE]);

impl ReplyKey {
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Result<Self, KeyError> {
        if !contributory(&bytes) {
            return Err(KeyError::LowOrder("the reply key"));
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    pub(crate) fn box_key(&self) -> crypto_box::PublicKey {
        crypto_box::PublicKey::from_bytes(self.0)
    }
}

impl fmt::Debug for ReplyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ReplyKey({})", to_hex(&self.0))
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    use crypto_box::aead::rand_core::RngCore;
    let mut out = [0u8; N];
    OsRng.fill_bytes(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_takes_64_hex_digits_only() {
        assert_eq!(from_hex(&"0f".repeat(32)), Ok([0x0F; 32]));
        assert_eq!(from_hex(&"AB".repeat(32)), Ok([0xAB; 32]));
        for bad in ["", "0", &"0".repeat(63), &"0".repeat(65), &format!("{}g", "0".repeat(63)), &"é".repeat(32)] {
            assert!(from_hex(bad).is_err(), "{bad:?}");
        }
        let key = ServerSecret::generate().public();
        assert_eq!(ServerKey::from_hex(&key.to_hex()), Ok(key));
    }
}
