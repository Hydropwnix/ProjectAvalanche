use std::fs::File;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

use super::{Seal, Spawned};

pub struct Process(Child);
pub struct Status(ExitStatus);

impl Process {
    pub fn id(&self) -> u32 {
        self.0.id()
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<Status>> {
        self.0.try_wait().map(|s| s.map(Status))
    }
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.0.kill()
    }
    pub fn wait(&mut self) -> std::io::Result<Status> {
        self.0.wait().map(Status)
    }
}

impl Status {
    pub fn by_sandbox(&self) -> bool {
        false
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub fn spawn(exe: &Path, arg: &str) -> Result<Spawned, String> {
    let mut child = Command::new(exe)
        .arg(arg)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("the movie decoder's process did not start: {e}"))?;
    let (Some(i), Some(o), Some(e)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the movie decoder's pipes".into());
    };
    let file = |fd: OwnedFd| File::from(fd);
    Ok(Spawned { process: Process(child), stdin: file(i.into()), stdout: file(o.into()), stderr: file(e.into()) })
}

pub fn prepare(_: Seal) -> Result<(), String> {
    Err("this system has no sandbox for the movie decoder (Linux on x86-64 and Windows have one)".into())
}

pub fn seal(_: Seal) -> Result<(), String> {
    Err("this system has no sandbox for the movie decoder".into())
}

pub fn measure_from_here() -> Result<(), String> {
    Err("the sandbox's list is measured on Linux".into())
}
