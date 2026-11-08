use std::net::TcpListener;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use ssx_movie::isolate::MEMORY_LIMIT;
use ssx_movie::isolate_os::{self, ImageAccess};
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessMitigationPolicy};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_TIMEOUT};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_GROUPS,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenCapabilities, TokenIntegrityLevel, TokenIsAppContainer,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOBOBJECT_BASIC_UI_RESTRICTIONS,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList, JobObjectBasicUIRestrictions,
    JobObjectExtendedLimitInformation, QueryInformationJobObject,
};
use windows_sys::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetProcessHandleCount, GetProcessMitigationPolicy, OpenProcessToken,
    PROCESS_MITIGATION_POLICY, ProcessChildProcessPolicy, ProcessDynamicCodePolicy, ProcessExtensionPointDisablePolicy,
    ProcessFontDisablePolicy, ProcessImageLoadPolicy, ProcessMitigationOptionsMask, ProcessStrictHandleCheckPolicy,
    ProcessSystemCallDisablePolicy, WaitForSingleObject,
};

use super::{PROBE, PROGRAM, Record, TINY_PICTURES, attempt, expect_end, finish, running};

const CREATION: u64 = 0x1111_0011_1000_1000;
const JOB_LIMITS: u32 = 0x2000 | 0x8 | 0x100 | 0x400;
const UI_LIMITS: u32 = 0xff;
const FOREIGN: usize = 0x10000;

// TODO(guess): Windows sets TEMP and TMP (to the container's own folder) and nothing else in an AppContainer
// process's environment, and keeps the passed LOCALAPPDATA and SystemRoot; the CI's first run showed only that an
// empty block fails (ERROR_ENVVAR_NOT_FOUND, 203); settle: this program's "environment" attempt on the Windows CI,
// which prints the child's variables and their values.
const CHILD_ENVIRONMENT: [&str; 4] = ["LOCALAPPDATA", "SystemRoot", "TEMP", "TMP"];

fn policy(h: HANDLE, which: PROCESS_MITIGATION_POLICY) -> Result<u32, String> {
    let mut v = 0u32;
    if unsafe { GetProcessMitigationPolicy(h, which, (&raw mut v).cast(), size_of::<u32>()) } != 0 {
        return Ok(v);
    }
    let e = unsafe { GetLastError() };
    let mut info = [which as u32, 0];
    let status = unsafe {
        NtQueryInformationProcess(h, ProcessMitigationPolicy, info.as_mut_ptr().cast(), 8, std::ptr::null_mut())
    };
    Err(format!("error {e}, NTSTATUS 0x{:08x}", status as u32))
}

fn runs(h: HANDLE) -> u32 {
    u32::from(unsafe { WaitForSingleObject(h, 0) } == WAIT_TIMEOUT)
}

fn token_info(token: HANDLE, class: i32) -> Result<Vec<u64>, String> {
    let mut buf = vec![0u64; 128];
    let mut n = 0u32;
    if unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), 1024, &mut n) } == 0 {
        return Err(format!("error {}", unsafe { GetLastError() }));
    }
    Ok(buf)
}

fn job_limits(job: HANDLE) -> Result<(JOBOBJECT_EXTENDED_LIMIT_INFORMATION, u32), String> {
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        let p = (&raw mut info).cast();
        if QueryInformationJobObject(job, JobObjectExtendedLimitInformation, p, size, std::ptr::null_mut()) == 0 {
            return Err(format!("its limits: error {}", GetLastError()));
        }
        let mut ui: JOBOBJECT_BASIC_UI_RESTRICTIONS = std::mem::zeroed();
        let size = size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>() as u32;
        let p = (&raw mut ui).cast();
        if QueryInformationJobObject(job, JobObjectBasicUIRestrictions, p, size, std::ptr::null_mut()) == 0 {
            return Err(format!("its UI limits: error {}", GetLastError()));
        }
        Ok((info, ui.UIRestrictionsClass))
    }
}

#[repr(C)]
struct IdList {
    assigned: u32,
    listed: u32,
    ids: [usize; 8],
}

fn job_processes(job: HANDLE) -> Result<Vec<usize>, String> {
    let mut list = IdList { assigned: 0, listed: 0, ids: [0; 8] };
    let size = size_of::<IdList>() as u32;
    let ok = unsafe {
        QueryInformationJobObject(job, JobObjectBasicProcessIdList, (&raw mut list).cast(), size, std::ptr::null_mut())
    };
    if ok == 0 {
        return Err(format!("error {}", unsafe { GetLastError() }));
    }
    Ok(list.ids[..(list.listed as usize).min(8)].to_vec())
}

