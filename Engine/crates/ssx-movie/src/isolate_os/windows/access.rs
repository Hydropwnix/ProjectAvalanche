use std::ffi::{OsStr, c_void};
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::path::Path;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, LUID, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    AUTHZ_ACCESS_REPLY, AUTHZ_ACCESS_REQUEST, AUTHZ_CLIENT_CONTEXT_HANDLE, AUTHZ_RESOURCE_MANAGER_HANDLE,
    AUTHZ_RM_FLAG_NO_AUDIT, AuthzAccessCheck, AuthzFreeContext, AuthzFreeResourceManager,
    AuthzInitializeContextFromToken, AuthzInitializeResourceManager, AuthzSetAppContainerInformation,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, SE_FILE_OBJECT, SetEntriesInAclW,
    SetNamedSecurityInfoW, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::Isolation::{CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName};
use windows_sys::Win32::Security::{
    ACL, CreateWellKnownSid, DACL_SECURITY_INFORMATION, DuplicateTokenEx, FreeSid, GROUP_SECURITY_INFORMATION,
    GetLengthSid, LABEL_SECURITY_INFORMATION, NO_INHERITANCE, OWNER_SECURITY_INFORMATION, PSID, SECURITY_MAX_SID_SIZE,
    SID_AND_ATTRIBUTES, SecurityImpersonation, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_DUPLICATE,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenImpersonation, TokenIntegrityLevel, WinLowLabelSid,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_EXECUTE, FILE_GENERIC_READ};
use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::{CONTAINER, last, owned, wide};

const WANT: u32 = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;

pub(super) struct Sid(pub(super) PSID);

impl Drop for Sid {
    fn drop(&mut self) {
        unsafe { FreeSid(self.0) };
    }
}

pub(super) fn container() -> Result<Sid, String> {
    let name = wide(OsStr::new(CONTAINER));
    let shown = wide(OsStr::new("Project Avalanche movie decoder"));
    let mut sid: PSID = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(name.as_ptr(), shown.as_ptr(), shown.as_ptr(), std::ptr::null(), 0, &mut sid)
    };
    if hr >= 0 && !sid.is_null() {
        return Ok(Sid(sid));
    }
    if hr != (0x8007_0000u32 | ERROR_ALREADY_EXISTS) as i32 {
        let why =
            if hr as u32 == 0x8007_0005 { ": access denied (a system policy may forbid AppContainers)" } else { "" };
        return Err(format!("the decoder's AppContainer: HRESULT 0x{:08x}{why}", hr as u32));
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

struct ResourceManager(AUTHZ_RESOURCE_MANAGER_HANDLE);

impl Drop for ResourceManager {
    fn drop(&mut self) {
        unsafe { AuthzFreeResourceManager(self.0) };
    }
}

struct Context(AUTHZ_CLIENT_CONTEXT_HANDLE);

impl Drop for Context {
    fn drop(&mut self) {
        unsafe { AuthzFreeContext(self.0) };
    }
}

fn low_token() -> Result<OwnedHandle, String> {
    let mut own: HANDLE = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut own) } == 0 {
        return Err(last("this program's token"));
    }
    let own = owned(own, "this program's token")?;
    let mut dup: HANDLE = std::ptr::null_mut();
    let ok = unsafe {
        DuplicateTokenEx(
            own.as_raw_handle(),
            TOKEN_QUERY | TOKEN_ADJUST_DEFAULT,
            std::ptr::null(),
            SecurityImpersonation,
            TokenImpersonation,
            &mut dup,
        )
    };
    if ok == 0 {
        return Err(last("a copy of this program's token"));
    }
    let dup = owned(dup, "a copy of this program's token")?;
    let mut sid = [0u32; SECURITY_MAX_SID_SIZE as usize / 4];
    let mut n = SECURITY_MAX_SID_SIZE;
    if unsafe { CreateWellKnownSid(WinLowLabelSid, std::ptr::null_mut(), sid.as_mut_ptr().cast(), &mut n) } == 0 {
        return Err(last("the low integrity SID"));
    }
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES { Sid: sid.as_mut_ptr().cast(), Attributes: SE_GROUP_INTEGRITY as u32 },
    };
    let ok = unsafe {
        let size = size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(label.Label.Sid);
        SetTokenInformation(dup.as_raw_handle(), TokenIntegrityLevel, (&raw const label).cast(), size)
    };
    if ok == 0 {
        return Err(last("a copy of this program's token at low integrity"));
    }
    Ok(dup)
}

