use std::time::{Duration, Instant};

use ssx_content::movie::av1::OBU_TEMPORAL_DELIMITER;
use ssx_movie::av1::{Expect, Settings};
use ssx_movie::isolate::{self, Isolated, Out};
use ssx_movie::isolate_os::Seal;
use ssx_movie::yuv::{Matrix, Siting};

const ATTEMPT: &[u8] = b"SSX-SANDBOX-ATTEMPT\0";
const NOT_STOPPED: i32 = 97;

const TINY: &[u8] = &[
    0x12, 0x00, 0x0a, 0x0d, 0x00, 0x00, 0x00, 0x02, 0xaf, 0xff, 0x9b, 0x5f, 0x22, 0x04, 0x04, 0x02, 0x08, 0x32, 0xbd,
    0x01, 0x14, 0x00, 0x3d, 0xe0, 0x34, 0xd4, 0xa1, 0x8b, 0x2e, 0xba, 0xd9, 0x6e, 0x95, 0xb5, 0x07, 0x57, 0x93, 0x16,
    0x1d, 0xfd, 0x8d, 0x0f, 0x81, 0xb9, 0xb9, 0xd9, 0xe3, 0x3b, 0xd2, 0xb9, 0x66, 0xbc, 0x24, 0xdc, 0x38, 0xf1, 0xe7,
    0x5a, 0x5e, 0x3d, 0x05, 0xf9, 0x51, 0x4e, 0x34, 0xe9, 0x76, 0x5e, 0x41, 0x4e, 0xd3, 0xf9, 0xe2, 0xb7, 0x67, 0x1b,
    0x65, 0x3d, 0xe2, 0x4b, 0x73, 0xb6, 0xf9, 0x3e, 0x4c, 0x15, 0x64, 0x12, 0xbb, 0x64, 0x9c, 0xfe, 0xe0, 0xc9, 0xa4,
    0x4d, 0xbe, 0x61, 0xf1, 0x76, 0x37, 0x23, 0x98, 0x79, 0xac, 0x6f, 0x31, 0xcd, 0xdf, 0x81, 0x62, 0x19, 0x98, 0x88,
    0x52, 0x2e, 0xe2, 0x13, 0xf0, 0x1a, 0x2f, 0xc8, 0x78, 0x1c, 0x82, 0xde, 0x10, 0xcb, 0x71, 0xf9, 0xaf, 0x7e, 0xc1,
    0x7a, 0x76, 0x6c, 0x33, 0xa4, 0x5e, 0xda, 0x11, 0x18, 0x74, 0x6d, 0x97, 0xee, 0xac, 0xdd, 0xd4, 0x55, 0xb8, 0xbf,
    0x40, 0x45, 0x6c, 0x9e, 0x8e, 0xd3, 0xf7, 0x24, 0xe9, 0x19, 0xfa, 0x66, 0xb6, 0xf9, 0x86, 0x20, 0x38, 0xfa, 0x9e,
    0xe9, 0x3e, 0xb8, 0x17, 0x43, 0x92, 0x02, 0xd1, 0x6d, 0xe0, 0x6c, 0x85, 0x88, 0x77, 0xbd, 0xbb, 0x55, 0x3c, 0x46,
    0x1c, 0x90, 0x4c, 0xd0, 0xb9, 0x7e, 0x46, 0x80, 0xfd, 0x8c, 0xfd, 0x62, 0xf6, 0xd3, 0x84, 0x99, 0xac, 0xe5, 0x88,
    0x12, 0x00, 0x32, 0x11, 0x32, 0x01, 0xe0, 0x40, 0x00, 0x00, 0x03, 0x7e, 0xc0, 0x00, 0x01, 0x60, 0x00, 0x10, 0x00,
    0x99, 0x10, 0x12, 0x00, 0x32, 0x11, 0x32, 0x02, 0xe0, 0x82, 0x00, 0x00, 0x03, 0x7f, 0xc0, 0x00, 0x01, 0x60, 0x00,
    0x10, 0x00, 0x9c, 0xf0,
];
const TINY_PICTURES: usize = 3;

