use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use eframe::egui;
use ssx_assets::setup::{self, Cancelled, DiscCheck, Progress, Summary};

use crate::command::Invocation;

pub struct Background<T> {
    slot: Arc<Mutex<Option<T>>>,
}

impl<T: Send + 'static> Background<T> {
    pub fn spawn(ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Self {
        let slot = Arc::new(Mutex::new(None));
        let (out, ctx) = (slot.clone(), ctx.clone());
        std::thread::spawn(move || {
            let v = f();
            if let Ok(mut s) = out.lock() {
                *s = Some(v);
            }
            ctx.request_repaint();
        });
        Self { slot }
    }

    pub fn take(&self) -> Option<T> {
        self.slot.lock().ok()?.take()
    }
}

pub fn inspect(ctx: &egui::Context, iso: PathBuf) -> Background<Result<DiscCheck, String>> {
    Background::spawn(ctx, move || setup::inspect(&iso).map_err(|e| format!("{e:#}")))
}

pub enum SetupEnd {
    Done(Summary),
    Cancelled,
    Failed(String),
}

pub struct SetupJob {
    progress: Arc<Mutex<Progress>>,
    cancel: Arc<AtomicBool>,
    end: Background<SetupEnd>,
    pub started: Instant,
}

impl SetupJob {
    pub fn start(ctx: &egui::Context, iso: PathBuf, out: PathBuf) -> Self {
        let progress = Arc::new(Mutex::new(Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (p, c, repaint) = (progress.clone(), cancel.clone(), ctx.clone());
        let end = Background::spawn(ctx, move || {
            let mut last = Instant::now();
            let result = setup::run(&iso, &out, &c, &mut |now| {
                if let Ok(mut p) = p.lock() {
                    *p = now.clone();
                }
                if last.elapsed().as_millis() >= 100 {
                    last = Instant::now();
                    repaint.request_repaint();
                }
            });
            match result {
                Ok(summary) => SetupEnd::Done(summary),
                Err(e) if e.is::<Cancelled>() => SetupEnd::Cancelled,
                Err(e) => SetupEnd::Failed(format!("{e:#}")),
            }
        });
        Self { progress, cancel, end, started: Instant::now() }
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    pub fn end(&self) -> Option<SetupEnd> {
        self.end.take()
    }
}

const LOG_LINES: usize = 2000;

pub const GAME_LOG: &str = "game.log";
pub const VIEWER_LOG: &str = "asset-viewer.log";

pub struct Viewer {
    pub log: Arc<Mutex<Vec<String>>>,
    pub status: Arc<Mutex<Option<String>>>,
    pub pid: u32,
}

impl Viewer {
    pub fn spawn(ctx: &egui::Context, inv: &Invocation, log: &str) -> std::io::Result<Self> {
        Self::spawn_in(ctx, inv, &crate::settings::app_dir(), log)
    }

    fn spawn_in(ctx: &egui::Context, inv: &Invocation, dir: &Path, log: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(log);
        let mut file = std::fs::File::create(&path)?;
        writeln!(file, "$ {}", inv.display())?;
        let mut cmd = inv.command();
        cmd.stdin(Stdio::null()).stdout(Stdio::from(file.try_clone()?)).stderr(Stdio::from(file));
        cmd.env("NO_COLOR", "1");
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id();
        let log = Arc::new(Mutex::new(Vec::new()));
        let status = Arc::new(Mutex::new(None));
        let exited = Arc::new(AtomicBool::new(false));
        let (done, flag, repaint) = (status.clone(), exited.clone(), ctx.clone());
        std::thread::spawn(move || {
            let text = match child.wait() {
                Ok(s) if s.success() => "exited normally".to_string(),
                Ok(s) => match s.code() {
                    Some(code) => format!("exited with code {code}"),
                    None => format!("ended: {s}"),
                },
                Err(e) => format!("could not be waited for: {e}"),
            };
            flag.store(true, Ordering::Release);
            std::thread::sleep(std::time::Duration::from_millis(300));
            if let Ok(mut d) = done.lock() {
                *d = Some(text);
            }
            repaint.request_repaint();
        });
        let (lines, ctx) = (log.clone(), ctx.clone());
        std::thread::spawn(move || tail(&path, &lines, &exited, &ctx));
        Ok(Self { log, status, pid })
    }

    pub fn status(&self) -> Option<String> {
        self.status.lock().ok()?.clone()
    }
}

fn tail(path: &std::path::Path, lines: &Mutex<Vec<String>>, exited: &AtomicBool, ctx: &egui::Context) {
    let Ok(file) = std::fs::File::open(path) else { return };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    loop {
        let finished = exited.load(Ordering::Acquire);
        match reader.read_line(&mut line) {
            Ok(n) if n > 0 && line.ends_with('\n') => {
                if let Ok(mut l) = lines.lock() {
                    if l.len() >= LOG_LINES {
                        l.remove(0);
                    }
                    l.push(strip_ansi(line.trim_end()));
                }
                line.clear();
                ctx.request_repaint();
            }
            Ok(_) if !finished => std::thread::sleep(std::time::Duration::from_millis(100)),
            Ok(_) | Err(_) => {
                if !line.is_empty()
                    && let Ok(mut l) = lines.lock()
                {
                    l.push(strip_ansi(line.trim_end()));
                }
                return;
            }
        }
    }
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        if chars.clone().next() == Some('[') {
            chars.next();
            for f in chars.by_ref() {
                if ('@'..='~').contains(&f) {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod child_start {
    use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

    static STARTS: RwLock<()> = RwLock::new(());

    pub(crate) fn shared() -> RwLockReadGuard<'static, ()> {
        STARTS.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn exclusive() -> RwLockWriteGuard<'static, ()> {
        STARTS.write().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{GAME_LOG, VIEWER_LOG, Viewer, child_start};
    use crate::command::Invocation;

    #[test]
    fn strips_colours() {
        let line = "\u{1b}[2m2026-10-07\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m viewer";
        assert_eq!(super::strip_ansi(line), "2026-10-07  INFO viewer");
    }

    #[test]
    fn a_starts_output_goes_to_its_log() {
        let dir = std::env::temp_dir().join(format!("launcher-jobs-{}", std::process::id()));
        let inv = Invocation {
            program: std::env::current_exe().expect("the test binary"),
            args: vec!["--list".into()],
            env: Vec::new(),
        };
        let ctx = eframe::egui::Context::default();
        for log in [GAME_LOG, VIEWER_LOG] {
            let started = {
                let _no_lock_test = child_start::exclusive();
                Viewer::spawn_in(&ctx, &inv, &dir, log).expect("the start")
            };
            let listed =
                || started.log.lock().is_ok_and(|l| l.iter().any(|l| l.contains("a_starts_output_goes_to_its_log")));
            let since = Instant::now();
            while started.status().is_none() || !listed() {
                assert!(since.elapsed() < Duration::from_secs(60), "{log}: {:?}", started.status());
                std::thread::sleep(Duration::from_millis(50));
            }
            assert_eq!(started.status().as_deref(), Some("exited normally"));
            let text = std::fs::read_to_string(dir.join(log)).expect("the log");
            assert!(text.starts_with("$ ") && text.contains("--list"), "{text}");
        }
        assert!(!dir.join("viewer.log").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
