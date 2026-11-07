use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};

use super::Status;

const KEY: &str = r"Software\Classes\avalanche";
const COMMAND_KEY: &str = r"Software\Classes\avalanche\shell\open\command";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn command(exe: &Path) -> String {
    format!("\"{}\" --link \"%1\"", exe.display())
}

fn read(key: &str, name: &str) -> Result<Option<String>, String> {
    let (key, name) = (wide(key), wide(name));
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    match r {
        ERROR_SUCCESS => {
            let n = (size as usize / 2).min(buf.len());
            Ok(Some(String::from_utf16_lossy(&buf[..n]).trim_end_matches('\0').to_string()))
        }
        ERROR_FILE_NOT_FOUND => Ok(None),
        e => Err(format!("reading HKCU\\{}: error {e}", String::from_utf16_lossy(&key[..key.len() - 1]))),
    }
}

fn write(key: &str, name: &str, value: &str) -> Result<(), String> {
    let (k, n, v) = (wide(key), wide(name), wide(value));
    let r = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), n.as_ptr(), REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32)
    };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(format!("writing HKCU\\{key} {name:?}: error {r}")) }
}

pub fn status(exe: &Path) -> Status {
    match read(COMMAND_KEY, "") {
        Ok(Some(c)) if c == command(exe) => Status::Ours,
        Ok(Some(c)) => Status::Other(c),
        Ok(None) => Status::None,
        Err(e) => Status::Unavailable(e),
    }
}

pub fn register(exe: &Path) -> Result<String, String> {
    write(KEY, "", "URL:Project Avalanche server link")?;
    write(KEY, "URL Protocol", "")?;
    write(COMMAND_KEY, "", &command(exe))?;
    Ok(format!("wrote HKCU\\{KEY} (URL Protocol) with the command {}", command(exe)))
}

pub fn unregister(exe: &Path) -> Result<String, String> {
    match status(exe) {
        Status::Ours => {}
        Status::None => return Ok("nothing to remove".into()),
        Status::Other(c) => return Err(format!("another program handles avalanche:// links ({c}): left as it is")),
        Status::Unavailable(e) => return Err(e),
    }
    let k = wide(KEY);
    let r = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if r == ERROR_SUCCESS {
        Ok(format!("deleted HKCU\\{KEY}"))
    } else {
        Err(format!("deleting HKCU\\{KEY}: error {r}"))
    }
}