fn main() {
    if isolate::is_child() {
        child();
    }
    let mut args = libtest_mimic::Arguments::from_args();
    args.test_threads = Some(1);
    let trial = libtest_mimic::Trial::test("sandbox", || {
        let mut failed = Vec::new();
        platform::run(&mut failed);
        if failed.is_empty() {
            println!("sandbox: every check passed");
            Ok(())
        } else {
            Err(failed.iter().map(|f| format!("FAILED: {f}")).collect::<Vec<_>>().join("\n").into())
        }
    });
    libtest_mimic::run(&args, vec![trial]).exit();
}

fn child() -> ! {
    let mut s = match isolate::enter_child(Seal::Enforce) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1)
        }
    };
    let code = match s.read_unit() {
        Ok(Some((_, unit))) if unit.starts_with(ATTEMPT) => {
            let rest = &unit[ATTEMPT.len()..];
            let (name, arg) =
                rest.iter().position(|&b| b == 0).map_or((rest, &[][..]), |k| (&rest[..k], &rest[k + 1..]));
            platform::attempt(&String::from_utf8_lossy(name), &String::from_utf8_lossy(arg))
        }
        Ok(first) => match s.serve_from(first) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{e}");
                1
            }
        },
        Err(e) => {
            eprintln!("{e}");
            1
        }
    };
    std::process::exit(code)
}

fn not_stopped(what: &str) -> i32 {
    eprintln!("NOT STOPPED: {what}");
    NOT_STOPPED
}

fn tiny() -> (Settings, Expect) {
    let expect =
        Expect { width: 64, height: 64, mono: false, matrix: Matrix::Bt709, full_range: false, siting: Siting::Centre };
    (Settings { threads: 4, frame_delay: 0, max_pixels: 64 * 64 }, expect)
}

fn tiny_units() -> Vec<Vec<u8>> {
    let obus = ssx_content::movie::av1::obus(TINY, 64).unwrap_or_default();
    let mut cuts: Vec<usize> = obus.iter().filter(|o| o.kind == OBU_TEMPORAL_DELIMITER).map(|o| o.start).collect();
    cuts.push(TINY.len());
    cuts.windows(2).map(|w| TINY[w[0]..w[1]].to_vec()).collect()
}

fn attempt(name: &str, arg: &str) -> Result<String, String> {
    let (settings, expect) = tiny();
    let mut iso = Isolated::spawn(&settings, &expect)?;
    let mut unit = ATTEMPT.to_vec();
    unit.extend(name.as_bytes());
    unit.push(0);
    unit.extend(arg.as_bytes());
    if iso.push(0, unit)?.is_some() {
        return Err("the child's queue is full before its first unit".into());
    }
    iso.finish();
    let start = Instant::now();
    loop {
        match iso.next(Duration::from_millis(100)) {
            Err(reason) => return Ok(reason),
            Ok(Some(Out::Picture(..))) => return Err("the child gave a picture".into()),
            Ok(Some(Out::Done)) => return Err("the child ended its stream normally".into()),
            Ok(None) if start.elapsed() > Duration::from_secs(30) => return Err("no end within 30 s".into()),
            Ok(None) => {}
        }
    }
}

fn expect_end(failed: &mut Vec<String>, name: &str, arg: &str, want: &[&str], not: &[&str]) {
    match attempt(name, arg) {
        Ok(reason) => {
            let missing: Vec<_> = want.iter().filter(|w| !reason.contains(*w)).collect();
            let present: Vec<_> = not.iter().filter(|w| reason.contains(*w)).collect();
            if missing.is_empty() && present.is_empty() && !reason.contains("NOT STOPPED") {
                println!("stopped  {name}: {reason}");
            } else {
                failed.push(format!("{name}: {reason} (wanted {want:?}, not {not:?})"));
            }
        }
        Err(e) => failed.push(format!("{name}: {e}")),
    }
}

