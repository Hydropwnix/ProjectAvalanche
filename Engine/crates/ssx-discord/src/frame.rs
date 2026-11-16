use std::fmt;

pub const HEADER: usize = 8;

// PORT(choice): a frame's payload is at most 64 KiB either way; a longer one ends the connection. Discord documents no
// limit; the frames the presence reads are the handshake's READY (about 1 KiB with the user and the configuration),
// the replies to SET_ACTIVITY (the activity echoed, under 4 KiB) and PING. The console had no Discord.
pub const MAX_PAYLOAD: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opcode {
    Handshake,
    Frame,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    pub fn number(self) -> u32 {
        match self {
            Opcode::Handshake => 0,
            Opcode::Frame => 1,
            Opcode::Close => 2,
            Opcode::Ping => 3,
            Opcode::Pong => 4,
        }
    }

    pub fn from_number(n: u32) -> Option<Opcode> {
        Some(match n {
            0 => Opcode::Handshake,
            1 => Opcode::Frame,
            2 => Opcode::Close,
            3 => Opcode::Ping,
            4 => Opcode::Pong,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    Opcode(u32),
    Length(u64),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Opcode(n) => write!(f, "a frame with the opcode {n} (only 0 to 4 exist)"),
            FrameError::Length(n) => write!(f, "a frame of {n} bytes (at most {MAX_PAYLOAD})"),
        }
    }
}

impl std::error::Error for FrameError {}

pub fn encode(opcode: Opcode, payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(FrameError::Length(payload.len() as u64));
    }
    let mut out = Vec::with_capacity(HEADER + payload.len());
    out.extend_from_slice(&opcode.number().to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

#[derive(Debug, Default)]
pub struct Decoder {
    header: Vec<u8>,
    current: Option<(Opcode, usize)>,
    payload: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, mut bytes: &[u8]) -> Result<Vec<Frame>, FrameError> {
        let mut frames = Vec::new();
        while !bytes.is_empty() {
            let Some((opcode, length)) = self.current else {
                let take = (HEADER - self.header.len()).min(bytes.len());
                self.header.extend_from_slice(&bytes[..take]);
                bytes = &bytes[take..];
                if self.header.len() == HEADER {
                    let word = |i: usize| {
                        u32::from_le_bytes([self.header[i], self.header[i + 1], self.header[i + 2], self.header[i + 3]])
                    };
                    let (number, length) = (word(0), word(4));
                    let opcode = Opcode::from_number(number).ok_or(FrameError::Opcode(number))?;
                    let length = usize::try_from(length)
                        .ok()
                        .filter(|&l| l <= MAX_PAYLOAD)
                        .ok_or(FrameError::Length(u64::from(length)))?;
                    self.header.clear();
                    self.current = Some((opcode, length));
                    self.payload = Vec::with_capacity(length);
                    if length == 0 {
                        frames.push(self.finish(opcode));
                    }
                }
                continue;
            };
            let take = (length - self.payload.len()).min(bytes.len());
            self.payload.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.payload.len() == length {
                frames.push(self.finish(opcode));
            }
        }
        Ok(frames)
    }

    pub fn held(&self) -> usize {
        self.header.len() + self.payload.len()
    }

    fn finish(&mut self, opcode: Opcode) -> Frame {
        self.current = None;
        Frame { opcode, payload: std::mem::take(&mut self.payload) }
    }
}

#[cfg(test)]
mod tests;
