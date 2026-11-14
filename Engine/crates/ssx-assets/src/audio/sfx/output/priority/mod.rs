#[cfg(target_os = "linux")]
mod dbus;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheduling {
    Normal,
    Audio,
}

impl Scheduling {
    pub(super) fn or_env(self) -> Self {
        match std::env::var("SSX_AUDIO_PRIORITY").as_deref() {
            Ok("normal") if self == Scheduling::Audio => {
                ssx_core::eprintln!("audio: the mixer thread keeps its normal priority (SSX_AUDIO_PRIORITY=normal)");
                Scheduling::Normal
            }
            _ => self,
        }
    }
}

pub(super) struct Promotion {
    #[cfg(target_os = "linux")]
    grant: Option<linux::Grant>,
    #[cfg(windows)]
    _mmcss: Option<windows::Mmcss>,
}

impl Promotion {
    pub(super) fn start(scheduling: Scheduling) -> Self {
        let audio = scheduling == Scheduling::Audio;
        Self {
            #[cfg(target_os = "linux")]
            grant: audio.then(linux::promote).flatten(),
            #[cfg(windows)]
            _mmcss: audio.then(windows::promote).flatten(),
        }
    }

    pub(super) fn blocked(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(grant) = &mut self.grant {
            grant.blocked();
        }
    }

    pub(super) fn check(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(grant) = &mut self.grant {
            grant.check();
        }
    }
}