fn decode(iso: &mut Isolated) -> Result<usize, String> {
    for (k, u) in tiny_units().into_iter().enumerate() {
        if iso.push(k as i64, u)?.is_some() {
            return Err("the child's queue is full".into());
        }
    }
    iso.finish();
    let (mut pictures, start) = (0, Instant::now());
    loop {
        match iso.next(Duration::from_millis(100))? {
            Some(Out::Picture(..)) => pictures += 1,
            Some(Out::Done) => return Ok(pictures),
            None if start.elapsed() > Duration::from_secs(30) => return Err("no end within 30 s".into()),
            None => {}
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod platform {
    use std::ffi::CString;
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    use ssx_movie::isolate::{Isolated, MEMORY_LIMIT};

    use super::{NOT_STOPPED, TINY_PICTURES, expect_end, not_stopped, tiny};

    fn written() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("ssx-movie-sandbox-write-attempt")
    }

    pub fn attempt(name: &str, arg: &str) -> i32 {
        let parent: libc::pid_t = arg.parse().unwrap_or(0);
        unsafe {
            match name {
                "open-read" => {
                    let fd = libc::open(c"/etc/passwd".as_ptr(), libc::O_RDONLY);
                    not_stopped(&format!("open /etc/passwd for reading: {fd}"))
                }
                "open-write" => {
                    let p = CString::new(arg).unwrap_or_default();
                    let fd = libc::open(p.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o600);
                    not_stopped(&format!("open {arg} for writing: {fd}"))
                }
                "socket" => {
                    let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
                    not_stopped(&format!("socket: {fd}"))
                }
                "connect" => {
                    let port: u16 = arg.parse().unwrap_or(0);
                    let mut a: libc::sockaddr_in = std::mem::zeroed();
                    a.sin_family = libc::AF_INET as libc::sa_family_t;
                    a.sin_port = port.to_be();
                    a.sin_addr.s_addr = u32::from_be_bytes([127, 0, 0, 1]).to_be();
                    let size = size_of::<libc::sockaddr_in>() as libc::socklen_t;
                    let r = libc::connect(0, (&raw const a).cast(), size);
                    not_stopped(&format!("connect to 127.0.0.1:{port}: {r}"))
                }
                "execve" => {
                    let argv = [c"sh".as_ptr(), c"-c".as_ptr(), c"true".as_ptr(), std::ptr::null()];
                    let envp = [std::ptr::null()];
                    let r = libc::execve(c"/bin/sh".as_ptr(), argv.as_ptr(), envp.as_ptr());
                    not_stopped(&format!("execve /bin/sh: {r}"))
                }
                "clone" => {
                    let r = libc::syscall(libc::SYS_clone, libc::SIGCHLD, 0, 0, 0, 0);
                    if r == 0 {
                        libc::_exit(0);
                    }
                    not_stopped(&format!("clone: {r}"))
                }
                "fork" => {
                    let pid = libc::fork();
                    if pid == 0 {
                        libc::_exit(0);
                    }
                    not_stopped(&format!("fork: {pid}"))
                }
                "clone3" => {
                    let mut args = [0u64; 11];
                    args[4] = libc::SIGCHLD as u64;
                    let r = libc::syscall(libc::SYS_clone3, args.as_mut_ptr(), size_of_val(&args));
                    if r == 0 {
                        libc::_exit(0);
                    }
                    not_stopped(&format!("clone3: {r}"))
                }
                "thread" => match std::thread::Builder::new().spawn(|| {}) {
                    Ok(_) => not_stopped("a new thread"),
                    Err(e) => not_stopped(&format!("a new thread refused by an error only: {e}")),
                },
                "ptrace" => {
                    let r = libc::ptrace(libc::PTRACE_ATTACH, parent, 0, 0);
                    not_stopped(&format!("ptrace attach to the parent {parent}: {r}"))
                }
                "kill" => {
                    let r = libc::kill(parent, libc::SIGKILL);
                    not_stopped(&format!("kill of the parent {parent}: {r}"))
                }
                "tgkill" => {
                    let r = libc::syscall(libc::SYS_tgkill, parent, parent, libc::SIGKILL);
                    not_stopped(&format!("tgkill of the parent {parent}: {r}"))
                }
                "mmap-exec" => {
                    let p = libc::mmap(
                        std::ptr::null_mut(),
                        4096,
                        libc::PROT_READ | libc::PROT_EXEC,
                        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                        -1,
                        0,
                    );
                    not_stopped(&format!("executable memory: {p:?}"))
                }
                "mprotect-exec" => {
                    let page = Box::new([0u8; 8192]);
                    let at = (page.as_ptr() as usize).next_multiple_of(4096);
                    let r = libc::mprotect(at as *mut libc::c_void, 4096, libc::PROT_READ | libc::PROT_EXEC);
                    not_stopped(&format!("heap memory made executable: {r}"))
                }
                "write-fd3" => {
                    let r = libc::write(3, b"x".as_ptr().cast(), 1);
                    not_stopped(&format!("a write to descriptor 3: {r}"))
                }
                "x32" => {
                    let r = libc::syscall(0x4000_0000 | libc::SYS_getpid);
                    not_stopped(&format!("an x32 system call: {r}"))
                }
                "int80" => {
                    let mut eax: u32 = 20;
                    std::arch::asm!("int 0x80", inout("eax") eax);
                    not_stopped(&format!("a 32-bit system call: {eax}"))
                }
                "memory" => {
                    const CHUNK: usize = 8 << 20;
                    let mut kept = Vec::new();
                    while kept.len() < 8192 {
                        let mut v = Vec::<u8>::new();
                        if v.try_reserve_exact(CHUNK).is_err() {
                            break;
                        }
                        v.resize(CHUNK, 1);
                        kept.push(v);
                    }
                    eprintln!("{} MiB allocated before the limit", kept.len() * 8);
                    let more = vec![1u8; CHUNK];
                    kept.push(more);
                    not_stopped(&format!("{} MiB allocated", kept.len() * 8))
                }
                "environment" => {
                    eprintln!("environment: {} variables", std::env::vars_os().count());
                    0
                }
                "glibc-fatal" => {
                    let p = libc::malloc(64);
                    libc::free(p);
                    libc::free(std::hint::black_box(p));
                    not_stopped("a double free")
                }
                _ => {
                    eprintln!("no attempt {name}");
                    NOT_STOPPED
                }
            }
        }
    }

    fn field<'a>(status: &'a str, key: &str) -> Option<&'a str> {
        status.lines().find_map(|l| l.strip_prefix(key)).map(str::trim)
    }

    fn limit<'a>(limits: &'a str, name: &str) -> Option<(&'a str, &'a str)> {
        let rest = limits.lines().find_map(|l| l.strip_prefix(name))?;
        let mut v = rest.split_whitespace();
        Some((v.next()?, v.next()?))
    }

    fn outside(failed: &mut Vec<String>) {
        let own = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let inherited: u32 = field(&own, "Seccomp_filters:").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut fds = [0 as libc::c_int; 2];
        if unsafe { libc::pipe2(fds.as_mut_ptr(), 0) } != 0 {
            failed.push(format!("a pipe: {}", std::io::Error::last_os_error()));
            return;
        }
        let (settings, expect) = tiny();
        let spawned = Isolated::spawn(&settings, &expect);
        unsafe { libc::close(fds[1]) };
        let mut iso = match spawned {
            Ok(i) => i,
            Err(e) => {
                failed.push(format!("the child: {e}"));
                return;
            }
        };
        let pid = iso.id();
        let sealed = inherited + 1;
        let start = Instant::now();
        let status = loop {
            let s = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
            if field(&s, "Seccomp_filters:").and_then(|v| v.parse::<u32>().ok()) == Some(sealed) {
                break s;
            }
            if start.elapsed() > Duration::from_secs(10) {
                failed.push(format!("the child {pid} was not sealed within 10 s: {s}"));
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut check = |what: String, ok: bool| {
            println!("{}  {what}", if ok { "holds   " } else { "FAILS   " });
            if !ok {
                failed.push(what);
            }
        };
        for (key, want) in [("Seccomp:", "2"), ("NoNewPrivs:", "1")] {
            let v = field(&status, key).unwrap_or("-");
            check(format!("/proc/{pid}/status {key} {v}"), v == want);
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
        check(
            format!("{} of its {} threads (rav1d's {} and its own) filtered", filtered, tasks.len(), settings.threads),
            tasks.len() > usize::from(settings.threads) && filtered == tasks.len(),
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
            check(format!("{name}: {v:?}"), v == Some((want, want)));
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
            check(format!("/proc/{pid}/{entry} closed to its own user (not dumpable): {e:?}"), denied);
        }
        let mut p = libc::pollfd { fd: fds[0], events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut p, 1, 0) };
        let alive = std::fs::read_to_string(format!("/proc/{pid}/status"))
            .is_ok_and(|s| field(&s, "State:").is_some_and(|v| !v.starts_with('Z')));
        check(
            format!(
                "its inherited descriptor closed while it runs (poll {r}, revents 0x{:x}, alive {alive})",
                p.revents
            ),
            r == 1 && p.revents & libc::POLLHUP != 0 && alive,
        );
        unsafe { libc::close(fds[0]) };
        match super::decode(&mut iso) {
            Ok(n) => check(format!("the test card decoded in the sandbox: {n} pictures"), n == TINY_PICTURES),
            Err(e) => check(format!("the test card decoded in the sandbox: {e}"), false),
        }
    }

    pub fn run(failed: &mut Vec<String>) {
        outside(failed);
        let parent = std::process::id().to_string();
        let path = written();
        let _ = std::fs::remove_file(&path);
        let listener = TcpListener::bind("127.0.0.1:0").and_then(|l| l.set_nonblocking(true).map(|()| l));
        let Ok(listener) = listener else {
            failed.push("no listener on 127.0.0.1".into());
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
                "memory" | "environment" | "glibc-fatal" => expect_end(failed, name, arg, &want, &[sandbox]),
                _ => {
                    want.extend([sandbox, "SIGSYS"]);
                    expect_end(failed, name, arg, &want, &[]);
                }
            }
        }
        if path.exists() {
            failed.push(format!("{} was made", path.display()));
        }
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => failed.push(format!("the listener on 127.0.0.1:{port} was reached: {other:?}")),
        }
        let mut refused = |name: &str, line: &[&str]| match super::attempt(name, "") {
            Ok(r) if !r.contains(sandbox) || !r.contains("SIGSYS") || r.contains("NOT STOPPED") => {
                failed.push(format!("{name}: {r}"));
            }
            Ok(r) if !r.contains("refused system call") => {
                println!("stopped  {name} (no line: refused with every signal blocked): {r}");
            }
            Ok(r) if line.iter().all(|w| r.contains(w)) => println!("stopped  {name}: {r}"),
            Ok(r) => failed.push(format!("{name}: {r} (a line must name {line:?})")),
            Err(e) => failed.push(format!("{name}: {e}")),
        };
        refused("fork", &["refused system call 56 (clone)"]);
        refused("thread", &["refused system call 28 (madvise)", " 0x66 "]);
        match super::attempt("int80", "") {
            Ok(r) if r.contains(sandbox) && !r.contains("refused system call") => {
                println!("stopped  int80 (killed by the architecture check): {r}");
            }
            Ok(r) if r.contains("SIGSEGV") => println!("stopped  int80 (no 32-bit entry in this kernel): {r}"),
            Ok(r) => failed.push(format!("int80: {r}")),
            Err(e) => failed.push(format!("int80: {e}")),
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::os::windows::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    use ssx_movie::isolate::Isolated;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_GROUPS,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenCapabilities, TokenIntegrityLevel, TokenIsAppContainer,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, WriteFile,
    };
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualProtect,
    };
    use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
    use windows_sys::Win32::System::Threading::{
        GetProcessMitigationPolicy, OpenProcess, OpenProcessToken, PROCESS_MITIGATION_POLICY,
        PROCESS_QUERY_INFORMATION, PROCESS_TERMINATE, PROCESS_VM_READ, ProcessChildProcessPolicy,
        ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy, ProcessFontDisablePolicy, ProcessImageLoadPolicy,
        ProcessStrictHandleCheckPolicy, ProcessSystemCallDisablePolicy,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN};

    use super::{NOT_STOPPED, TINY_PICTURES, expect_end, not_stopped, tiny};

    const REFUSED: i32 = 98;

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn refused(what: &str) -> i32 {
        let e = unsafe { GetLastError() };
        eprintln!("refused: {what}: error {e}");
        REFUSED
    }

    pub fn attempt(name: &str, arg: &str) -> i32 {
        unsafe {
            match name {
                "open-read" | "open-write" => {
                    let path = wide(arg);
                    let (access, how) =
                        if name == "open-read" { (GENERIC_READ, OPEN_EXISTING) } else { (GENERIC_WRITE, CREATE_NEW) };
                    let h = CreateFileW(
                        path.as_ptr(),
                        access,
                        0,
                        std::ptr::null(),
                        how,
                        FILE_ATTRIBUTE_NORMAL,
                        std::ptr::null_mut(),
                    );
                    if h == INVALID_HANDLE_VALUE {
                        refused(&format!("{name} {arg}"))
                    } else {
                        not_stopped(&format!("{name} {arg}"))
                    }
                }
                "connect" => {
                    let to: SocketAddr = arg.parse().unwrap_or(SocketAddr::from(([127, 0, 0, 1], 0)));
                    match TcpStream::connect_timeout(&to, Duration::from_secs(5)) {
                        Ok(_) => not_stopped(&format!("connect to {to}")),
                        Err(e) => {
                            eprintln!("refused: connect to {to}: {e}");
                            REFUSED
                        }
                    }
                }
                "create-process" => match std::process::Command::new(arg).arg("/c").arg("exit").spawn() {
                    Ok(_) => not_stopped(&format!("a process {arg}")),
                    Err(e) => {
                        eprintln!("refused: a process {arg}: {e}");
                        REFUSED
                    }
                },
                "open-parent" => {
                    let pid: u32 = arg.parse().unwrap_or(0);
                    let h = OpenProcess(PROCESS_TERMINATE | PROCESS_VM_READ, 0, pid);
                    if h.is_null() {
                        refused(&format!("the parent {pid} opened"))
                    } else {
                        not_stopped(&format!("the parent {pid} opened"))
                    }
                }
                "dynamic-code" => {
                    let p = VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE);
                    if p.is_null() { refused("executable memory") } else { not_stopped("executable memory") }
                }
                "protect-exec" => {
                    let p = VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, 0x04);
                    let mut old = 0u32;
                    if p.is_null() {
                        return not_stopped("no page to try");
                    }
                    if VirtualProtect(p, 4096, PAGE_EXECUTE_READ, &mut old) == 0 {
                        refused("memory made executable")
                    } else {
                        not_stopped("memory made executable")
                    }
                }
                "handle" => {
                    let h = arg.parse::<usize>().unwrap_or(0) as HANDLE;
                    let mut n = 0u32;
                    if WriteFile(h, b"x".as_ptr(), 1, &mut n, std::ptr::null_mut()) == 0 {
                        refused("the parent's other handle")
                    } else {
                        not_stopped("the parent's other handle written")
                    }
                }
                "memory" => {
                    const CHUNK: usize = 8 << 20;
                    let mut kept = Vec::new();
                    while kept.len() < 8192 {
                        let mut v = Vec::<u8>::new();
                        if v.try_reserve_exact(CHUNK).is_err() {
                            break;
                        }
                        v.resize(CHUNK, 1);
                        kept.push(v);
                    }
                    eprintln!("{} MiB allocated before the limit", kept.len() * 8);
                    let more = vec![1u8; CHUNK];
                    kept.push(more);
                    not_stopped(&format!("{} MiB allocated", kept.len() * 8))
                }
                "environment" => {
                    eprintln!("environment: {} variables", std::env::vars_os().count());
                    0
                }
                _ => {
                    eprintln!("no attempt {name}");
                    NOT_STOPPED
                }
            }
        }
    }

    fn policy(h: HANDLE, which: PROCESS_MITIGATION_POLICY) -> Result<u32, String> {
        let mut v = 0u32;
        if unsafe { GetProcessMitigationPolicy(h, which, (&raw mut v).cast(), size_of::<u32>()) } == 0 {
            return Err(format!("error {}", unsafe { GetLastError() }));
        }
        Ok(v)
    }

    fn token_info(token: HANDLE, class: i32) -> Result<Vec<u64>, String> {
        let mut buf = vec![0u64; 128];
        let mut n = 0u32;
        if unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast::<c_void>(), 1024, &mut n) } == 0 {
            return Err(format!("error {}", unsafe { GetLastError() }));
        }
        Ok(buf)
    }

    fn outside(failed: &mut Vec<String>) {
        let (settings, expect) = tiny();
        let mut iso = match Isolated::spawn(&settings, &expect) {
            Ok(i) => i,
            Err(e) => {
                failed.push(format!("the child: {e}"));
                return;
            }
        };
        let pid = iso.id();
        let h = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid) };
        if h.is_null() {
            failed.push(format!("the child {pid} cannot be queried: {}", std::io::Error::last_os_error()));
            return;
        }
        let mut check = |what: String, ok: bool| {
            println!("{}  {what}", if ok { "holds   " } else { "FAILS   " });
            if !ok {
                failed.push(what);
            }
        };
        let start = Instant::now();
        while policy(h, ProcessSystemCallDisablePolicy).map_or(true, |v| v & 1 == 0) {
            if start.elapsed() > Duration::from_secs(10) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for (name, which, bits) in [
            ("win32k system calls off (set at the seal)", ProcessSystemCallDisablePolicy, 1),
            ("no dynamic code", ProcessDynamicCodePolicy, 1),
            ("strict handle checks", ProcessStrictHandleCheckPolicy, 1),
            ("extension points off", ProcessExtensionPointDisablePolicy, 1),
            ("system fonts only", ProcessFontDisablePolicy, 1),
            ("no remote, no low-label images, system images first", ProcessImageLoadPolicy, 0b111),
            ("no child process", ProcessChildProcessPolicy, 1),
        ] {
            let v = policy(h, which);
            check(format!("{name}: {v:?}"), v.is_ok_and(|v| v & bits == bits));
        }
        let mut in_job = 0;
        let r = unsafe { IsProcessInJob(h, std::ptr::null_mut(), &mut in_job) };
        check(format!("in a job: {in_job}"), r != 0 && in_job != 0);
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe { OpenProcessToken(h, TOKEN_QUERY, &mut token) } == 0 {
            check(format!("its token: {}", std::io::Error::last_os_error()), false);
        } else {
            let app = token_info(token, TokenIsAppContainer).map(|b| b[0] as u32);
            check(format!("an AppContainer: {app:?}"), app == Ok(1));
            let caps = token_info(token, TokenCapabilities)
                .map(|b| unsafe { (*b.as_ptr().cast::<TOKEN_GROUPS>()).GroupCount });
            check(format!("no capabilities: {caps:?}"), caps == Ok(0));
            let level = token_info(token, TokenIntegrityLevel).map(|b| unsafe {
                let sid = (*b.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
                let n = u32::from(*GetSidSubAuthorityCount(sid));
                *GetSidSubAuthority(sid, n.saturating_sub(1))
            });
            check(format!("low integrity: {level:x?}"), level == Ok(0x1000));
            unsafe { CloseHandle(token) };
        }
        unsafe { CloseHandle(h) };
        match super::decode(&mut iso) {
            Ok(n) => check(format!("the test card decoded in the sandbox: {n} pictures"), n == TINY_PICTURES),
            Err(e) => check(format!("the test card decoded in the sandbox: {e}"), false),
        }
    }

    pub fn run(failed: &mut Vec<String>) {
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        println!("user32 loaded (the screen {width} pixels wide), as in the game's child");
        outside(failed);
        let tmp = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
        let readable = tmp.join("ssx-movie-sandbox-user-file.txt");
        let written = tmp.join("ssx-movie-sandbox-write-attempt");
        let _ = std::fs::remove_file(&written);
        if let Err(e) = std::fs::write(&readable, "the user's own file") {
            failed.push(format!("{}: {e}", readable.display()));
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").and_then(|l| l.set_nonblocking(true).map(|()| l));
        let Ok(listener) = listener else {
            failed.push("no listener on 127.0.0.1".into());
            return;
        };
        let to = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
        let cmd = std::env::var("SystemRoot").map(|r| format!("{r}\\System32\\cmd.exe")).unwrap_or_default();
        let parent = std::process::id().to_string();
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut r, mut w): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
        if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
            failed.push(format!("an inheritable pipe: {}", std::io::Error::last_os_error()));
            return;
        }
        let refusal = ["exit code: 98", "refused:"];
        let (rd, wr) = (readable.to_string_lossy().into_owned(), written.to_string_lossy().into_owned());
        let handle = (w as usize).to_string();
        for (name, arg) in [
            ("open-read", rd.as_str()),
            ("open-write", wr.as_str()),
            ("connect", to.as_str()),
            ("create-process", cmd.as_str()),
            ("open-parent", parent.as_str()),
            ("dynamic-code", ""),
            ("protect-exec", ""),
        ] {
            expect_end(failed, name, arg, &refusal, &[]);
        }
        match super::attempt("handle", &handle) {
            Ok(e) if e.contains("refused:") || e.contains("0xc0000008") => println!("stopped  handle: {e}"),
            Ok(e) => failed.push(format!("handle: {e}")),
            Err(e) => failed.push(format!("handle: {e}")),
        }
        let mut avail = 0u32;
        let peeked = unsafe {
            PeekNamedPipe(r, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
        };
        if peeked == 0 || avail != 0 {
            failed.push(format!("the parent's other pipe: {avail} bytes reached it"));
        }
        unsafe {
            CloseHandle(r);
            CloseHandle(w);
        }
        expect_end(
            failed,
            "memory",
            "",
            &["memory allocation of 8388608 bytes failed", "MiB allocated before the limit"],
            &[],
        );
        expect_end(failed, "environment", "", &["exit code: 0", "environment: 0 variables"], &[]);
        if written.exists() {
            failed.push(format!("{} was made", written.display()));
        }
        match listener.accept() {
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            other => failed.push(format!("the listener on {to} was reached: {other:?}")),
        }
    }
}

#[cfg(not(any(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"), windows)))]
mod platform {
    pub fn attempt(_: &str, _: &str) -> i32 {
        super::NOT_STOPPED
    }

    pub fn run(failed: &mut Vec<String>) {
        failed.push("the movie decoder has no sandbox on this system (Linux x86-64 and Windows have one)".into());
    }
}
