use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemServices::PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessMitigationPolicy, PROCESS_MITIGATION_POLICY, ProcessStrictHandleCheckPolicy,
    ProcessSystemCallDisablePolicy, SetProcessMitigationPolicy,
};

use super::super::Seal;

const STRICT_HANDLES: u32 = 0b11;
const WIN32K_OFF: u32 = 1;

fn own_policy(which: PROCESS_MITIGATION_POLICY, name: &str) -> Result<u32, String> {
    let mut v = 0u32;
    if unsafe { GetProcessMitigationPolicy(GetCurrentProcess(), which, (&raw mut v).cast(), size_of::<u32>()) } == 0 {
        return Err(format!("{name} cannot be read: error {}", unsafe { GetLastError() }));
    }
    Ok(v)
}

pub fn prepare(seal: Seal) -> Result<(), String> {
    if seal == Seal::Measure {
        return Err("the sandbox's list is measured on Linux".into());
    }
    let mut p = PROCESS_MITIGATION_STRICT_HANDLE_CHECK_POLICY::default();
    p.Anonymous.Flags = STRICT_HANDLES;
    // TODO(guess): the AppContainer child may set strict handle checks on itself after its start; Chromium's sandbox
    // and Firefox's RDD decoder set them after startup, in the child (Chromium refuses them at creation, no reason
    // given); not run on Windows yet (the Windows CI's third run read "error 5" for them only after its child had
    // ended); settle: the Windows CI line "holds    strict handle checks (set by the child, read while it runs):
    // Ok(3)" of `cargo test --release -p ssx-movie --features sandbox-probe --test sandbox`.
    // SAFETY: the structure lives for the call and its size is passed.
    let ok =
        unsafe { SetProcessMitigationPolicy(ProcessStrictHandleCheckPolicy, (&raw const p).cast(), size_of_val(&p)) };
    if ok == 0 {
        return Err(format!("strict handle checks stay off: error {}", unsafe { GetLastError() }));
    }
    let v = own_policy(ProcessStrictHandleCheckPolicy, "strict handle checks")?;
    if v != STRICT_HANDLES {
        return Err(format!("strict handle checks read back as 0x{v:x}, not 0x3"));
    }
    for dll in ["user32.dll", "gdi32.dll"] {
        let name: Vec<u16> = dll.encode_utf16().chain(std::iter::once(0)).collect();
        if !unsafe { GetModuleHandleW(name.as_ptr()) }.is_null() {
            return Err(format!("{dll} is loaded in the decoder's process (a window library: win32k's)"));
        }
    }
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
    Ok(())
}

pub fn seal(_: Seal) -> Result<(), String> {
    let v = own_policy(ProcessSystemCallDisablePolicy, "win32k's lockdown")?;
    if v & WIN32K_OFF == 0 {
        return Err(format!("win32k's system calls are on (policy flags 0x{v:x}): the creation's lockdown is missing"));
    }
    Ok(())
}

pub fn measure_from_here() -> Result<(), String> {
    Err("the sandbox's list is measured on Linux".into())
}