fn outside(rec: &mut Record) {
    let (mut iso, rest, pictures) = match running(Path::new(PROGRAM)) {
        Ok(r) => r,
        Err(e) => return rec.fail(format!("the decoder's program: {e}")),
    };
    let pid = iso.id();
    let (lines, before, after) = iso.with_process(|p| reads(p.handle().as_raw_handle(), p.job().as_raw_handle(), pid));
    rec.check(
        format!("the decoder ran through the reads (running before: {before}, after: {after})"),
        before == 1 && after == 1,
    );
    for (what, result, ok) in lines {
        match result {
            Ok(v) => rec.check(format!("{what}: {v}"), ok && after == 1),
            Err(e) => rec.fail(format!("{what}: {e} (running before: {before}, after: {after})")),
        }
    }
    match finish(&mut iso, rest, pictures) {
        Ok(n) => rec.check(format!("the test card decoded in the sandbox: {n} pictures"), n == TINY_PICTURES),
        Err(e) => rec.fail(format!("the test card decoded in the sandbox: {e}")),
    }
    match iso.with_process(|p| job_limits(p.job().as_raw_handle())) {
        Ok((info, _)) => rec.info(&format!(
            "peak commit of the test card's decode: {:.1} MiB (limit {} MiB)",
            info.PeakProcessMemoryUsed as f64 / f64::from(1 << 20),
            MEMORY_LIMIT >> 20
        )),
        Err(e) => rec.fail(format!("the decoder's job after the decode: {e}")),
    }
}

type Line = (String, Result<String, String>, bool);

fn reads(h: HANDLE, job: HANDLE, pid: u32) -> (Vec<Line>, u32, u32) {
    let before = runs(h);
    let mut lines: Vec<Line> = Vec::new();
    for (name, which, want) in [
        ("win32k system calls off (from creation)", ProcessSystemCallDisablePolicy, 1),
        ("no dynamic code (from creation)", ProcessDynamicCodePolicy, 1),
        ("extension points off (from creation)", ProcessExtensionPointDisablePolicy, 1),
        ("system fonts only (from creation)", ProcessFontDisablePolicy, 1),
        ("no remote, no low-label images, system images first (from creation)", ProcessImageLoadPolicy, 0b111),
        ("no child process (from creation)", ProcessChildProcessPolicy, 1),
    ] {
        let v = policy(h, which);
        let ok = v.as_ref().is_ok_and(|v| v & want == want);
        lines.push((name.into(), v.map(|v| format!("Ok({v})")), ok));
    }
    let v = policy(h, ProcessStrictHandleCheckPolicy);
    let ok = v == Ok(3);
    lines.push((
        "strict handle checks (set by the child, read while it runs)".into(),
        v.map(|v| format!("Ok({v})")),
        ok,
    ));
    let mut in_job = 0;
    let r = unsafe { IsProcessInJob(h, job, &mut in_job) };
    let ids = job_processes(job);
    let ok = r != 0 && in_job == 1 && ids.as_ref().is_ok_and(|ids| ids == &[pid as usize]);
    let value = ids.map(|ids| format!("{in_job}; its processes: {ids:?}"));
    lines.push(("in the decoder's own job".into(), value, ok));
    let limits = job_limits(job);
    let ok = limits.as_ref().is_ok_and(|(i, ui)| {
        let b = &i.BasicLimitInformation;
        b.LimitFlags == JOB_LIMITS
            && b.ActiveProcessLimit == 1
            && i.ProcessMemoryLimit as u64 == MEMORY_LIMIT
            && *ui == UI_LIMITS
    });
    let value = limits.map(|(i, ui)| {
        let b = &i.BasicLimitInformation;
        let mib = i.ProcessMemoryLimit >> 20;
        format!(
            "limits 0x{:x}, {} process, {mib} MiB a process, UI limits 0x{ui:x}",
            b.LimitFlags, b.ActiveProcessLimit
        )
    });
    lines.push(("the decoder's job".into(), value, ok));
    let mut count = 0u32;
    let r = unsafe { GetProcessHandleCount(h, &mut count) };
    let value = if r != 0 {
        Ok(format!("{count} (a value of 0x{FOREIGN:x} or more is not one of its own)"))
    } else {
        Err(format!("error {}", unsafe { GetLastError() }))
    };
    lines.push(("its handles".into(), value, r != 0 && (count as usize) < FOREIGN / 4));
    token(h, &mut lines);
    (lines, before, runs(h))
}

