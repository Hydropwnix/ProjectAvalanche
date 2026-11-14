use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadPriority,
    THREAD_PRIORITY_TIME_CRITICAL,
};

pub(super) struct Mmcss(HANDLE);

impl Drop for Mmcss {
    fn drop(&mut self) {
        if unsafe { AvRevertMmThreadCharacteristics(self.0) } == 0 {
            ssx_core::eprintln!("audio: the mixer thread's MMCSS task: {}", std::io::Error::last_os_error());
        }
    }
}

// PORT(choice): the MMCSS task "Pro Audio" (the system's own task for low-latency audio; its scheduling category,
// priority and clock rate are the system's, HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Multimedia\SystemProfile\
// Tasks) on top of THREAD_PRIORITY_TIME_CRITICAL; the console runs its Dac thread at EA::Thread priority 15 on a
// hardware thread of its own (System +0x14E, XSetThreadProcessor 0x82351550), which a PC has no counterpart of.
pub(super) fn promote() -> Option<Mmcss> {
    if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL) } == 0 {
        ssx_core::eprintln!("audio: the mixer thread's priority: {}", std::io::Error::last_os_error());
    }
    let task: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
    let mut index = 0u32;
    let handle = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut index) };
    if handle.is_null() {
        let e = std::io::Error::last_os_error();
        ssx_core::eprintln!("audio: the mixer thread keeps THREAD_PRIORITY_TIME_CRITICAL (MMCSS \"Pro Audio\": {e})");
        return None;
    }
    ssx_core::eprintln!("audio: the mixer thread runs as the MMCSS task \"Pro Audio\" (task index {index})");
    Some(Mmcss(handle))
}
