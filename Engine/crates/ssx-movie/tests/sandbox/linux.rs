use std::net::TcpListener;
use std::path::Path;

use ssx_movie::isolate::MEMORY_LIMIT;

use super::{PROGRAM, Record, TINY_PICTURES, attempt, expect_end, finish, running};

fn written() -> std::path::PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("ssx-movie-sandbox-write-attempt")
}

fn field<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.lines().find_map(|l| l.strip_prefix(key)).map(str::trim)
}

fn limit<'a>(limits: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
    let rest = limits.lines().find_map(|l| l.strip_prefix(name))?;
    let mut v = rest.split_whitespace();
    Some((v.next()?, v.next()?))
}

fn outside(rec: &mut Record) {
    let own = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let inherited: u32 = field(&own, "Seccomp_filters:").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), 0) } != 0 {
        rec.fail(format!("a pipe: {}", std::io::Error::last_os_error()));
        return;
    }
    let spawned = running(Path::new(PROGRAM));
    unsafe { libc::close(fds[1]) };
    let (mut iso, rest, pictures) = match spawned {
        Ok(r) => r,
        Err(e) => {
            rec.fail(format!("the decoder's program: {e}"));
            unsafe { libc::close(fds[0]) };
            return;
        }
    };
    let pid = iso.id();
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
    let sealed = inherited + 1;
    for (key, want) in
        [("Seccomp:", "2".to_string()), ("NoNewPrivs:", "1".into()), ("Seccomp_filters:", sealed.to_string())]
    {
        let v = field(&status, key).unwrap_or("-");
        rec.check(format!("/proc/{pid}/status {key} {v}"), v == want);
    }
    let tasks: Vec<String> = std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let filtered = tasks
        .iter()
        .filter(|t| {
            let s = std::fs::read_to_string(format!("/proc/{pid}/task/{t}/status")).unwrap_or_default();
            field(&s, "Seccomp:") == Some("2")
                && field(&s, "Seccomp_filters:").and_then(|v| v.parse::<u32>().ok()) == Some(sealed)
        })
        .count();
    let threads = usize::from(super::tiny(true).0.threads);
    rec.check(
        format!("{filtered} of its {} threads (rav1d's {threads} and its own) filtered", tasks.len()),
        tasks.len() > threads && filtered == tasks.len(),
    );
    let limits = std::fs::read_to_string(format!("/proc/{pid}/limits")).unwrap_or_default();
    let data = MEMORY_LIMIT.to_string();
    for (name, want) in [
        ("Max data size", data.as_str()),
        ("Max processes", "0"),
        ("Max open files", "3"),
        ("Max file size", "0"),
        ("Max core file size", "0"),
    ] {
        let v = limit(&limits, name);
        rec.check(format!("{name}: {v:?}"), v == Some((want, want)));
    }
    for entry in ["fd", "environ", "cwd"] {
        let e = std::fs::read_dir(format!("/proc/{pid}/{entry}")).map(|_| ()).or_else(|e| {
            if e.kind() == std::io::ErrorKind::NotADirectory {
                std::fs::read(format!("/proc/{pid}/{entry}")).map(|_| ())
            } else {
                Err(e)
            }
        });
        let denied = matches!(&e, Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied);
        rec.check(format!("/proc/{pid}/{entry} closed to its own user (not dumpable): {e:?}"), denied);
    }
    let mut p = libc::pollfd { fd: fds[0], events: libc::POLLIN, revents: 0 };
    let r = unsafe { libc::poll(&mut p, 1, 0) };
    let alive = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .is_ok_and(|s| field(&s, "State:").is_some_and(|v| !v.starts_with('Z')));
    rec.check(
        format!("its inherited descriptor closed while it runs (poll {r}, revents 0x{:x}, alive {alive})", p.revents),
        r == 1 && p.revents & libc::POLLHUP != 0 && alive,
    );
    unsafe { libc::close(fds[0]) };
    match finish(&mut iso, rest, pictures) {
        Ok(n) => rec.check(format!("the test card decoded in the sandbox: {n} pictures"), n == TINY_PICTURES),
        Err(e) => rec.fail(format!("the test card decoded in the sandbox: {e}")),
    }
}

