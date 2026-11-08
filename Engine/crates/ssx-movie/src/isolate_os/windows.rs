use std::ffi::c_void;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, LocalFree, SetHandleInformation,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, EqualSid, FreeSid, GetAce, NO_INHERITANCE, PSID,
    SECURITY_ATTRIBUTES, SECURITY_CAPABILITIES,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Diagnostics::Debug::{SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX, SetErrorMode};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::SystemServices::PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_MITIGATION_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES, PROCESS_INFORMATION,
    ProcessSystemCallDisablePolicy, ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, SetProcessMitigationPolicy,
    TerminateProcess, WaitForSingleObject,
};
use windows_sys::Win32::System::WindowsProgramming::PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
use windows_sys::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

use super::{MEMORY_LIMIT, Seal, Spawned};

const CONTAINER: &str = "ProjectAvalanche.MovieDecoder";

const HEAP_TERMINATE: u64 = 1 << 12;
const STRICT_HANDLE_CHECKS: u64 = 1 << 24;
const EXTENSION_POINT_DISABLE: u64 = 1 << 32;
const PROHIBIT_DYNAMIC_CODE: u64 = 1 << 36;
const FONT_DISABLE: u64 = 1 << 48;
const IMAGE_LOAD_NO_REMOTE: u64 = 1 << 52;
const IMAGE_LOAD_NO_LOW_LABEL: u64 = 1 << 56;
const IMAGE_LOAD_PREFER_SYSTEM32: u64 = 1 << 60;

pub struct Process {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
}

pub struct Status(u32);

fn last(what: &str) -> String {
    format!("{what}: {}", std::io::Error::last_os_error())
}

fn owned(h: HANDLE, what: &str) -> Result<OwnedHandle, String> {
    if h.is_null() || h as isize == -1 {
        return Err(last(what));
    }
    Ok(unsafe { OwnedHandle::from_raw_handle(h) })
}

impl Process {
    pub fn id(&self) -> u32 {
        self.pid
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

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0 >= 0x8000_0000 {
            write!(f, "exit code: 0x{:08x}", self.0)
        } else {
            write!(f, "exit code: {}", self.0)
        }
    }
}

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

struct Sid(PSID);

impl Drop for Sid {
    fn drop(&mut self) {
        unsafe { FreeSid(self.0) };
    }
}

fn container() -> Result<Sid, String> {
    let name = wide(std::ffi::OsStr::new(CONTAINER));
    let shown = wide(std::ffi::OsStr::new("Project Avalanche movie decoder"));
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(name.as_ptr(), shown.as_ptr(), shown.as_ptr(), std::ptr::null(), 0, &mut sid)
    };
    if hr >= 0 && !sid.is_null() {
        return Ok(Sid(sid));
    }
    if hr != (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32 {
        return Err(format!("the decoder's AppContainer: HRESULT 0x{:08x}", hr as u32));
    }
    let hr = unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) };
    if hr < 0 || sid.is_null() {
        return Err(format!("the decoder's AppContainer SID: HRESULT 0x{:08x}", hr as u32));
    }
    Ok(Sid(sid))
}

struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

struct TaskMem(*mut c_void);

impl Drop for TaskMem {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(self.0) };
    }
}

const VALUE_MAX: usize = 32_766;

fn environment() -> Result<Vec<u16>, String> {
    // TODO(guess): LOCALAPPDATA is the one variable Windows' AppContainer process creation needs in the block (an
    // empty block fails with ERROR_ENVVAR_NOT_FOUND 203; Chromium's filtered environment keeps it "for App Container
    // processes"), and the child then holds at most LOCALAPPDATA, TEMP and TMP; not run on Windows yet. Settled by the
    // Windows CI step `cargo test --release -p ssx-movie --test sandbox`, whose "environment" attempt lists the
    // child's variables.
    let mut path: *mut u16 = std::ptr::null_mut();
    let hr = unsafe {
        SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT as u32, std::ptr::null_mut(), &mut path)
    };
    let _path = TaskMem(path.cast());
    if hr < 0 || path.is_null() {
        return Err(format!("the local application data folder: HRESULT 0x{:08x}", hr as u32));
    }
    let mut len = 0usize;
    while len <= VALUE_MAX && unsafe { *path.add(len) } != 0 {
        len += 1;
    }
    if len == 0 || len > VALUE_MAX {
        return Err(format!("the local application data folder: a path of {len} or more units"));
    }
    let folder = unsafe { std::slice::from_raw_parts(path, len) };
    let mut block: Vec<u16> = "LOCALAPPDATA=".encode_utf16().collect();
    block.extend_from_slice(folder);
    block.extend([0, 0]);
    Ok(block)
}

fn image_access(exe: &Path, sid: &Sid) -> Result<(), String> {
    let path = wide(exe.as_os_str());
    let want = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
    let (mut dacl, mut sd): (*mut ACL, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
    let e = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    let _sd_guard = Local(sd);
    if e != ERROR_SUCCESS {
        return Err(format!("this program's file's security: error {e}"));
    }
    if dacl.is_null() {
        return Ok(());
    }
    let has = unsafe {
        (0..u32::from((*dacl).AceCount)).any(|i| {
            let mut ace: *mut c_void = std::ptr::null_mut();
            if GetAce(dacl, i, &mut ace) == 0 || ace.is_null() {
                return false;
            }
            let a = ace.cast::<ACCESS_ALLOWED_ACE>();
            (*a).Header.AceType == 0
                && (*a).Mask & want == want
                && EqualSid((&raw mut (*a).SidStart).cast(), sid.0) != 0
        })
    };
    if has {
        return Ok(());
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: want,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: NO_INHERITANCE,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid.0.cast(),
        },
    };
    let mut merged: *mut ACL = std::ptr::null_mut();
    let e = unsafe { SetEntriesInAclW(1, &entry, dacl, &mut merged) };
    let _merged = Local(merged.cast());
    if e != ERROR_SUCCESS {
        return Err(format!("this program's file's DACL: error {e}"));
    }
    let e = unsafe {
        SetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            merged,
            std::ptr::null(),
        )
    };
    if e != ERROR_SUCCESS {
        return Err(format!("this program's file cannot be opened to the decoder's AppContainer: error {e}"));
    }
    Ok(())
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

