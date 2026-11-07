use std::collections::BTreeSet;
use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, sync_channel};

use ssx_assets::content::GameContent;
use ssx_content::package::{Origin, PackageSource};
use ssx_script::host::limits::HostLimits;
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{ScriptHost, Side};

use super::ScriptsHandle;
use super::thread::Job;
use crate::resources::Started;

pub(crate) struct Transaction {
    stop: Vec<String>,
    start: Vec<PackageScripts>,
    restore: Vec<PackageScripts>,
}

fn scripts_of(name: &str, dir: &Path) -> Result<Option<PackageScripts>, String> {
    let src = PackageSource::open_as(dir, Origin::Server).map_err(|e| format!("{name}: {e}"))?;
    let p = PackageScripts::from_source(&src, Side::Server, &HostLimits::SERVER).map_err(|e| format!("{name}: {e}"))?;
    Ok((!p.scripts.is_empty()).then_some(p))
}

fn names(s: &Started) -> Vec<String> {
    std::iter::once(s.loaded.name.clone()).chain(s.loaded.manifest.dependencies.provide.iter().cloned()).collect()
}

pub fn transact(
    handle: &ScriptsHandle,
    content: Option<&GameContent>,
    before: &[Started],
    after: &[Started],
) -> Result<Vec<String>, String> {
    let same = |a: &Started, b: &Started| a.loaded.name == b.loaded.name && a.loaded.dir == b.loaded.dir;
    let kept = |s: &Started, other: &[Started]| other.iter().any(|o| same(s, o));
    let mut touched: BTreeSet<String> = before.iter().filter(|s| !kept(s, after)).flat_map(names).collect();
    touched.extend(after.iter().filter(|s| !kept(s, before)).flat_map(names));
    let requires = |s: &Started, touched: &BTreeSet<String>| {
        s.loaded.manifest.dependencies.require.iter().any(|d| touched.contains(&d.name))
    };
    let mut again: BTreeSet<String> = BTreeSet::new();
    for s in after.iter().filter(|s| kept(s, before)) {
        if requires(s, &touched) {
            again.insert(s.loaded.name.clone());
            touched.extend(names(s));
        }
    }
    let mut t = Transaction { stop: Vec::new(), start: Vec::new(), restore: Vec::new() };
    for s in before.iter().filter(|s| !kept(s, after) || again.contains(&s.loaded.name)) {
        if let Some(p) = scripts_of(&s.loaded.name, &s.loaded.dir)? {
            t.restore.push(p);
        }
    }
    t.stop = t.restore.iter().rev().map(|p| p.name.clone()).collect();
    for s in after.iter().filter(|s| !kept(s, before) || again.contains(&s.loaded.name)) {
        if let Some(p) = scripts_of(&s.loaded.name, &s.loaded.dir)? {
            t.start.push(p);
        }
    }
    if t.stop.is_empty() && t.start.is_empty() {
        return Ok(Vec::new());
    }
    let mut said = Vec::new();
    if let Some(content) = content {
        let (catalog, problems) = super::catalog_of(content);
        said.extend(problems);
        said.push(format!("the scripts' catalogue: {} ids, {} markers", catalog.ids.len(), catalog.markers.len()));
        handle.send(Job::Catalog(std::sync::Arc::new(catalog)))?;
    }
    let (tx, rx) = sync_channel(0);
    handle.send(Job::Transact(t, tx))?;
    let mut ran = match rx.recv_timeout(super::ANSWER_BOUND) {
        Ok(r) => r?,
        Err(RecvTimeoutError::Timeout) => {
            let s = super::ANSWER_BOUND.as_secs();
            return Err(format!("the scripts did not take the change within {s} s (the scripts' thread undoes it)"));
        }
        Err(RecvTimeoutError::Disconnected) => return Err("the scripts' thread ended during the change".into()),
    };
    said.append(&mut ran);
    Ok(said)
}

pub(super) struct Undo {
    started: Vec<String>,
    restore: Vec<PackageScripts>,
}

impl Undo {
    pub(super) fn run(self, host: &ScriptHost) -> Vec<String> {
        let mut said = Vec::new();
        for n in self.started.iter().rev() {
            if host.stop(n, "its resource change was given up") {
                said.push(format!("{n}: its server scripts stopped (the change was given up)"));
            }
        }
        for p in self.restore {
            let n = p.name.clone();
            match host.start(p) {
                Ok(()) => said.push(format!("{n}: its server scripts started again (the change was given up)")),
                Err(e) => said.push(format!("{n} does not start again: {e}")),
            }
        }
        said
    }
}

impl Transaction {
    #[cfg(test)]
    pub(super) fn starting(start: Vec<PackageScripts>) -> Self {
        Self { stop: Vec::new(), start, restore: Vec::new() }
    }

    pub(super) fn run(self, host: &ScriptHost) -> Result<(Vec<String>, Undo), String> {
        let mut said = Vec::new();
        for name in &self.stop {
            if host.stop(name, "its resource stopped or changed") {
                said.push(format!("{name}: its server scripts stopped"));
            }
        }
        let mut started: Vec<String> = Vec::new();
        for p in self.start {
            let name = p.name.clone();
            if let Err(e) = host.start(p) {
                for n in started.iter().rev() {
                    host.stop(n, "its resource set was refused");
                }
                let mut again = Vec::new();
                for p in self.restore {
                    let n = p.name.clone();
                    if let Err(e) = host.start(p) {
                        again.push(format!("{n} does not start again: {e}"));
                    }
                }
                let again = if again.is_empty() { String::new() } else { format!("; {}", again.join("; ")) };
                return Err(format!("{name}: its server scripts do not start: {e}{again}"));
            }
            said.push(format!("{name}: its server scripts started"));
            started.push(name);
        }
        Ok((said, Undo { started, restore: self.restore }))
    }
}
