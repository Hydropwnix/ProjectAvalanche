use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED};
use windows_sys::Win32::System::Pipes::PeekNamedPipe;

use super::Transport;

pub(super) struct Pipe(File);

pub(super) fn connect(path: &Path) -> io::Result<Pipe> {
    OpenOptions::new().read(true).write(true).open(path).map(Pipe)
}

impl Transport for Pipe {
    // PORT(choice): the pipe is opened for synchronous I/O, as Discord's own discord-rpc library opens it, so a write
    // has no timeout: a Discord client that stops reading holds the presence's thread (never the game's) until it
    // reads or closes. The console had no Discord.
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        let mut waiting: u32 = 0;
        let ok = unsafe {
            PeekNamedPipe(
                self.0.as_raw_handle(),
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut waiting,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let e = io::Error::last_os_error();
            let closed = [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED].map(|c| c as i32);
            return Err(if e.raw_os_error().is_some_and(|c| closed.contains(&c)) {
                io::Error::new(io::ErrorKind::UnexpectedEof, "Discord closed the connection")
            } else {
                e
            });
        }
        if waiting == 0 {
            return Ok(None);
        }
        let take = buf.len().min(waiting as usize);
        match self.0.read(&mut buf[..take])? {
            0 => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Discord closed the connection")),
            n => Ok(Some(n)),
        }
    }
}
