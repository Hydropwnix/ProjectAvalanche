use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub const INSTANCES: u32 = 10;

pub(crate) trait Transport: Send {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()>;
    fn receive(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>>;
}

pub fn endpoints() -> Vec<PathBuf> {
    endpoints_in(&|key| std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from))
}

// PORT(choice): Linux and macOS: `XDG_RUNTIME_DIR`, the Flatpak client's `XDG_RUNTIME_DIR/app/com.discordapp.Discord`
// and the Snap client's `XDG_RUNTIME_DIR/snap.discord` (where those packages' clients put the socket: known from the
// community's clients, not in Discord's documentation), then `TMPDIR`, `TMP`, `TEMP` and `/tmp` (the documented
// order); each folder once, `discord-ipc-0` … `-9` in each. Windows: `\\?\pipe\discord-ipc-0` … `-9`. The console
// had no Discord.
pub fn endpoints_in(env: &dyn Fn(&str) -> Option<PathBuf>) -> Vec<PathBuf> {
    if cfg!(windows) {
        return (0..INSTANCES).map(|n| PathBuf::from(format!(r"\\?\pipe\discord-ipc-{n}"))).collect();
    }
    let mut folders: Vec<PathBuf> = Vec::new();
    let runtime = env("XDG_RUNTIME_DIR");
    folders.extend(runtime.clone());
    if let Some(r) = &runtime {
        folders.push(r.join("app").join("com.discordapp.Discord"));
        folders.push(r.join("snap.discord"));
    }
    folders.extend(["TMPDIR", "TMP", "TEMP"].iter().filter_map(|k| env(k)));
    folders.push(PathBuf::from("/tmp"));
    let mut seen: Vec<PathBuf> = Vec::new();
    for f in folders {
        if !seen.contains(&f) {
            seen.push(f);
        }
    }
    seen.iter().flat_map(|f| (0..INSTANCES).map(move |n| f.join(format!("discord-ipc-{n}")))).collect()
}

pub(crate) fn connect(path: &Path, write_timeout: Duration) -> io::Result<Box<dyn Transport>> {
    #[cfg(unix)]
    {
        unix::connect(path, write_timeout).map(|t| Box::new(t) as Box<dyn Transport>)
    }
    #[cfg(windows)]
    {
        let _ = write_timeout;
        windows::connect(path).map(|t| Box::new(t) as Box<dyn Transport>)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (path, write_timeout);
        Err(io::Error::new(io::ErrorKind::Unsupported, "no Discord IPC on this system"))
    }
}

#[cfg(test)]
mod tests;
