use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use eframe::egui;
use ssx_assets::display::{DisplayList, MAX_LIST_BYTES};

use crate::jobs::Background;

pub const QUERY_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_ERROR: usize = 2048;

#[derive(Default)]
pub struct Displays {
    pub list: Option<Result<DisplayList, String>>,
    job: Option<Background<Result<DisplayList, String>>>,
    pub synced: bool,
}

impl Displays {
    pub fn ask(&mut self, ctx: &egui::Context, again: bool) {
        if self.job.is_none() && (again || self.list.is_none()) {
            let program = crate::command::game_exe();
            self.job = Some(Background::spawn(ctx, move || list_from(&program)));
        }
    }

    pub fn asking(&self) -> bool {
        self.job.is_some()
    }

    pub fn poll(&mut self) {
        if let Some(answer) = self.job.as_ref().and_then(Background::take) {
            self.job = None;
            self.list = Some(answer);
        }
    }
}

pub fn list_from(program: &std::path::Path) -> Result<DisplayList, String> {
    if !program.is_file() {
        return Err(format!("the game is missing: {}", program.display()));
    }
    let mut cmd = Command::new(program);
    cmd.arg("--list-displays");
    cmd.env("NO_COLOR", "1");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let what = program.display().to_string();
    let (status, out, err) = run_bounded(cmd, &what, QUERY_TIMEOUT)?;
    if !status.success() {
        let text = String::from_utf8_lossy(&err);
        return Err(format!(
            "the game could not list the monitors ({status}): {}",
            crate::ui::widgets::shown(text.trim())
        ));
    }
    let text = std::str::from_utf8(&out).map_err(|_| "the game's list is not UTF-8".to_string())?;
    DisplayList::parse(text).map_err(|e| format!("the game's list does not read: {e}"))
}

fn run_bounded(
    mut cmd: Command,
    what: &str,
    timeout: Duration,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("could not start {what}: {e}"))?;
    let read = |pipe: Option<Box<dyn Read + Send>>, limit: usize| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(p) = pipe {
                let _ = p.take(limit as u64 + 1).read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), MAX_LIST_BYTES);
    let err = read(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>), MAX_ERROR);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{what} did not answer within {:.1} s", timeout.as_secs_f32()));
            }
            Err(e) => return Err(format!("waiting for {what}: {e}")),
        }
    };
    let left = || deadline.saturating_duration_since(Instant::now()).max(Duration::from_millis(100));
    let out = out.recv_timeout(left()).map_err(|_| format!("{what}'s output did not end"))?;
    let mut err = err.recv_timeout(left()).unwrap_or_default();
    err.truncate(MAX_ERROR);
    Ok((status, out, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_program_that_does_not_list_gives_an_error() {
        let _no_lock_test = crate::jobs::child_start::exclusive();
        assert!(list_from(std::path::Path::new("/nonexistent/avalanche-game")).unwrap_err().contains("missing"));
        #[cfg(unix)]
        {
            let e = list_from(std::path::Path::new("/bin/false")).unwrap_err();
            assert!(e.contains("could not list"), "{e}");
            let e = list_from(std::path::Path::new("/bin/true")).unwrap_err();
            assert!(e.contains("does not read"), "{e}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn every_wait_on_the_game_is_bounded() {
        let _no_lock_test = crate::jobs::child_start::exclusive();
        let t = Instant::now();
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "sleep 30"]);
        let e = run_bounded(cmd, "sh", Duration::from_millis(300)).unwrap_err();
        assert!(e.contains("did not answer within 0.3 s"), "{e}");
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "(sleep 30 &) ; echo started"]);
        let e = run_bounded(cmd, "sh", Duration::from_millis(500)).unwrap_err();
        assert!(e.contains("output did not end"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(5), "{:?}", t.elapsed());
    }
}