fn token(h: HANDLE, lines: &mut Vec<Line>) {
    let mut token: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(h, TOKEN_QUERY, &mut token) } == 0 {
        lines.push(("its token".into(), Err(std::io::Error::last_os_error().to_string()), false));
        return;
    }
    let app = token_info(token, TokenIsAppContainer).map(|b| b[0] as u32);
    lines.push(("an AppContainer".into(), app.clone().map(|v| format!("Ok({v})")), app == Ok(1)));
    let caps = token_info(token, TokenCapabilities).map(|b| unsafe { (*b.as_ptr().cast::<TOKEN_GROUPS>()).GroupCount });
    lines.push(("no capabilities".into(), caps.clone().map(|v| format!("Ok({v})")), caps == Ok(0)));
    let level = token_info(token, TokenIntegrityLevel).map(|b| unsafe {
        let sid = (*b.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
        let n = u32::from(*GetSidSubAuthorityCount(sid));
        *GetSidSubAuthority(sid, n.saturating_sub(1))
    });
    lines.push(("low integrity".into(), level.clone().map(|v| format!("Ok(0x{v:x})")), level == Ok(0x1000)));
    unsafe { CloseHandle(token) };
}

fn context(rec: &mut Record) {
    match isolate_os::windows_build() {
        Ok(b) => rec.info(&format!("windows: build {b}")),
        Err(e) => rec.fail(format!("windows: {e}")),
    }
    match isolate_os::outer_job() {
        Ok(j) => rec.info(&format!(
            "this test program's job: in a job: {}, limits: {}; the decoder breaks away: {}",
            u8::from(j.in_job),
            j.limit_names(),
            if j.breaks_away() { "yes" } else { "no" }
        )),
        Err(e) => rec.fail(format!("this test program's job: {e}")),
    }
    let mut mask = [0u64; 2];
    let ok = unsafe {
        let p = mask.as_mut_ptr().cast();
        GetProcessMitigationPolicy(GetCurrentProcess(), ProcessMitigationOptionsMask, p, size_of_val(&mask))
    };
    let missing = CREATION & !mask[0];
    rec.check(
        format!("creation mitigations supported: all of 0x{CREATION:x} (mask 0x{:x}, missing 0x{missing:x})", mask[0]),
        ok != 0 && missing == 0,
    );
    match isolate_os::passed_environment() {
        Ok(vars) => {
            for (name, value) in vars {
                rec.info(&format!("passed {name}={}", value.to_string_lossy()));
            }
        }
        Err(e) => rec.fail(format!("the passed environment: {e}")),
    }
    for program in [PROGRAM, PROBE] {
        match isolate_os::grant_image_access(Path::new(program)) {
            Ok(ImageAccess::AlreadyGranted) => rec.info(&format!("image access: {program}: already granted")),
            Ok(ImageAccess::EntryAdded) => rec.info(&format!("image access: {program}: entry added")),
            Err(e) => rec.fail(format!("image access: {e}")),
        }
    }
    let base = std::env::var_os("ProgramFiles").map_or_else(|| PathBuf::from("C:\\Program Files"), PathBuf::from);
    let dir = base.join(format!("ProjectAvalanche-sandbox-check-{}", std::process::id()));
    let copy = dir.join("avalanche-movie.exe");
    let made = std::fs::create_dir(&dir).and_then(|()| std::fs::write(&copy, std::fs::read(PROGRAM)?));
    let said = match made.map_err(|e| e.to_string()).and_then(|()| isolate_os::container_may_run(&copy)) {
        Ok(true) => "the container may read and run it: yes".to_string(),
        Ok(false) => "the container may read and run it: no".to_string(),
        Err(e) => format!("not checked: {e}"),
    };
    rec.info(&format!("image access in {}: {said}", base.display()));
    let _ = std::fs::remove_dir_all(&dir);
}

fn foreign_pipe() -> Result<(HANDLE, HANDLE), String> {
    let mut held = Vec::new();
    let made = loop {
        let e = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
        if e.is_null() {
            break Err(format!("an event: {}", std::io::Error::last_os_error()));
        }
        held.push(e);
        if e as usize >= FOREIGN {
            let sa = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: std::ptr::null_mut(),
                bInheritHandle: 1,
            };
            let (mut r, mut w): (HANDLE, HANDLE) = (std::ptr::null_mut(), std::ptr::null_mut());
            if unsafe { CreatePipe(&mut r, &mut w, &sa, 0) } == 0 {
                break Err(format!("an inheritable pipe: {}", std::io::Error::last_os_error()));
            }
            break Ok((r, w));
        }
        if held.len() > 2 * FOREIGN / 4 {
            break Err(format!("no handle value of 0x{FOREIGN:x} after {} events", held.len()));
        }
    };
    for e in held {
        unsafe { CloseHandle(e) };
    }
    match made {
        Ok((r, w)) if (w as usize) < FOREIGN => {
            unsafe {
                CloseHandle(r);
                CloseHandle(w);
            }
            Err(format!("the inheritable pipe's handle 0x{:x} is below 0x{FOREIGN:x}", w as usize))
        }
        other => other,
    }
}

