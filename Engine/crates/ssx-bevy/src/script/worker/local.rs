use std::path::PathBuf;

use ssx_content::PackageSource;
use ssx_script::host::loopback::Loopback;
use ssx_script::host::monitor::StopRecord;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::server::{EndRound, ServerFrame};
use ssx_script::host::{ClientId, HostConfig, Side};

use super::{Worker, clean};

pub(in crate::script) const LOCAL_CLIENT: ClientId = ClientId(1);

const MAX_REFUSED_LINES: usize = 8;

#[derive(Debug, Default)]
pub(in crate::script) struct LocalOut {
    pub running: Option<String>,
    pub end_round: Vec<EndRound>,
}

pub(super) struct Local {
    lb: Loopback,
    package: String,
    client_mod: Option<PathBuf>,
    last_stop: Option<StopRecord>,
}

impl Local {
    pub(super) fn set_catalog(&self, catalog: std::sync::Arc<ssx_script::host::game::Catalog>) {
        self.lb.server.set_catalog(catalog);
    }
}

impl Worker {
    fn server_package(&mut self, name: &str) -> Option<PackageScripts> {
        let roots: Vec<PathBuf> = self.server_packages.iter().chain(self.dev.iter()).cloned().collect();
        for root in roots {
            let opened = PackageSource::open(&root)
                .map_err(|e| e.to_string())
                .and_then(|src| PackageScripts::from_source(&src, Side::Server, self.host.limits()));
            match opened {
                Ok(p) if p.name == name => return Some(p),
                Ok(_) => {}
                Err(e) => self.note(true, format!("scripts: {}: {}", clean(&root.to_string_lossy()), clean(&e))),
            }
        }
        None
    }

    pub(super) fn start_local(&mut self, package: &str, player: &str) {
        if self.local.as_ref().is_some_and(|l| l.package == package) {
            return;
        }
        self.end_local("another level's gamemode");
        let alone = self.word == Some(None);
        if !alone || !self.session.is_empty() || self.held_session.is_some() || self.host.local_player().is_some() {
            let line = format!("scripts: {}'s server scripts do not run locally in a network session", clean(package));
            return self.note(true, line);
        }
        let Some(p) = self.server_package(package) else {
            let line = format!("scripts: no mounted package {} has server scripts: no local host", clean(package));
            return self.note(false, line);
        };
        if p.scripts.is_empty() {
            return self.note(false, format!("scripts: {} has no server scripts: no local host", clean(package)));
        }
        let mut config = HostConfig::new(Side::Server);
        match ssx_files::create_dir_all(&self.server_storage) {
            Ok(()) => config.storage_root = Some(self.server_storage.clone()),
            Err(e) => self.note(true, format!("scripts: the local host's storage: {}", clean(&e.to_string()))),
        }
        let mut lb = match Loopback::new(config) {
            Ok(lb) => lb,
            Err(e) => return self.note(true, format!("scripts: the local host cannot start: {}", clean(&e))),
        };
        if let Some(c) = &self.catalog {
            lb.server.set_catalog(c.clone());
        }
        let n = p.scripts.len();
        if let Err(e) = lb.server.start(p) {
            for (problem, line) in super::outputs(&lb.server, &mut None, "scripts: local server") {
                self.note(problem, line);
            }
            let line = format!("scripts: {}'s server scripts did not start: {}", clean(package), clean(&e));
            return self.note(true, line);
        }
        let (client, _) = match lb.adopt(&self.host, player) {
            Ok(a) => a,
            Err(e) => {
                let line = format!("scripts: the local host did not take the player {}: {}", clean(player), clean(&e));
                return self.note(true, line);
            }
        };
        if client != LOCAL_CLIENT {
            let line = format!("scripts: the local host named its client {} (the game's frames name {})", client.0, 1);
            lb.release_adopted("the local host stopped");
            self.host.leave_session();
            return self.note(true, line);
        }
        let mut l = Local { lb, package: package.to_string(), client_mod: None, last_stop: None };
        if self.host.running().iter().any(|r| r == package) {
            l.client_mod = self.restart_client_mod(package, "the local host names the player");
            if l.client_mod.is_some()
                && let Err(e) = self.host.acknowledge_start(package)
            {
                self.note(true, format!("scripts: {}: no acknowledgement: {}", clean(package), clean(&e)));
            }
        }
        let line = format!(
            "scripts: the local script host runs {}'s {n} server scripts for {}",
            clean(package),
            clean(player)
        );
        self.note(false, line);
        self.collect_local(&mut l);
        self.local = Some(l);
    }

