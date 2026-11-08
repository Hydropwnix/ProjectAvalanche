use std::ffi::OsString;
use std::fs::File;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
use windows_sys::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::Security::{SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, IsProcessInJob, TerminateJobObject};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetSystemWindowsDirectoryW, OSVERSIONINFOW};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
    DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess, GetExitCodeProcess,
    GetProcessMitigationPolicy, InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY,
    PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION, PROCESS_MITIGATION_POLICY,
    ProcessChildProcessPolicy, ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy, ProcessFontDisablePolicy,
    ProcessImageLoadPolicy, ProcessMitigationOptionsMask, ProcessSystemCallDisablePolicy, ResumeThread,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use super::super::Spawned;
use super::access::{container, ensure};
use super::job::{decoder_job, outer_job, process_ids};
use super::{MITIGATION_VALUE, MITIGATIONS, last, owned, wide};

// TODO(guess): Windows before 10 1809 (build 17763) refuses an AppContainer process with creation-time mitigations
// (CreateProcessW fails with ERROR_INVALID_PARAMETER, 87), as Chromium's sandbox_policy_base.cc records ("Prior to
// Win10 RS5 CreateProcess fails when AppContainer and mitigation flags are enabled"); no build before or at 17763 was
// tried here (the Windows CI runs a Server build after it); settle: the decoder started on Windows 10 1803 (17134)
// and 1809 (17763).
const MIN_BUILD: u32 = 17763;

pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
}

pub struct Status(u32);

impl Process {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn handle(&self) -> BorrowedHandle<'_> {
        self.process.as_handle()
    }

    pub fn job(&self) -> BorrowedHandle<'_> {
        self.job.as_handle()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<Status>> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => self.code().map(Some),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(std::io::Error::last_os_error()),
        }
    }

    fn code(&self) -> std::io::Result<Status> {
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Status(code))
    }

    pub fn kill(&mut self) -> std::io::Result<()> {
        if unsafe { TerminateJobObject(self.job.as_raw_handle(), 1) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn wait(&mut self) -> std::io::Result<Status> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 10_000) } {
            WAIT_OBJECT_0 => self.code(),
            _ => Err(std::io::Error::other("the movie decoder's process did not end within 10 s")),
        }
    }
}

impl Status {
    pub fn by_sandbox(&self) -> bool {
        false
    }
}

fn meaning(code: u32) -> Option<&'static str> {
    Some(match code {
        0xC000_0005 => "STATUS_ACCESS_VIOLATION",
        0xC000_0008 => "STATUS_INVALID_HANDLE: a strict handle check ended it",
        0xC000_0142 => "STATUS_DLL_INIT_FAILED: a library that needs win32k was loaded at its start",
        0xC000_0409 => "a fast fail: Rust's abort",
        _ => return None,
    })
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 >= 0x8000_0000 {
            write!(f, "exit code: 0x{:08x}", self.0)?;
        } else {
            write!(f, "exit code: {}", self.0)?;
        }
        match meaning(self.0) {
            Some(m) => write!(f, " ({m})"),
            None => Ok(()),
        }
    }
}

pub fn windows_build() -> Result<u32, String> {
    let mut v: OSVERSIONINFOW = unsafe { std::mem::zeroed() };
    v.dwOSVersionInfoSize = size_of::<OSVERSIONINFOW>() as u32;
    let status = unsafe { RtlGetVersion(&mut v) };
    if status < 0 {
        return Err(format!("the Windows version: NTSTATUS 0x{:08x}", status as u32));
    }
    Ok(v.dwBuildNumber)
}

fn supported() -> Result<(), String> {
    let mut mask = [0u64; 2];
    let ok = unsafe {
        GetProcessMitigationPolicy(
            GetCurrentProcess(),
            ProcessMitigationOptionsMask,
            mask.as_mut_ptr().cast(),
            size_of_val(&mask),
        )
    };
    if ok == 0 {
        return Err(last("the mitigations this Windows has"));
    }
    let missing: Vec<&str> = MITIGATIONS.iter().filter(|(bit, _)| mask[0] & bit == 0).map(|(_, n)| *n).collect();
    if !missing.is_empty() {
        return Err(format!("this Windows lacks the decoder's mitigations: {}", missing.join(", ")));
    }
    Ok(())
}

const VALUE_MAX: usize = 32_766;

