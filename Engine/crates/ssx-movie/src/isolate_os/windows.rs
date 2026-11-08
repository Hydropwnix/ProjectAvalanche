use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;

mod access;
mod child;
mod job;
mod spawn;

pub use access::{ImageAccess, container_may_run, grant_image_access};
pub(crate) use child::{measure_from_here, prepare, seal};
pub use job::{OuterJob, outer_job};
pub use spawn::{Process, Status, passed_environment, spawn, windows_build};

const CONTAINER: &str = "ProjectAvalanche.MovieDecoder";

const MITIGATIONS: [(u64, &str); 8] = [
    (1 << 12, "heap terminate"),
    (1 << 28, "win32k system calls disable"),
    (1 << 32, "extension point disable"),
    (1 << 36, "prohibit dynamic code"),
    (1 << 48, "font disable"),
    (1 << 52, "image load no remote"),
    (1 << 56, "image load no low label"),
    (1 << 60, "image load prefer system32"),
];

const fn mitigation_value() -> u64 {
    let mut v = 0;
    let mut k = 0;
    while k < MITIGATIONS.len() {
        v |= MITIGATIONS[k].0;
        k += 1;
    }
    v
}

const MITIGATION_VALUE: u64 = mitigation_value();

fn last(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn owned(h: HANDLE, what: &str) -> Result<OwnedHandle, String> {
    if h.is_null() || h as isize == -1 {
        return Err(last(what));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_creation_value() {
        assert_eq!(super::MITIGATION_VALUE, 0x1111_0011_1000_1000);
        assert_eq!(super::MITIGATION_VALUE & (1 << 24), 0);
    }
}