fn may_run(path: &[u16], sid: &Sid) -> Result<bool, String> {
    let (mut dacl, mut sd): (*mut ACL, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
    let what = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let e = unsafe {
        GetNamedSecurityInfoW(
            path.as_ptr(),
            SE_FILE_OBJECT,
            what | LABEL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    let _sd = Local(sd);
    if e != ERROR_SUCCESS {
        return Err(format!("the program file's security: error {e}"));
    }
    if dacl.is_null() {
        return Ok(true);
    }
    let token = low_token()?;
    let mut rm: AUTHZ_RESOURCE_MANAGER_HANDLE = std::ptr::null_mut();
    let ok =
        unsafe { AuthzInitializeResourceManager(AUTHZ_RM_FLAG_NO_AUDIT, None, None, None, std::ptr::null(), &mut rm) };
    if ok == 0 {
        return Err(last("the access check's resource manager"));
    }
    let rm = ResourceManager(rm);
    let mut ctx: AUTHZ_CLIENT_CONTEXT_HANDLE = std::ptr::null_mut();
    let none = LUID { LowPart: 0, HighPart: 0 };
    let ok = unsafe {
        AuthzInitializeContextFromToken(
            0,
            token.as_raw_handle(),
            rm.0,
            std::ptr::null(),
            none,
            std::ptr::null(),
            &mut ctx,
        )
    };
    if ok == 0 {
        return Err(last("the access check's context"));
    }
    let ctx = Context(ctx);
    if unsafe { AuthzSetAppContainerInformation(ctx.0, sid.0, 0, std::ptr::null()) } == 0 {
        return Err(last("the access check's AppContainer"));
    }
    let request = AUTHZ_ACCESS_REQUEST {
        DesiredAccess: WANT,
        PrincipalSelfSid: std::ptr::null_mut(),
        ObjectTypeList: std::ptr::null_mut(),
        ObjectTypeListLength: 0,
        OptionalArguments: std::ptr::null_mut(),
    };
    let (mut granted, mut error) = (0u32, 0u32);
    let mut reply = AUTHZ_ACCESS_REPLY {
        ResultListLength: 1,
        GrantedAccessMask: &mut granted,
        SaclEvaluationResults: std::ptr::null_mut(),
        Error: &mut error,
    };
    // TODO(guess): AuthZ's access check with the container's SID (AuthzSetAppContainerInformation) and a low
    // integrity copy of the user's token decides as Windows does for the container's process; it is the documented
    // AuthZ call for AppContainers (Windows 8+), Chromium checks with a lowbox token instead (NtCreateLowBoxToken, not
    // in windows-sys 0.61.2); not run on Windows yet; settle: the Windows CI lines "info     image access: …" followed
    // by the child's start, and "info     image access in C:\Program Files: …" (a folder the runner's user did not
    // make).
    // SAFETY: the context, request, descriptor and reply are valid and live for the call; one result.
    let ok = unsafe {
        AuthzAccessCheck(
            0,
            ctx.0,
            &request,
            std::ptr::null_mut(),
            sd,
            std::ptr::null(),
            0,
            &mut reply,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last("the access check"));
    }
    Ok(error == ERROR_SUCCESS && granted & WANT == WANT)
}

fn add_entry(path: &[u16], sid: &Sid) -> Result<(), String> {
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
    let _sd = Local(sd);
    if e != ERROR_SUCCESS {
        return Err(format!("its DACL: error {e}"));
    }
    let entry = EXPLICIT_ACCESS_W {
        grfAccessPermissions: WANT,
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
        return Err(format!("its DACL with the entry: error {e}"));
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
        return Err(format!("its DACL cannot be changed: error {e}"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageAccess {
    AlreadyGranted,
    EntryAdded,
}

pub fn grant_image_access(exe: &Path) -> Result<ImageAccess, String> {
    ensure(exe, &container()?)
}

pub(super) fn ensure(exe: &Path, sid: &Sid) -> Result<ImageAccess, String> {
    let path = wide(exe.as_os_str());
    if may_run(&path, sid)? {
        return Ok(ImageAccess::AlreadyGranted);
    }
    let refusal = |why: &str| {
        let folder = exe.parent().map(|f| f.display().to_string()).unwrap_or_default();
        format!(
            "the movie decoder's program {} cannot be read and run by its sandbox (the AppContainer {CONTAINER}), \
             and that cannot be granted ({why}): Project Avalanche needs a folder its user may change (it is in \
             {folder})",
            exe.display()
        )
    };
    add_entry(&path, sid).map_err(|e| refusal(&e))?;
    if !may_run(&path, sid)? {
        return Err(refusal("an entry for it was added, and the access check still refuses"));
    }
    Ok(ImageAccess::EntryAdded)
}

pub fn container_may_run(path: &Path) -> Result<bool, String> {
    may_run(&wide(path.as_os_str()), &container()?)
}
