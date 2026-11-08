use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, IN_ADDR, IN_ADDR_0, INVALID_SOCKET, IPPROTO_TCP, SOCK_STREAM, SOCKADDR, SOCKADDR_IN, SOCKET_ERROR,
    WSA_FLAG_NO_HANDLE_INHERIT, WSADATA, WSAGetLastError, WSASocketW, WSAStartup, closesocket, connect,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, WriteFile,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_READWRITE, VirtualAlloc, VirtualProtect,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, PROCESS_VM_READ};

use super::{NOT_STOPPED, not_stopped};

const REFUSED: i32 = 98;
const NOT_MADE: i32 = 96;

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn refused(what: &str) -> i32 {
    let e = unsafe { GetLastError() };
    eprintln!("refused: {what}: error {e}");
    REFUSED
}

fn connect_to(to: &str) -> i32 {
    let Some(port) = to.strip_prefix("127.0.0.1:").and_then(|p| p.parse::<u16>().ok()) else {
        eprintln!("no address in {to:?}");
        return NOT_MADE;
    };
    unsafe {
        let mut data: WSADATA = std::mem::zeroed();
        let e = WSAStartup(0x0202, &mut data);
        if e != 0 {
            eprintln!("no Winsock: error {e}");
            return NOT_MADE;
        }
        let s =
            WSASocketW(i32::from(AF_INET), SOCK_STREAM, IPPROTO_TCP, std::ptr::null(), 0, WSA_FLAG_NO_HANDLE_INHERIT);
        if s == INVALID_SOCKET {
            eprintln!("no socket: error {}", WSAGetLastError());
            return NOT_MADE;
        }
        eprintln!("socket made");
        let a = SOCKADDR_IN {
            sin_family: AF_INET,
            sin_port: port.to_be(),
            sin_addr: IN_ADDR { S_un: IN_ADDR_0 { S_addr: u32::from_be_bytes([127, 0, 0, 1]).to_be() } },
            sin_zero: [0; 8],
        };
        let r = connect(s, (&raw const a).cast::<SOCKADDR>(), size_of::<SOCKADDR_IN>() as i32);
        let code = if r == SOCKET_ERROR {
            let e = std::io::Error::from_raw_os_error(WSAGetLastError());
            eprintln!("refused: connect to {to}: {e}");
            REFUSED
        } else {
            not_stopped(&format!("connect to {to}"))
        };
        closesocket(s);
        code
    }
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
            "connect" => connect_to(arg),
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
                let p = VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
                if p.is_null() {
                    eprintln!("no page to try");
                    return NOT_MADE;
                }
                let mut old = 0u32;
                if VirtualProtect(p, 4096, PAGE_EXECUTE_READ, &mut old) == 0 {
                    refused("memory made executable")
                } else {
                    not_stopped("memory made executable")
                }
            }
            "win32k" => {
                let h = LoadLibraryW(wide("gdi32.dll").as_ptr());
                if h.is_null() { refused("gdi32.dll not loaded") } else { not_stopped("gdi32.dll loaded") }
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
            "environment" if !arg.is_empty() => {
                match std::env::var_os(arg) {
                    Some(v) => eprintln!("{arg}={}", v.to_string_lossy()),
                    None => eprintln!("{arg} is not set"),
                }
                0
            }
            "environment" => {
                let vars: Vec<_> = std::env::vars_os().collect();
                let names: Vec<_> = vars.iter().map(|(k, _)| k.to_string_lossy()).collect();
                eprintln!("environment: {} variables: [{}]", vars.len(), names.join(" "));
                for (k, v) in &vars {
                    eprintln!("{}={}", k.to_string_lossy(), v.to_string_lossy());
                }
                0
            }
            "modules" => {
                let loaded = |dll: &str| !GetModuleHandleW(wide(dll).as_ptr()).is_null();
                let said = |dll: &str| if loaded(dll) { format!("{dll} LOADED") } else { format!("{dll} not loaded") };
                eprintln!("modules: {}, {}", said("user32"), said("gdi32"));
                0
            }
            _ => {
                eprintln!("no attempt {name}");
                NOT_STOPPED
            }
        }
    }
}