    pub(super) fn end_local(&mut self, reason: &str) {
        let Some(mut l) = self.local.take() else { return };
        l.lb.carry_with(Some(&self.host));
        l.lb.release_adopted(reason);
        for name in l.lb.server.running() {
            l.lb.server.stop(&name, reason);
        }
        self.collect_local(&mut l);
        self.host.leave_session();
        if l.client_mod.is_some() {
            self.restart_client_mod(&l.package, "the local host ended");
        }
        self.note(false, format!("scripts: the local script host stopped: {}", clean(reason)));
    }

    fn restart_client_mod(&mut self, name: &str, why: &str) -> Option<PathBuf> {
        let roots: Vec<PathBuf> = self.packages.iter().chain(self.dev.iter()).cloned().collect();
        for root in roots {
            let Ok(p) = self.scripts_of(&root) else { continue };
            if p.name != name {
                continue;
            }
            self.host.stop(name, why);
            return match self.host.start(p) {
                Ok(()) => {
                    self.note(false, format!("scripts: {} started again ({why})", clean(name)));
                    Some(root)
                }
                Err(e) => {
                    self.note(true, format!("scripts: {} did not start again: {}", clean(name), clean(&e)));
                    None
                }
            };
        }
        None
    }

    pub(super) fn local_before(&mut self, f: ServerFrame) -> Option<LocalOut> {
        let mut l = self.local.take()?;
        l.lb.carry_with(Some(&self.host));
        let orders = match l.lb.server.server_frame(f) {
            Ok(o) => o,
            Err(e) => {
                self.note(true, format!("scripts: the local host's frame was refused: {}", clean(&e)));
                Default::default()
            }
        };
        l.lb.carry_with(Some(&self.host));
        let out = LocalOut { running: Some(l.package.clone()), end_round: orders.end_round };
        self.local = Some(l);
        Some(out)
    }

    pub(super) fn local_after(&mut self) {
        let Some(mut l) = self.local.take() else { return };
        l.lb.carry_with(Some(&self.host));
        self.collect_local(&mut l);
        self.local = Some(l);
    }

    pub(super) fn local_monitor(&self) -> Option<String> {
        let l = self.local.as_ref()?;
        Some(format!("local server ({}):\n{}", l.package, l.lb.server.monitor().render()))
    }

    fn collect_local(&mut self, l: &mut Local) {
        let lines = super::outputs(&l.lb.server, &mut l.last_stop, "scripts: local server");
        for (problem, line) in lines {
            self.note(problem, line);
        }
        let refused = std::mem::take(&mut l.lb.refused);
        for r in refused.iter().take(MAX_REFUSED_LINES) {
            self.note(true, format!("scripts: the local host refused a message: {}", clean(r)));
        }
        if refused.len() > MAX_REFUSED_LINES {
            let more = refused.len() - MAX_REFUSED_LINES;
            self.note(true, format!("scripts: the local host refused {more} more messages"));
        }
    }
}

#[cfg(test)]
pub(in crate::script) fn local_client_is_the_first_admitted() -> bool {
    let Ok(mut lb) = Loopback::new(HostConfig::new(Side::Server)) else { return false };
    let Ok(game) = ssx_script::host::ScriptHost::new(HostConfig::new(Side::Client)) else { return false };
    lb.adopt(&game, "Player").is_ok_and(|(c, _)| c == LOCAL_CLIENT)
}
