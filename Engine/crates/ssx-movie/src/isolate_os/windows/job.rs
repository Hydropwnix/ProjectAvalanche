use std::os::windows::io::{AsRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT_ACTIVE_PROCESS, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
    JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_JOB_TIME,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_LIMIT_PROCESS_TIME,
    JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS,
    JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
    JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectBasicProcessIdList,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, QueryInformationJobObject,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::super::MEMORY_LIMIT;
use super::{last, owned};

pub(super) const LIMITS: u32 = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
    | JOB_OBJECT_LIMIT_PROCESS_MEMORY
    | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION;

pub(super) const UI_LIMITS: u32 = JOB_OBJECT_UILIMIT_DESKTOP
    | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
    | JOB_OBJECT_UILIMIT_EXITWINDOWS
    | JOB_OBJECT_UILIMIT_GLOBALATOMS
    | JOB_OBJECT_UILIMIT_HANDLES
    | JOB_OBJECT_UILIMIT_READCLIPBOARD
    | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
    | JOB_OBJECT_UILIMIT_WRITECLIPBOARD;

pub(super) fn decoder_job() -> Result<OwnedHandle, String> {
    let job = owned(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) }, "the decoder's job")?;
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    info.BasicLimitInformation.LimitFlags = LIMITS;
    info.BasicLimitInformation.ActiveProcessLimit = 1;
    // TODO(guess): the committed memory of the child on Windows is within the Linux measurement of its data (VmData,
    // 176 MiB as the programs ask, 490 MiB at 16 threads and 16 frames in flight) and so within MEMORY_LIMIT (768 MiB);
    // not measured on Windows (rav1d's buffers through the system heap, thread stacks committed as they grow); settle:
    // the peak commit of the child (PeakProcessMemoryUsed of the job's extended information) over the proof inputs on
    // Windows.
    info.ProcessMemoryLimit = usize::try_from(MEMORY_LIMIT).map_err(|_| "the memory limit")?;
    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS { UIRestrictionsClass: UI_LIMITS };
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OuterJob {
    pub in_job: bool,
    pub limits: u32,
}

impl OuterJob {
    pub fn breaks_away(&self) -> bool {
        self.in_job && self.limits & JOB_OBJECT_LIMIT_BREAKAWAY_OK != 0
    }

    pub fn limit_names(&self) -> String {
        let names: Vec<&str> = [
            (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, "KILL_ON_JOB_CLOSE"),
            (JOB_OBJECT_LIMIT_BREAKAWAY_OK, "BREAKAWAY_OK"),
            (JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, "SILENT_BREAKAWAY_OK"),
            (JOB_OBJECT_LIMIT_ACTIVE_PROCESS, "ACTIVE_PROCESS"),
            (JOB_OBJECT_LIMIT_PROCESS_MEMORY, "PROCESS_MEMORY"),
            (JOB_OBJECT_LIMIT_JOB_MEMORY, "JOB_MEMORY"),
            (JOB_OBJECT_LIMIT_PROCESS_TIME, "PROCESS_TIME"),
            (JOB_OBJECT_LIMIT_JOB_TIME, "JOB_TIME"),
            (JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION, "DIE_ON_UNHANDLED_EXCEPTION"),
        ]
        .iter()
        .filter(|(bit, _)| self.limits & bit != 0)
        .map(|(_, name)| *name)
        .collect();
        format!("0x{:x} [{}]", self.limits, names.join(" "))
    }
}

pub fn outer_job() -> Result<OuterJob, String> {
    let mut in_job = 0;
    if unsafe { IsProcessInJob(GetCurrentProcess(), std::ptr::null_mut(), &mut in_job) } == 0 {
        return Err(last("this program's job"));
    }
    if in_job == 0 {
        return Ok(OuterJob { in_job: false, limits: 0 });
    }
    let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    let size = size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
    let ok = unsafe {
        QueryInformationJobObject(
            std::ptr::null_mut(),
            JobObjectExtendedLimitInformation,
            (&raw mut info).cast(),
            size,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last("this program's job's limits"));
    }
    Ok(OuterJob { in_job: true, limits: info.BasicLimitInformation.LimitFlags })
}

#[repr(C)]
struct IdList {
    assigned: u32,
    listed: u32,
    ids: [usize; 8],
}

pub(super) fn process_ids(job: HANDLE) -> Result<(u32, Vec<usize>), String> {
    let mut list = IdList { assigned: 0, listed: 0, ids: [0; 8] };
    let ok = unsafe {
        QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            (&raw mut list).cast(),
            size_of::<IdList>() as u32,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(last("the decoder's job's processes"));
    }
    let listed = (list.listed as usize).min(list.ids.len());
    Ok((list.assigned, list.ids[..listed].to_vec()))
}