fn nested(rec: &mut Record) {
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() || unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } == 0 {
        let e = std::io::Error::last_os_error();
        return rec.info(&format!("a parent in a job without breakaway: not checked: {e}"));
    }
    let said = match running(Path::new(PROGRAM)) {
        Ok((mut iso, rest, pictures)) => match finish(&mut iso, rest, pictures) {
            Ok(n) => format!("ok ({n} pictures)"),
            Err(e) => e,
        },
        Err(e) => e,
    };
    rec.info(&format!("a parent in a job without breakaway: the decoder's job joined: {said}"));
}

pub fn run(rec: &mut Record) {
    context(rec);
    outside(rec);
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let readable = tmp.join("ssx-movie-sandbox-user-file.txt");
    let written = tmp.join("ssx-movie-sandbox-write-attempt");
    let _ = std::fs::remove_file(&written);
    if let Err(e) = std::fs::write(&readable, "the user's own file") {
        return rec.fail(format!("{}: {e}", readable.display()));
    }
    let listener = TcpListener::bind("127.0.0.1:0").and_then(|l| l.set_nonblocking(true).map(|()| l));
    let Ok(listener) = listener else {
        return rec.fail("no listener on 127.0.0.1".into());
    };
    let to = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    let cmd = std::env::var("SystemRoot").map(|r| format!("{r}\\System32\\cmd.exe")).unwrap_or_default();
    let parent = std::process::id().to_string();
    let refusal = ["refused:", "exit code: 98)"];
    let (rd, wr) = (readable.to_string_lossy().into_owned(), written.to_string_lossy().into_owned());
    for (name, arg) in [
        ("open-read", rd.as_str()),
        ("open-write", wr.as_str()),
        ("create-process", cmd.as_str()),
        ("open-parent", parent.as_str()),
        ("dynamic-code", ""),
        ("protect-exec", ""),
    ] {
        expect_end(rec, name, arg, &refusal, &[]);
    }
    expect_end(rec, "win32k", "", &["refused: gdi32.dll not loaded: error ", "exit code: 98)"], &[]);
    let connect = ["socket made", "refused: connect to 127.0.0.1:", "(os error ", "exit code: 98)"];
    expect_end(rec, "connect", &to, &connect, &[]);
    match foreign_pipe() {
        Ok((r, w)) => {
            expect_end(rec, "handle", &(w as usize).to_string(), &["exit code: 0xc0000008"], &["refused:"]);
            let mut avail = 0u32;
            let peeked = unsafe {
                PeekNamedPipe(r, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut())
            };
            rec.check(format!("the parent's other pipe: {avail} bytes reached it"), peeked != 0 && avail == 0);
            unsafe {
                CloseHandle(r);
                CloseHandle(w);
            }
        }
        Err(e) => rec.fail(format!("handle: {e}")),
    }
    let memory =
        ["memory allocation of 8388608 bytes failed", "MiB allocated before the limit", "exit code: 0xc0000409"];
    expect_end(rec, "memory", "", &memory, &[]);
    match attempt("modules", "") {
        Ok(e) if e.contains("exit code: 0)") && e.contains("modules: user32 not loaded, gdi32 not loaded") => {
            rec.check("the child: user32 not loaded, gdi32 not loaded".into(), true);
        }
        Ok(e) => rec.fail(format!("the child's modules: {e}")),
        Err(e) => rec.fail(format!("the child's modules: {e}")),
    }
    environment(rec);
    rec.check(format!("{} not made", written.display()), !written.exists());
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            rec.check(format!("the listener on {to} not reached"), true)
        }
        other => rec.fail(format!("the listener on {to} was reached: {other:?}")),
    }
    nested(rec);
}

fn environment(rec: &mut Record) {
    match attempt("environment", "") {
        Ok(e) => {
            let names = e.split_once("variables: [").and_then(|(_, r)| r.split_once(']')).map(|(n, _)| n);
            let names: Vec<&str> = names.unwrap_or_default().split(' ').filter(|n| !n.is_empty()).collect();
            let foreign: Vec<&&str> =
                names.iter().filter(|n| !CHILD_ENVIRONMENT.iter().any(|a| a.eq_ignore_ascii_case(n))).collect();
            let ok = e.contains("exit code: 0)") && names.len() == CHILD_ENVIRONMENT.len() && foreign.is_empty();
            rec.check(format!("environment: {} variables: [{}]", names.len(), names.join(" ")), ok);
        }
        Err(e) => rec.fail(format!("environment: {e}")),
    }
    for name in CHILD_ENVIRONMENT {
        match attempt("environment", name) {
            Ok(e) => rec.info(&format!("the child's {}", e.split_once("it said: ").map_or(e.as_str(), |(_, v)| v))),
            Err(e) => rec.fail(format!("the child's {name}: {e}")),
        }
    }
}