fn job() -> Result<OwnedHandle, String> {
    let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) }, "the decoder's job")?;
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
    info.BasicLimitInformation.ActiveProcessLimit = 1;
    // TODO(guess): the committed memory of the child on Windows is within the Linux measurement of its data (VmData,
    // 176 MiB as the programs ask, 490 MiB at 16 threads and 16 frames in flight) and so within MEMORY_LIMIT (768 MiB);
    // not measured on Windows (rav1d's buffers through the system heap, thread stacks committed as they grow). Settled
    // by the peak commit of the child (PeakProcessMemoryUsed of the job's extended information) over the proof inputs
    // on Windows.
    info.ProcessMemoryLimit = usize::try_from(MEMORY_LIMIT).map_err(|_| "the memory limit")?;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    unsafe {
        let h = job.as_raw_handle();
        let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        if SetInformationJobObject(h, JobObjectExtendedLimitInformation, (&raw const info).cast(), size) == 0 {
            return Err(last("the decoder's job limits"));
        }
        let size = size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32;
        if SetInformationJobObject(h, JobObjectBasicUIRestrictions, (&raw const ui).cast(), size) == 0 {
            return Err(last("the decoder's job UI limits"));
        }
    }
    Ok(job)
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
            windows_sys::Win32::System::Threading::UpdateProcThreadAttribute(
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

pub fn spawn(exe: &Path, arg: &str) -> Result<Spawned, String> {
    let sid = container()?;
    image_access(exe, &sid)?;
    let (child_in, parent_in) = pipe(true)?;
    let (child_out, parent_out) = pipe(false)?;
    let (child_err, parent_err) = pipe(false)?;
    let job = job()?;
    let handles: [HANDLE; 3] = [child_in.as_raw_handle(), child_out.as_raw_handle(), child_err.as_raw_handle()];
    let caps = SECURITY_CAPABILITIES {
        AppContainerSid: sid.0,
        Capabilities: std::ptr::null_mut(),
        CapabilityCount: 0,
        Reserved: 0,
    };
    let mitigations: u64 = HEAP_TERMINATE
        | STRICT_HANDLE_CHECKS
        | EXTENSION_POINT_DISABLE
        | PROHIBIT_DYNAMIC_CODE
        | FONT_DISABLE
        | IMAGE_LOAD_NO_REMOTE
        | IMAGE_LOAD_NO_LOW_LABEL
        | IMAGE_LOAD_PREFER_SYSTEM32;
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
    let mut line = wide(std::ffi::OsStr::new(&format!("\"{}\" {arg}", exe.display())));
    let env = environment()?;
    let mut sys = [0u16; 261];
    let n = unsafe { GetSystemDirectoryW(sys.as_mut_ptr(), sys.len() as u32) } as usize;
    if n == 0 || n >= sys.len() {
        return Err(last("the system folder"));
    }
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            app.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            env.as_ptr().cast(),
            sys.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        )
    };
    if ok == 0 {
        return Err(last("the movie decoder's process did not start"));
    }
    let (process, thread) =
        unsafe { (OwnedHandle::from_raw_handle(pi.hProcess), OwnedHandle::from_raw_handle(pi.hThread)) };
    let me = Process { process, job, pid: pi.dwProcessId };
    unsafe {
        if AssignProcessToJobObject(me.job.as_raw_handle(), me.process.as_raw_handle()) == 0 {
            let e = last("the movie decoder's process joins no job");
            TerminateProcess(me.process.as_raw_handle(), 1);
            return Err(e);
        }
        if ResumeThread(thread.as_raw_handle()) == u32::MAX {
            let e = last("the movie decoder's process does not start");
            TerminateProcess(me.process.as_raw_handle(), 1);
            return Err(e);
        }
    }
    drop((thread, child_in, child_out, child_err));
    Ok(Spawned {
        process: me,
        stdin: File::from(parent_in),
        stdout: File::from(parent_out),
        stderr: File::from(parent_err),
    })
}

pub fn prepare(seal: Seal) -> Result<(), String> {
    if seal == Seal::Measure {
        return Err("the sandbox's list is measured on Linux".into());
    }
    unsafe { SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
    Ok(())
}

pub fn seal(_: Seal) -> Result<(), String> {
    // TODO(guess): turning win32k off at run time, after user32 (which the game imports) has started in this process,
    // succeeds and leaves the decoding and the process's end unharmed; not run on Windows yet (the creation-time flag
    // is not used because user32's start needs win32k). Settled by the Windows CI step `cargo test --release -p
    // ssx-movie --test sandbox`, whose program imports user32 too.
    let mut p = PROCESS_MITIGATION_SYSTEM_CALL_DISABLE_POLICY::default();
    p.Anonymous.Flags = 1;
    let ok =
        unsafe { SetProcessMitigationPolicy(ProcessSystemCallDisablePolicy, (&raw const p).cast(), size_of_val(&p)) };
    if ok == 0 {
        return Err(format!("win32k's system calls stay on: error {}", unsafe { GetLastError() }));
    }
    Ok(())
}

pub fn measure_from_here() -> Result<(), String> {
    Err("the sandbox's list is measured on Linux".into())
}
