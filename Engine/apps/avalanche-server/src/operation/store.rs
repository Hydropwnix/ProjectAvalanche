use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{PoisonError, RwLock, RwLockReadGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use ssx_net::server::Decision;
use ssx_net_ops::{Acl, Ban, BanStore, Mute};
use ssx_net_proto::keys::PlayerKey;

pub const BANS_FILE: &str = "bans.toml";
pub const ACL_FILE: &str = "acl.toml";

pub const NO_BAN_REASON: &str = "banned by the server's operator";
pub const NO_MUTE_REASON: &str = "muted by the server's operator";

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub struct Operation {
    bans_path: PathBuf,
    acl_path: PathBuf,
    bans: RwLock<BanStore>,
    acl: RwLock<Acl>,
    acl_generation: AtomicU64,
}

impl Operation {
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let (bans_path, acl_path) = (data_dir.join(BANS_FILE), data_dir.join(ACL_FILE));
        let bans = BanStore::load(&bans_path)?;
        let acl = Acl::load(&acl_path)?;
        Ok(Self {
            bans_path,
            acl_path,
            bans: RwLock::new(bans),
            acl: RwLock::new(acl),
            acl_generation: AtomicU64::new(0),
        })
    }

    pub fn bans_path(&self) -> &Path {
        &self.bans_path
    }

    pub fn acl_path(&self) -> &Path {
        &self.acl_path
    }

    pub fn bans(&self) -> RwLockReadGuard<'_, BanStore> {
        self.bans.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn acl(&self) -> RwLockReadGuard<'_, Acl> {
        self.acl.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn change_bans<R>(&self, change: impl FnOnce(&mut BanStore) -> Result<R, String>) -> Result<R, String> {
        let mut guard = self.bans.write().unwrap_or_else(PoisonError::into_inner);
        let mut copy = guard.clone();
        let out = change(&mut copy)?;
        copy.save(&self.bans_path).map_err(|e| format!("{} not written: {e}", self.bans_path.display()))?;
        *guard = copy;
        Ok(out)
    }

    pub fn change_acl<R>(&self, change: impl FnOnce(&mut Acl) -> Result<R, String>) -> Result<R, String> {
        let mut guard = self.acl.write().unwrap_or_else(PoisonError::into_inner);
        let mut copy = guard.clone();
        let out = change(&mut copy)?;
        copy.save(&self.acl_path).map_err(|e| format!("{} not written: {e}", self.acl_path.display()))?;
        *guard = copy;
        self.acl_changed();
        Ok(out)
    }

    pub fn reload_bans(&self) -> Result<(usize, usize), String> {
        let fresh = BanStore::load(&self.bans_path)?;
        let counts = (fresh.bans().len(), fresh.mutes().len());
        *self.bans.write().unwrap_or_else(PoisonError::into_inner) = fresh;
        Ok(counts)
    }

    pub fn reload_acl(&self) -> Result<(usize, usize), String> {
        let fresh = Acl::load(&self.acl_path)?;
        let counts = (fresh.groups().len(), fresh.requests().len());
        let mut guard = self.acl.write().unwrap_or_else(PoisonError::into_inner);
        *guard = fresh;
        self.acl_changed();
        Ok(counts)
    }

    pub fn acl_generation(&self) -> u64 {
        self.acl_generation.load(Ordering::Acquire)
    }

    fn acl_changed(&self) {
        self.acl_generation.fetch_add(1, Ordering::AcqRel);
    }

    pub fn ban_of(&self, id: Option<&[u8; 32]>, ip: Option<IpAddr>, now: u64) -> Option<Ban> {
        self.bans().banned(id, ip, now).cloned()
    }

    pub fn mute_of(&self, id: &[u8; 32], now: u64) -> Option<Mute> {
        self.bans().muted(id, now).cloned()
    }

    pub fn expire(&self, now: u64) -> Result<(Vec<Ban>, Vec<Mute>), String> {
        {
            let store = self.bans();
            let any =
                store.bans().iter().any(|b| b.until.passed(now)) || store.mutes().iter().any(|m| m.until.passed(now));
            if !any {
                return Ok((Vec::new(), Vec::new()));
            }
        }
        self.change_bans(|store| Ok(store.expire(now)))
    }
}

pub fn ban_text(ban: &Ban) -> &str {
    if ban.reason.is_empty() { NO_BAN_REASON } else { &ban.reason }
}

pub fn mute_text(mute: &Mute) -> &str {
    if mute.reason.is_empty() { NO_MUTE_REASON } else { &mute.reason }
}

impl ssx_net::server::Operator for Operation {
    fn banned(&self, player: &PlayerKey, address: IpAddr) -> Option<Decision> {
        let id = player.id();
        let ban = self.ban_of(Some(&id.0), Some(address), unix_now())?;
        Some(Decision { reason: ban_text(&ban).to_string(), until: ban.until })
    }

    fn muted(&self, player: &PlayerKey) -> Option<Decision> {
        let mute = self.mute_of(&player.id().0, unix_now())?;
        Some(Decision { reason: mute.reason, until: mute.until })
    }
}
