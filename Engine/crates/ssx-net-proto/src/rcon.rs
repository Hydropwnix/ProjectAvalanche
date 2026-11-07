use core::fmt;

use crypto_box::aead::OsRng;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::JoinError;
use crate::join::{CHALLENGE_SIZE, Cursor, ENVELOPE, SEAL, SIGNATURE, open_envelope, seal_envelope};
use crate::keys::{KEY_SIZE, ReplyKey, ServerKey, ServerSecret, to_hex};

pub const RCON_VERSION: u32 = 1;
pub const MAX_COMMAND_BYTES: usize = 1024;
pub const MAX_ANSWER_LINE: usize = 1024;
pub const MAX_ANSWER_LINES: usize = 1024;
pub const MAX_ANSWER_TEXT: usize = 64 * 1024;
pub const MAX_REFUSAL_TEXT: usize = crate::join::MAX_REFUSAL_TEXT;

const REQUEST_MAGIC: [u8; 4] = *b"SSXC";
const ANSWER_MAGIC: [u8; 4] = *b"SSXO";
const SIGNATURE_DOMAIN: &[u8] = b"SSXA rcon request v1\0";
const STABLE_PREFIX: usize = 4 + KEY_SIZE;
const FIXED: usize = STABLE_PREFIX + CHALLENGE_SIZE + KEY_SIZE + 2;
const MIN_INNER: usize = FIXED + 1 + SIGNATURE;
const MAX_INNER: usize = FIXED + MAX_COMMAND_BYTES + SIGNATURE;
pub const MIN_REQUEST_BODY: usize = ENVELOPE + SEAL + MIN_INNER;
pub const MAX_REQUEST_BODY: usize = ENVELOPE + SEAL + MAX_INNER;
pub const MAX_ANSWER_BODY: usize = ENVELOPE + SEAL + 1 + 1 + 2 + 2 * MAX_ANSWER_LINES + MAX_ANSWER_TEXT;
const MIN_ANSWER_BODY: usize = ENVELOPE + SEAL + 1 + 3;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperatorKey([u8; KEY_SIZE]);

impl OperatorKey {
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Result<Self, crate::KeyError> {
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| crate::KeyError::NotAPoint)?;
        if key.is_weak() {
            return Err(crate::KeyError::LowOrder("the operator key"));
        }
        Ok(Self(bytes))
    }

    pub fn from_hex(text: &str) -> Result<Self, crate::KeyError> {
        Self::from_bytes(crate::keys::from_hex(text)?)
    }

    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        to_hex(&self.0)
    }

    pub fn id(&self) -> OperatorId {
        OperatorId(Sha256::digest(self.0).into())
    }

    fn verify(&self, message: &[u8], signature: &[u8; SIGNATURE]) -> bool {
        let Ok(key) = VerifyingKey::from_bytes(&self.0) else { return false };
        key.verify_strict(message, &Signature::from_bytes(signature)).is_ok()
    }
}

impl fmt::Debug for OperatorKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OperatorKey({})", self.to_hex())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperatorId(pub [u8; KEY_SIZE]);

impl OperatorId {
    pub fn short(&self) -> String {
        to_hex(&self.0)[..16].to_string()
    }
}

impl fmt::Debug for OperatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OperatorId({})", self.short())
    }
}

impl fmt::Display for OperatorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

pub struct OperatorSecret(SigningKey);

impl OperatorSecret {
    pub fn generate() -> Self {
        Self(SigningKey::generate(&mut OsRng))
    }

    pub fn from_bytes(seed: [u8; KEY_SIZE]) -> Result<Self, crate::KeyError> {
        let secret = Self(SigningKey::from_bytes(&seed));
        OperatorKey::from_bytes(secret.0.verifying_key().to_bytes())?;
        Ok(secret)
    }

    pub fn to_bytes(&self) -> Zeroizing<[u8; KEY_SIZE]> {
        Zeroizing::new(self.0.to_bytes())
    }

    pub fn public(&self) -> OperatorKey {
        OperatorKey(self.0.verifying_key().to_bytes())
    }
}

pub fn command_text(text: &str) -> Result<&str, JoinError> {
    if text.is_empty() || text.len() > MAX_COMMAND_BYTES {
        return Err(JoinError::Size { what: "the command", min: 1, max: MAX_COMMAND_BYTES, actual: text.len() });
    }
    if text.chars().any(char::is_control) {
        return Err(JoinError::Field("the command has a control character (one line only)"));
    }
    if text.chars().all(|c| c == ' ') {
        return Err(JoinError::Field("the command is only spaces"));
    }
    Ok(text)
}

