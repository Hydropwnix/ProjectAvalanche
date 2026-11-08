use std::ffi::CString;

use super::{NOT_STOPPED, not_stopped};

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