fn local_app_data() -> Result<OsString, String> {
    let mut path: *mut u16 = std::ptr::null_mut();
    let hr = unsafe {
        SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut path)
    };
    let mut len = 0usize;
    while hr >= 0 && !path.is_null() && len <= VALUE_MAX && unsafe { *path.add(len) } != 0 {
        len += 1;
    }
    let value = if hr < 0 || path.is_null() || len == 0 || len > VALUE_MAX {
        Err(format!("the local application data folder: HRESULT 0x{:08x}, {len} units", hr as u32))
    } else {
        Ok(OsString::from_wide(unsafe { std::slice::from_raw_parts(path, len) }))
    };
    unsafe { CoTaskMemFree(path.cast()) };
    value
}

fn windows_folder() -> Result<OsString, String> {
    let mut buf = vec![0u16; 261];
    for _ in 0..2 {
        let n = unsafe { GetSystemWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 || n > VALUE_MAX {
            return Err(last("the Windows folder"));
        }
        if n < buf.len() {
            return Ok(OsString::from_wide(&buf[..n]));
        }
        buf = vec![0u16; n];
    }
    Err("the Windows folder: its length changed between two calls".into())
}

pub fn passed_environment() -> Result<Vec<(&'static str, OsString)>, String> {
    Ok(vec![("LOCALAPPDATA", local_app_data()?), ("SystemRoot", windows_folder()?)])
}

fn environment() -> Result<Vec<u16>, String> {
    // TODO(guess): LOCALAPPDATA and SystemRoot are all an AppContainer child's block needs, and the child then holds
    // exactly LOCALAPPDATA, SystemRoot, TEMP and TMP; known: an empty block fails CreateProcessW with
    // ERROR_ENVVAR_NOT_FOUND (203, the Windows CI's first run, 42443408055) and Chromium's filtered block keeps
    // LOCALAPPDATA "for App Container processes" and SystemRoot "for normal operation"; settle: the Windows CI line
    // "holds    environment: 4 variables: [LOCALAPPDATA SystemRoot TEMP TMP]" of the sandbox step.
    let mut block = Vec::new();
    for (name, value) in passed_environment()? {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Ok(block)
}

fn pipe(child_reads: bool) -> Result<(OwnedHandle, OwnedHandle), String> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    let (mut r, mut w): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
    if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
        return Err(last("a pipe for the movie decoder"));
    }
    let (r, w) = (owned(r, "a pipe")?, owned(w, "a pipe")?);
    let (child, parent) = if child_reads { (r, w) } else { (w, r) };
    if unsafe { SetHandleInformation(parent.as_raw_handle(), HANDLE_FLAG_INHERIT, 0) } == 0 {
        return Err(last("a pipe for the movie decoder"));
    }
    Ok((child, parent))
}

struct Attributes(Vec<u8>);

impl Attributes {
    fn new(count: u32) -> Result<Self, String> {
        let mut size = 0usize;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), count, 0, &mut size) };
        let mut buf = vec![0u8; size];
        if unsafe { InitializeProcThreadAttributeList(buf.as_mut_ptr().cast(), count, 0, &mut size) } == 0 {
            return Err(last("the decoder's process attributes"));
        }
        Ok(Self(buf))
    }

    fn set<T>(&mut self, attribute: u32, value: *const T, size: usize) -> Result<(), String> {
        let ok = unsafe {
            UpdateProcThreadAttribute(
                self.0.as_mut_ptr().cast(),
                0,
                attribute as usize,
                value.cast(),
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(last("a process attribute of the decoder"));
        }
        Ok(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.0.as_mut_ptr().cast()) };
    }
}

const CREATION_POLICIES: [(&str, PROCESS_MITIGATION_POLICY, u32); 6] = [
    ("win32k system calls off", ProcessSystemCallDisablePolicy, 1),
    ("no dynamic code", ProcessDynamicCodePolicy, 1),
    ("extension points off", ProcessExtensionPointDisablePolicy, 1),
    ("system fonts only", ProcessFontDisablePolicy, 1),
    ("no remote, no low-label images, system images first", ProcessImageLoadPolicy, 0b111),
    ("no child process", ProcessChildProcessPolicy, 1),
];

fn verify(p: &Process) -> Result<(), String> {
    let h = p.process.as_raw_handle();
    for (name, which, bits) in CREATION_POLICIES {
        let mut v = 0u32;
        if unsafe { GetProcessMitigationPolicy(h, which, (&raw mut v).cast(), size_of::<u32>()) } == 0 {
            return Err(last(&format!("its policy \"{name}\" cannot be read")));
        }
        if v & bits != bits {
            return Err(format!("its policy \"{name}\" is not in force (flags 0x{v:x})"));
        }
    }
    let mut in_job = 0;
    if unsafe { IsProcessInJob(h, p.job.as_raw_handle(), &mut in_job) } == 0 {
        return Err(last("its job"));
    }
    if in_job == 0 {
        return Err("it is not in the decoder's job".into());
    }
    let (assigned, ids) = process_ids(p.job.as_raw_handle())?;
    if ids == [p.pid as usize] {
        return Ok(());
    }
    if ids.is_empty() && unsafe { WaitForSingleObject(h, 0) } == WAIT_OBJECT_0 {
        return Ok(());
    }
    Err(format!("the decoder's job holds {assigned} processes {ids:?}, not [{}]", p.pid))
}