pub struct RconRequest {
    pub rcon_version: u32,
    pub reply: ReplyKey,
    pub challenge: [u8; CHALLENGE_SIZE],
    pub operator: OperatorKey,
    pub command: String,
}

fn signed_message(server: &ServerKey, inner_unsigned: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut m = Zeroizing::new(Vec::with_capacity(SIGNATURE_DOMAIN.len() + KEY_SIZE + inner_unsigned.len()));
    m.extend_from_slice(SIGNATURE_DOMAIN);
    m.extend_from_slice(server.as_bytes());
    m.extend_from_slice(inner_unsigned);
    m
}

impl RconRequest {
    pub fn seal(&self, server: &ServerKey, signer: &OperatorSecret) -> Result<Vec<u8>, JoinError> {
        seal_rcon_inner(&self.signed_bytes(server, signer)?, server)
    }

    pub fn signed_bytes(&self, server: &ServerKey, signer: &OperatorSecret) -> Result<Zeroizing<Vec<u8>>, JoinError> {
        if signer.public() != self.operator {
            return Err(JoinError::Field("the signing key is not the request's operator key"));
        }
        let command = command_text(&self.command)?;
        let mut inner = Zeroizing::new(Vec::with_capacity(MAX_INNER));
        inner.extend_from_slice(&self.rcon_version.to_be_bytes());
        inner.extend_from_slice(self.reply.as_bytes());
        inner.extend_from_slice(&self.challenge);
        inner.extend_from_slice(self.operator.as_bytes());
        inner.extend_from_slice(&(command.len() as u16).to_be_bytes());
        inner.extend_from_slice(command.as_bytes());
        let signature = signer.0.sign(&signed_message(server, &inner)).to_bytes();
        inner.extend_from_slice(&signature);
        Ok(inner)
    }
}

pub fn seal_rcon_inner(inner: &[u8], server: &ServerKey) -> Result<Vec<u8>, JoinError> {
    seal_envelope(REQUEST_MAGIC, &server.box_key(), inner)
}

pub struct OpenedRcon(Zeroizing<Vec<u8>>);

pub fn open_rcon(body: &[u8], secret: &ServerSecret) -> Result<OpenedRcon, JoinError> {
    open_envelope("rcon request", REQUEST_MAGIC, body, (MIN_REQUEST_BODY, MAX_REQUEST_BODY), secret.box_secret())
        .map(OpenedRcon)
}

impl OpenedRcon {
    pub fn prefix(&self) -> Result<(u32, ReplyKey), JoinError> {
        let mut r = Cursor(&self.0);
        let version = r.u32()?;
        Ok((version, ReplyKey::from_bytes(r.array()?)?))
    }

    pub fn request(&self, server: &ServerKey) -> Result<RconRequest, JoinError> {
        let bytes: &[u8] = &self.0;
        if bytes.len() < MIN_INNER || bytes.len() > MAX_INNER {
            return Err(JoinError::Size { what: "rcon request", min: MIN_INNER, max: MAX_INNER, actual: bytes.len() });
        }
        let (rcon_version, reply) = self.prefix()?;
        if rcon_version != RCON_VERSION {
            return Err(JoinError::Field("another RCON version"));
        }
        let mut r = Cursor(&bytes[STABLE_PREFIX..]);
        let challenge = r.array()?;
        let operator = OperatorKey::from_bytes(r.array()?)?;
        let len = usize::from(r.u16()?);
        let command = r.take(len)?;
        let signature: [u8; SIGNATURE] = r.array()?;
        if !r.0.is_empty() {
            return Err(JoinError::Field("bytes after the signature"));
        }
        let unsigned = &bytes[..bytes.len() - SIGNATURE];
        if !operator.verify(&signed_message(server, unsigned), &signature) {
            return Err(JoinError::Signature);
        }
        let command = core::str::from_utf8(command).map_err(|_| JoinError::Field("the command is not UTF-8"))?;
        let command = command_text(command)?.to_string();
        Ok(RconRequest { rcon_version, reply, challenge, operator, command })
    }
}

mod answer;
pub use answer::{RconAnswer, RconRefusal, RconRefusalCode, seal_answer_inner};

#[cfg(test)]
mod tests;