pub fn run(rec: &mut Record) {
    outside(rec);
    let parent = std::process::id().to_string();
    let path = written();
    let _ = std::fs::remove_file(&path);
    let listener = TcpListener::bind("127.0.0.1:0").and_then(|l| l.set_nonblocking(true).map(|()| l));
    let Ok(listener) = listener else {
        rec.fail("no listener on 127.0.0.1".into());
        return;
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0).to_string();
    let path_text = path.to_string_lossy().into_owned();
    let sandbox = "stopped by its sandbox";
    let cases: [(&str, &str, &[&str]); 17] = [
        ("open-read", "", &["refused system call 257 (openat)"]),
        ("open-write", &path_text, &["refused system call 257 (openat)"]),
        ("socket", "", &["refused system call 41 (socket)"]),
        ("connect", &port, &["refused system call 42 (connect)"]),
        ("execve", "", &["refused system call 59 (execve)"]),
        ("clone", "", &["refused system call 56 (clone)"]),
        ("clone3", "", &["refused system call 435 (clone3)"]),
        ("ptrace", &parent, &["refused system call 101 (ptrace)"]),
        ("kill", &parent, &["refused system call 62 (kill)"]),
        ("tgkill", &parent, &["refused system call 234 (tgkill)"]),
        ("mmap-exec", "", &["refused system call 9 (mmap)", " 0x5 "]),
        ("mprotect-exec", "", &["refused system call 10 (mprotect)", " 0x5 "]),
        ("write-fd3", "", &["refused system call 1 (write), arguments 0x3 "]),
        ("x32", "", &["refused system call 1073741863,"]),
        ("memory", "", &["SIGABRT", "memory allocation of 8388608 bytes failed", "MiB allocated before the limit"]),
        ("environment", "", &["exit status: 0", "environment: 0 variables"]),
        ("glibc-fatal", "", &["SIGABRT", "double free"]),
    ];
    for (name, arg, said) in cases {
        let mut want = said.to_vec();
        match name {
            "memory" | "environment" | "glibc-fatal" => expect_end(rec, name, arg, &want, &[sandbox]),
            _ => {
                want.extend([sandbox, "SIGSYS"]);
                expect_end(rec, name, arg, &want, &[]);
            }
        }
    }
    rec.check(format!("{} not made", path.display()), !path.exists());
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            rec.check(format!("the listener on 127.0.0.1:{port} not reached"), true);
        }
        other => rec.fail(format!("the listener on 127.0.0.1:{port} was reached: {other:?}")),
    }
    let mut refused = |name: &str, line: &[&str]| match attempt(name, "") {
        Ok(r) if !r.contains(sandbox) || !r.contains("SIGSYS") || r.contains("NOT STOPPED") => {
            rec.fail(format!("{name}: {r}"));
        }
        Ok(r) if !r.contains("refused system call") => {
            rec.stopped(&format!("{name} (no line: refused with every signal blocked): {r}"));
        }
        Ok(r) if line.iter().all(|w| r.contains(w)) => rec.stopped(&format!("{name}: {r}")),
        Ok(r) => rec.fail(format!("{name}: {r} (a line must name {line:?})")),
        Err(e) => rec.fail(format!("{name}: {e}")),
    };
    refused("fork", &["refused system call 56 (clone)"]);
    refused("thread", &["refused system call 28 (madvise)", " 0x66 "]);
    match attempt("int80", "") {
        Ok(r) if r.contains(sandbox) && !r.contains("refused system call") => {
            rec.stopped(&format!("int80 (killed by the architecture check): {r}"));
        }
        Ok(r) if r.contains("SIGSEGV") => rec.stopped(&format!("int80 (no 32-bit entry in this kernel): {r}")),
        Ok(r) => rec.fail(format!("int80: {r}")),
        Err(e) => rec.fail(format!("int80: {e}")),
    }
}