pub fn spawn(exe: &Path, mode: Option<&str>) -> Result<Spawned, String> {
    if mode.is_some_and(|m| m.is_empty() || !m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
        return Err("the movie decoder's mode is one word".into());
    }
    let build = windows_build()?;
    if build < MIN_BUILD {
        return Err(format!(
            "the movie decoder's sandbox needs Windows 10 1809 (build {MIN_BUILD}) or later; this is build {build}"
        ));
    }
    supported()?;
    let outer = outer_job()?;
    let sid = container()?;
    ensure(exe, &sid)?;
    let (child_in, parent_in) = pipe(true)?;
    let (child_out, parent_out) = pipe(false)?;
    let (child_err, parent_err) = pipe(false)?;
    let job = decoder_job()?;
    let handles: [HANDLE; 3] = [child_in.as_raw_handle(), child_out.as_raw_handle(), child_err.as_raw_handle()];
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mitigations: u64 = MITIGATION_VALUE;
    let children: u32 = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    let mut attrs = Attributes::new(4)?;
    attrs.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, handles.as_ptr(), size_of_val(&handles))?;
    attrs.set(PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, &raw const caps, size_of::<SECURITY_CAPABILITIES>())?;
    attrs.set(PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, &raw const mitigations, size_of::<u64>())?;
    attrs.set(PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, &raw const children, size_of::<u32>())?;
    let mut si: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    (si.StartupInfo.hStdInput, si.StartupInfo.hStdOutput, si.StartupInfo.hStdError) =
        (handles[0], handles[1], handles[2]);
    si.lpAttributeList = attrs.0.as_mut_ptr().cast();
    let app = wide(exe.as_os_str());
    let mut command = OsString::from("\"");
    command.push(exe.as_os_str());
    command.push("\"");
    if let Some(m) = mode {
        command.push(" ");
        command.push(m);
    }
    let mut line = wide(&command);
    let env = environment()?;
    let mut sys = [0u16; 261];
    let n = unsafe { GetSystemDirectoryW(sys.as_mut_ptr(), sys.len() as u32) } as usize;
    if n == 0 || n >= sys.len() {
        return Err(last("the system folder"));
    }
    let breakaway = if outer.breaks_away() { CREATE_BREAKAWAY_FROM_JOB } else { 0 };
    let flags = EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | DETACHED_PROCESS | CREATE_UNICODE_ENVIRONMENT;
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            flags | breakaway,
            env.as_ptr().cast(),
            sys.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        )
    };
    if ok == 0 {
        let e = std::io::Error::last_os_error();
        let why = match e.raw_os_error() {
            Some(87) => "; Windows refused the AppContainer with its mitigations (Windows 10 1809 or later needed)",
            Some(203) => "; a variable Windows needs is missing from its environment",
            _ => "",
        };
        return Err(format!("the movie decoder's program {} did not start: {e}{why}", exe.display()));
    }
    let (process, thread) =
        unsafe { (OwnedHandle::from_raw_handle(pi.hProcess), OwnedHandle::from_raw_handle(pi.hThread)) };
    let me = Process { process, job, pid: pi.dwProcessId };
    unsafe {
        if AssignProcessToJobObject(me.job.as_raw_handle(), me.process.as_raw_handle()) == 0 {
            let e = last("the movie decoder's process cannot join its job");
            windows_sys::Win32::System::Threading::TerminateProcess(me.process.as_raw_handle(), 1);
            if outer.in_job {
                return Err(format!("{e} (this program runs in a job it may not leave: {})", outer.limit_names()));
            }
            return Err(e);
        }
        if ResumeThread(thread.as_raw_handle()) == u32::MAX {
            let e = last("the movie decoder's process does not start");
            TerminateJobObject(me.job.as_raw_handle(), 1);
            return Err(e);
        }
    }
    drop((thread, child_in, child_out, child_err));
    if let Err(e) = verify(&me) {
        unsafe {
            TerminateJobObject(me.job.as_raw_handle(), 1);
            WaitForSingleObject(me.process.as_raw_handle(), 10_000);
        }
        return Err(format!("the movie decoder's sandbox does not hold: {e}"));
    }
    Ok(Spawned {
        process: me,
        stdin: File::from(parent_in),
        stdout: File::from(parent_out),
        stderr: File::from(parent_err),
    })
}
