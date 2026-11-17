use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ssx_discord::frame::{self, Decoder, Opcode};

pub const READY: &str = r#"{"cmd":"DISPATCH","data":{"v":1,"config":{"cdn_host":"cdn.discordapp.com","api_endpoint":"//discord.com/api","environment":"production"},"user":{"id":"53908232506183680","username":"Mason","discriminator":"1337","avatar":null}},"evt":"READY"}"#;

#[derive(Clone, Debug, PartialEq)]
pub enum Behaviour {
    Normal,
    NoReady,
    Oversized,
    BadOpcode,
    PingAfterReady,
    Close(i64, String),
    RefuseActivities,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Seen {
    Connected,
    Handshake(serde_json::Value),
    Activity { at: Instant, payload: serde_json::Value },
    Pong(Vec<u8>),
    Other(Opcode, Vec<u8>),
    Disconnected,
}

pub struct FakeDiscord {
    pub path: PathBuf,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    kick: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeDiscord {
    pub fn start(path: &Path, behaviour: Behaviour) -> FakeDiscord {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path).expect("bind the fake's socket");
        listener.set_nonblocking(true).expect("non-blocking listener");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (stop, kick) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
        let (s, st, k) = (Arc::clone(&seen), Arc::clone(&stop), Arc::clone(&kick));
        let thread = std::thread::spawn(move || serve(listener, behaviour, s, st, k));
        FakeDiscord { path: path.to_path_buf(), seen, stop, kick, thread: Some(thread) }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn activities(&self) -> Vec<(Instant, serde_json::Value)> {
        self.seen()
            .into_iter()
            .filter_map(|s| match s {
                Seen::Activity { at, payload } => Some((at, payload)),
                _ => None,
            })
            .collect()
    }

    pub fn count(&self, f: impl Fn(&Seen) -> bool) -> usize {
        self.seen().iter().filter(|s| f(s)).count()
    }

    pub fn wait_for(&self, limit: Duration, f: impl Fn(&FakeDiscord) -> bool) -> bool {
        let end = Instant::now() + limit;
        while Instant::now() < end {
            if f(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        f(self)
    }

    pub fn kick(&self) {
        self.kick.store(true, Ordering::SeqCst);
    }
}

impl Drop for FakeDiscord {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

fn send(s: &mut UnixStream, op: Opcode, payload: &[u8]) -> bool {
    s.write_all(&frame::encode(op, payload).expect("a frame")).is_ok()
}

fn serve(
    listener: UnixListener,
    behaviour: Behaviour,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
    kick: Arc<AtomicBool>,
) {
    let log = |s: Seen| seen.lock().unwrap().push(s);
    while !stop.load(Ordering::SeqCst) {
        let Ok((mut s, _)) = listener.accept() else {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        };
        log(Seen::Connected);
        kick.store(false, Ordering::SeqCst);
        s.set_read_timeout(Some(Duration::from_millis(10))).unwrap();
        let mut decoder = Decoder::new();
        let mut buf = [0u8; 4096];
        'conn: while !stop.load(Ordering::SeqCst) && !kick.load(Ordering::SeqCst) {
            let n = match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Err(_) => break,
            };
            let Ok(frames) = decoder.feed(&buf[..n]) else { break };
            for f in frames {
                let json = serde_json::from_slice::<serde_json::Value>(&f.payload).unwrap_or_default();
                match f.opcode {
                    Opcode::Handshake => {
                        log(Seen::Handshake(json));
                        let ok = match &behaviour {
                            Behaviour::NoReady => true,
                            Behaviour::Close(code, msg) => {
                                let body = serde_json::json!({ "code": code, "message": msg }).to_string();
                                send(&mut s, Opcode::Close, body.as_bytes());
                                break 'conn;
                            }
                            _ => send(&mut s, Opcode::Frame, READY.as_bytes()),
                        };
                        let after = match &behaviour {
                            Behaviour::Oversized => {
                                let mut h = 1u32.to_le_bytes().to_vec();
                                h.extend_from_slice(&(frame::MAX_PAYLOAD as u32 + 1).to_le_bytes());
                                s.write_all(&h).is_ok()
                            }
                            Behaviour::BadOpcode => {
                                let mut h = 9u32.to_le_bytes().to_vec();
                                h.extend_from_slice(&2u32.to_le_bytes());
                                h.extend_from_slice(b"{}");
                                s.write_all(&h).is_ok()
                            }
                            Behaviour::PingAfterReady => send(&mut s, Opcode::Ping, b"{\"ping\":42}"),
                            _ => true,
                        };
                        if !(ok && after) {
                            break 'conn;
                        }
                    }
                    Opcode::Frame if json["cmd"] == "SET_ACTIVITY" => {
                        log(Seen::Activity { at: Instant::now(), payload: json.clone() });
                        let reply = if behaviour == Behaviour::RefuseActivities {
                            serde_json::json!({ "cmd": "SET_ACTIVITY", "evt": "ERROR", "nonce": json["nonce"],
                                "data": { "code": 4000, "message": "child \"activity\" fails because [\u{1b}[31mred]" } })
                        } else {
                            serde_json::json!({ "cmd": "SET_ACTIVITY", "evt": null, "nonce": json["nonce"],
                                "data": json["args"]["activity"] })
                        };
                        if !send(&mut s, Opcode::Frame, reply.to_string().as_bytes()) {
                            break 'conn;
                        }
                    }
                    Opcode::Pong => log(Seen::Pong(f.payload)),
                    op => log(Seen::Other(op, f.payload)),
                }
            }
        }
        log(Seen::Disconnected);
    }
}
