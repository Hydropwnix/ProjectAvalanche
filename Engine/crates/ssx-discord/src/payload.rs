use serde_json::{Map, Value};

use crate::activity::Activity;

pub const RPC_VERSION: u64 = 1;
pub const MESSAGE_CHARS: usize = 160;
const NAME_MAX: usize = 64;
const NONCE_MAX: usize = 32;

pub fn valid_client_id(id: &str) -> bool {
    id.bytes().all(|b| b.is_ascii_digit()) && !id.starts_with('0') && id.parse::<u64>().is_ok()
}

pub fn handshake(client_id: &str) -> Vec<u8> {
    serde_json::json!({ "v": RPC_VERSION, "client_id": client_id }).to_string().into_bytes()
}

pub fn set_activity(pid: u32, activity: &Activity, nonce: u64) -> Vec<u8> {
    serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": pid, "activity": activity.to_json() },
        "nonce": nonce.to_string(),
    })
    .to_string()
    .into_bytes()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Ready,
    Reply { nonce: u64, error: Option<RpcError> },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcError {
    pub code: Option<i64>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadError {
    Json(String),
    NotObject,
    Member(&'static str),
}

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadError::Json(e) => write!(f, "not JSON: {e}"),
            PayloadError::NotObject => f.write_str("JSON, but not an object"),
            PayloadError::Member(m) => write!(f, "its member `{m}` has the wrong type or length"),
        }
    }
}

impl std::error::Error for PayloadError {}

fn object(bytes: &[u8]) -> Result<Map<String, Value>, PayloadError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| PayloadError::Json(ssx_core::text::shown_max(&e.to_string(), MESSAGE_CHARS).to_string()))?;
    match value {
        Value::Object(m) => Ok(m),
        _ => Err(PayloadError::NotObject),
    }
}

fn name<'a>(m: &'a Map<String, Value>, key: &'static str, max: usize) -> Result<Option<&'a str>, PayloadError> {
    match m.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.len() <= max => Ok(Some(s)),
        Some(_) => Err(PayloadError::Member(key)),
    }
}

fn error_of(data: Option<&Value>) -> RpcError {
    let data = data.and_then(Value::as_object);
    let code = data.and_then(|d| d.get("code")).and_then(Value::as_i64);
    let message = data.and_then(|d| d.get("message")).and_then(Value::as_str).unwrap_or("");
    RpcError { code, message: ssx_core::text::shown_max(message, MESSAGE_CHARS).to_string() }
}

pub fn inbound(bytes: &[u8]) -> Result<Inbound, PayloadError> {
    let m = object(bytes)?;
    let cmd = name(&m, "cmd", NAME_MAX)?;
    let evt = name(&m, "evt", NAME_MAX)?;
    let nonce = name(&m, "nonce", NONCE_MAX)?;
    if cmd == Some("DISPATCH") && evt == Some("READY") {
        return Ok(Inbound::Ready);
    }
    let Some(nonce) = nonce.and_then(|n| n.parse::<u64>().ok()) else {
        return Ok(Inbound::Other);
    };
    let error = (evt == Some("ERROR")).then(|| error_of(m.get("data")));
    Ok(Inbound::Reply { nonce, error })
}

pub fn close_reason(bytes: &[u8]) -> RpcError {
    match object(bytes) {
        Ok(m) => error_of(Some(&Value::Object(m))),
        Err(_) => RpcError { code: None, message: String::new() },
    }
}

#[cfg(test)]
mod tests;
