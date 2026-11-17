use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::Transport;

pub(super) struct Socket(UnixStream);

fn user() -> u32 {
    unsafe { libc::geteuid() }
}

// PORT(choice): only a socket owned by this process's user is connected to — `/tmp` is shared with other users, who
// could put a `discord-ipc-0` there; in the sticky `/tmp` no other user can replace an entry of ours between the
// check and the connect. A link is not followed. The console had no Discord.
pub(super) fn connect(path: &Path, write_timeout: Duration) -> io::Result<Socket> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_socket() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a socket"));
    }
    if meta.uid() != user() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the socket belongs to another user"));
    }
    let stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(write_timeout))?;
    Ok(Socket(stream))
}

impl Transport for Socket {
    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.write_all(bytes)
    }

    fn receive(&mut self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        let n = unsafe { libc::recv(self.0.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
        match n {
            0 => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Discord closed the connection")),
            n if n > 0 => Ok(Some(n as usize)),
            _ => {
                let e = io::Error::last_os_error();
                match e.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => Ok(None),
                    _ => Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
impl Socket {
    pub(super) fn from_stream(stream: UnixStream) -> Self {
        Socket(stream)
    }
}
