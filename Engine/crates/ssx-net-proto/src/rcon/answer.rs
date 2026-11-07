use super::{ANSWER_MAGIC, MAX_ANSWER_BODY, MAX_ANSWER_LINE, MAX_ANSWER_LINES, MAX_ANSWER_TEXT, MAX_REFUSAL_TEXT};
use super::{MIN_ANSWER_BODY, open_envelope, seal_envelope};
use crate::JoinError;
use crate::join::Cursor;
use crate::keys::{ReplyKey, ReplySecret};

pub fn seal_answer_inner(inner: &[u8], reply: &ReplyKey) -> Result<Vec<u8>, JoinError> {
    seal_envelope(ANSWER_MAGIC, &reply.box_key(), inner)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RconRefusalCode {
    Version = 1,
    Request = 2,
    Challenge = 3,
    UnknownKey = 4,
    Limited = 5,
    Command = 6,
    Denied = 7,
    Busy = 8,
}

impl RconRefusalCode {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            1 => Self::Version,
            2 => Self::Request,
            3 => Self::Challenge,
            4 => Self::UnknownKey,
            5 => Self::Limited,
            6 => Self::Command,
            7 => Self::Denied,
            8 => Self::Busy,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RconRefusal {
    pub code: RconRefusalCode,
    pub retry_after_secs: u16,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RconAnswer {
    Ran { ok: bool, lines: Vec<String> },
    Refused(RconRefusal),
}

fn line_ok(line: &str) -> bool {
    line.len() <= MAX_ANSWER_LINE && !line.chars().any(char::is_control)
}

impl RconAnswer {
    const RAN: u8 = 1;
    const REFUSED: u8 = 2;

    pub fn ran(ok: bool, lines: impl IntoIterator<Item = String>) -> Self {
        let mut out = Vec::new();
        let mut total = 0usize;
        let mut cut = 0usize;
        for line in lines {
            let mut clean: String = line.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
            while clean.len() > MAX_ANSWER_LINE {
                clean.pop();
            }
            if out.len() + 1 >= MAX_ANSWER_LINES || total + clean.len() + 64 > MAX_ANSWER_TEXT {
                cut += 1;
                continue;
            }
            total += clean.len();
            out.push(clean);
        }
        if cut > 0 {
            out.push(format!("({cut} more lines are not shown)"));
        }
        Self::Ran { ok, lines: out }
    }

    pub fn refused(code: RconRefusalCode, retry_after_secs: u16, text: &str) -> Self {
        let mut text: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        while text.len() > MAX_REFUSAL_TEXT {
            text.pop();
        }
        Self::Refused(RconRefusal { code, retry_after_secs, text })
    }

    pub fn seal(&self, reply: &ReplyKey) -> Result<Vec<u8>, JoinError> {
        let mut inner = Vec::new();
        match self {
            Self::Ran { ok, lines } => {
                let total: usize = lines.iter().map(String::len).sum();
                if lines.len() > MAX_ANSWER_LINES || total > MAX_ANSWER_TEXT || !lines.iter().all(|l| line_ok(l)) {
                    return Err(JoinError::Field("an answer beyond the RCON answer's limits"));
                }
                inner.push(Self::RAN);
                inner.push(u8::from(*ok));
                inner.extend_from_slice(&(lines.len() as u16).to_be_bytes());
                for line in lines {
                    inner.extend_from_slice(&(line.len() as u16).to_be_bytes());
                    inner.extend_from_slice(line.as_bytes());
                }
            }
            Self::Refused(r) => {
                if r.text.len() > MAX_REFUSAL_TEXT || r.text.chars().any(char::is_control) {
                    return Err(JoinError::Field("a refusal's text beyond its limits"));
                }
                inner.push(Self::REFUSED);
                inner.push(r.code as u8);
                inner.extend_from_slice(&r.retry_after_secs.to_be_bytes());
                inner.push(r.text.len() as u8);
                inner.extend_from_slice(r.text.as_bytes());
            }
        }
        seal_envelope(ANSWER_MAGIC, &reply.box_key(), &inner)
    }

    pub fn open(body: &[u8], reply: &ReplySecret) -> Result<Self, JoinError> {
        let inner =
            open_envelope("rcon answer", ANSWER_MAGIC, body, (MIN_ANSWER_BODY, MAX_ANSWER_BODY), reply.box_secret())?;
        let mut r = Cursor(&inner);
        let answer = match r.u8()? {
            Self::RAN => {
                let ok = match r.u8()? {
                    0 => false,
                    1 => true,
                    _ => return Err(JoinError::Field("an answer's outcome is 0 or 1")),
                };
                let count = usize::from(r.u16()?);
                if count > MAX_ANSWER_LINES {
                    return Err(JoinError::Field("more lines than an answer has"));
                }
                if count * 2 > r.0.len() {
                    return Err(JoinError::Field("the answer ends early"));
                }
                let mut lines = Vec::with_capacity(count);
                let mut total = 0usize;
                for _ in 0..count {
                    let len = usize::from(r.u16()?);
                    let bytes = r.take(len)?;
                    let line = core::str::from_utf8(bytes).map_err(|_| JoinError::Field("a line is not UTF-8"))?;
                    total += line.len();
                    if !line_ok(line) || total > MAX_ANSWER_TEXT {
                        return Err(JoinError::Field("a line beyond the RCON answer's limits"));
                    }
                    lines.push(line.to_string());
                }
                Self::Ran { ok, lines }
            }
            Self::REFUSED => {
                let code = RconRefusalCode::from_byte(r.u8()?).ok_or(JoinError::Field("an unknown refusal code"))?;
                let retry_after_secs = r.u16()?;
                let text = r.field(0, MAX_REFUSAL_TEXT, "the refusal's text")?;
                let text = core::str::from_utf8(text).map_err(|_| JoinError::Field("the refusal is not UTF-8"))?;
                if text.chars().any(char::is_control) {
                    return Err(JoinError::Field("the refusal's text has a control character"));
                }
                Self::Refused(RconRefusal { code, retry_after_secs, text: text.to_string() })
            }
            _ => return Err(JoinError::Field("an unknown answer kind")),
        };
        if !r.0.is_empty() {
            return Err(JoinError::Field("bytes after the answer"));
        }
        Ok(answer)
    }
}
