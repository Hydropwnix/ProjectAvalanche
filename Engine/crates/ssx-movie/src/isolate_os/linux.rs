use std::ffi::c_int;
use std::fs::File;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};

use super::{MEMORY_LIMIT, Seal, Spawned, filter};

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
        self.0.signal() == Some(libc::SIGSYS)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub fn spawn(exe: &Path, mode: Option<&str>) -> Result<Spawned, String> {
    let mut child = Command::new(exe)
        .args(mode)
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("the movie decoder's program {} did not start: {e}", exe.display()))?;
    let (Some(i), Some(o), Some(e)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("the movie decoder's pipes".into());
    };
    let file = |fd: OwnedFd| File::from(fd);
    Ok(Spawned { process: Process(child), stdin: file(i.into()), stdout: file(o.into()), stderr: file(e.into()) })
}

fn os(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn limit(resource: libc::__rlimit_resource_t, value: u64, what: &str) -> Result<(), String> {
    let l = libc::rlimit { rlim_cur: value, rlim_max: value };
    if unsafe { libc::setrlimit(resource, &l) } != 0 {
        return Err(os(what));
    }
    Ok(())
}

fn malloc_option(option: c_int, value: c_int, what: &str) -> Result<(), String> {
    if unsafe { libc::mallopt(option, value) } != 1 {
        return Err(format!("glibc's {what}"));
    }
    Ok(())
}

fn prctl(option: c_int, value: libc::c_ulong, what: &str) -> Result<(), String> {
    if unsafe { libc::prctl(option, value, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong) } != 0 {
        return Err(os(what));
    }
    Ok(())
}

pub fn prepare(seal: Seal) -> Result<(), String> {
    if unsafe { libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, 0 as libc::c_uint) } != 0 {
        return Err(os("close_range"));
    }
    let parent = unsafe { libc::getppid() };
    prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, "PR_SET_PDEATHSIG")?;
    if unsafe { libc::getppid() } != parent {
        unsafe { libc::_exit(1) };
    }
    malloc_option(libc::M_ARENA_MAX, c_int::from(super::MAX_THREADS) + 1, "M_ARENA_MAX")?;
    malloc_option(libc::M_TRIM_THRESHOLD, c_int::MAX, "M_TRIM_THRESHOLD")?;
    limit(libc::RLIMIT_CORE, 0, "RLIMIT_CORE")?;
    if seal == Seal::Enforce {
        prctl(libc::PR_SET_DUMPABLE, 0, "PR_SET_DUMPABLE")?;
    }
    prctl(libc::PR_SET_NO_NEW_PRIVS, 1, "PR_SET_NO_NEW_PRIVS")?;
    limit(libc::RLIMIT_DATA, MEMORY_LIMIT, "RLIMIT_DATA")
}

pub fn seal(seal: Seal) -> Result<(), String> {
    let tids = threads()?;
    limit(libc::RLIMIT_NPROC, 0, "RLIMIT_NPROC")?;
    limit(libc::RLIMIT_NOFILE, if seal == Seal::Measure { 4 } else { 3 }, "RLIMIT_NOFILE")?;
    limit(libc::RLIMIT_FSIZE, 0, "RLIMIT_FSIZE")?;
    let pid = unsafe { libc::getpid() };
    reporter()?;
    match seal {
        Seal::Enforce => filter::enforce(pid, &tids),
        Seal::Measure => filter::marker(false),
    }
}

pub fn measure_from_here() -> Result<(), String> {
    filter::measure()
}

fn threads() -> Result<Vec<i32>, String> {
    let list = std::fs::read_dir("/proc/self/task").map_err(|e| format!("/proc/self/task: {e}"))?;
    let mut tids = Vec::new();
    for e in list {
        let e = e.map_err(|e| format!("/proc/self/task: {e}"))?;
        let tid = e.file_name().to_str().and_then(|t| t.parse::<i32>().ok());
        tids.push(tid.ok_or_else(|| format!("/proc/self/task: {:?}", e.file_name()))?);
        if tids.len() > usize::from(super::MAX_THREADS) + 1 {
            return Err(format!("more than {} threads", usize::from(super::MAX_THREADS) + 1));
        }
    }
    Ok(tids)
}

fn reporter() -> Result<(), String> {
    unsafe {
        let mut a: libc::sigaction = std::mem::zeroed();
        a.sa_sigaction = on_sigsys as *const () as libc::sighandler_t;
        a.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND | libc::SA_NODEFER;
        libc::sigemptyset(&mut a.sa_mask);
        if libc::sigaction(libc::SIGSYS, &a, std::ptr::null_mut()) != 0 {
            return Err(os("the SIGSYS handler"));
        }
    }
    Ok(())
}

const SI_SYSCALL: usize = 24;

struct Line {
    buf: [u8; 224],
    n: usize,
}

impl Line {
    fn put(&mut self, s: &[u8]) {
        for &b in s {
            if self.n < self.buf.len() {
                self.buf[self.n] = b;
                self.n += 1;
            }
        }
    }

    fn number(&mut self, v: u64, base: u64) {
        let mut digits = [0u8; 20];
        let (mut u, mut k) = (v, digits.len());
        loop {
            k -= 1;
            digits[k] = b"0123456789abcdef"[(u % base) as usize];
            u /= base;
            if u == 0 {
                break;
            }
        }
        self.put(&digits[k..]);
    }
}

extern "C" fn on_sigsys(_sig: c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    let (nr, args) = unsafe {
        let nr = info.cast::<u8>().add(SI_SYSCALL).cast::<c_int>().read_unaligned();
        let g = &(*ctx.cast::<libc::ucontext_t>()).uc_mcontext.gregs;
        let r = |k: c_int| g[k as usize] as u64;
        (nr, [r(libc::REG_RDI), r(libc::REG_RSI), r(libc::REG_RDX), r(libc::REG_R10)])
    };
    let mut line = Line { buf: [0; 224], n: 0 };
    line.put(b"the movie decoder's sandbox refused system call ");
    line.number(u64::from(nr as u32), 10);
    if let Some(name) = filter::call_name(i64::from(nr)) {
        line.put(b" (");
        line.put(name.as_bytes());
        line.put(b")");
    }
    line.put(b", arguments");
    for a in args {
        line.put(b" 0x");
        line.number(a, 16);
    }
    line.put(b"\n");
    unsafe {
        libc::write(2, line.buf.as_ptr().cast(), line.n);
        libc::raise(libc::SIGSYS);
    }
}
