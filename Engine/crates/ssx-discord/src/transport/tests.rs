use super::*;

fn env_of(vars: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<PathBuf> {
    move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| PathBuf::from(v))
}

#[cfg(unix)]
#[test]
fn the_folders_in_order_each_once() {
    let paths = endpoints_in(&env_of(&[("XDG_RUNTIME_DIR", "/run/user/1000"), ("TMPDIR", "/tmp"), ("TEMP", "/var/t")]));
    let folders: Vec<&Path> = paths.iter().step_by(INSTANCES as usize).map(|p| p.parent().unwrap()).collect();
    assert_eq!(
        folders,
        [
            Path::new("/run/user/1000"),
            Path::new("/run/user/1000/app/com.discordapp.Discord"),
            Path::new("/run/user/1000/snap.discord"),
            Path::new("/tmp"),
            Path::new("/var/t"),
        ]
    );
    assert_eq!(paths.len(), 5 * INSTANCES as usize);
    assert_eq!(paths[0], Path::new("/run/user/1000/discord-ipc-0"));
    assert_eq!(paths[9], Path::new("/run/user/1000/discord-ipc-9"));
    let paths = endpoints_in(&env_of(&[]));
    assert_eq!(paths.len(), INSTANCES as usize);
    assert_eq!(paths[0], Path::new("/tmp/discord-ipc-0"));
}

#[cfg(windows)]
#[test]
fn the_pipes() {
    let paths = endpoints_in(&env_of(&[("XDG_RUNTIME_DIR", "/run/user/1000")]));
    assert_eq!(paths.len(), INSTANCES as usize);
    assert_eq!(paths[0], Path::new(r"\\?\pipe\discord-ipc-0"));
}

#[cfg(unix)]
#[test]
fn only_a_socket_is_connected_to() {
    let dir = std::env::temp_dir().join(format!("ssx-discord-transport-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("discord-ipc-0");
    std::fs::write(&file, b"not a socket").unwrap();
    let e = connect(&file, Duration::from_secs(1)).err().unwrap();
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    let sock = dir.join("real");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let link = dir.join("discord-ipc-1");
    std::os::unix::fs::symlink(&sock, &link).unwrap();
    assert_eq!(connect(&link, Duration::from_secs(1)).err().unwrap().kind(), io::ErrorKind::InvalidInput);
    assert!(connect(&sock, Duration::from_secs(1)).is_ok());
    assert_eq!(connect(&dir.join("missing"), Duration::from_secs(1)).err().unwrap().kind(), io::ErrorKind::NotFound);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[cfg(unix)]
#[test]
fn receiving_never_blocks_and_sees_the_close() {
    let (a, mut b) = std::os::unix::net::UnixStream::pair().unwrap();
    let mut t = unix::Socket::from_stream(a);
    let mut buf = [0u8; 16];
    let started = std::time::Instant::now();
    assert_eq!(t.receive(&mut buf).unwrap(), None);
    assert!(started.elapsed() < Duration::from_millis(50));
    use std::io::Write;
    b.write_all(b"abc").unwrap();
    assert_eq!(t.receive(&mut buf).unwrap(), Some(3));
    drop(b);
    assert_eq!(t.receive(&mut buf).unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
}
