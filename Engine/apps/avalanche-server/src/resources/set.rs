use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use ssx_content::deps::start_order;
use ssx_content::manifest::Manifest;

use super::load::{Loaded, load, locate, stamp};
use super::{Context, ResourceCommand};

#[derive(Clone, Debug)]
pub struct Started {
    pub loaded: Loaded,
    pub explicit: bool,
}

#[derive(Debug)]
pub struct Outcome {
    pub running: Vec<Started>,
    pub removed: Vec<PathBuf>,
    pub said: Vec<String>,
    pub changed: bool,
}

fn provider<'a>(running: &'a [Started], name: &str) -> Option<&'a Started> {
    running
        .iter()
        .find(|s| s.loaded.name == name)
        .or_else(|| running.iter().find(|s| s.loaded.manifest.dependencies.provide.iter().any(|p| p == name)))
}

fn dependants(running: &[Started], name: &str) -> Vec<String> {
    let Some(target) = running.iter().find(|s| s.loaded.name == name) else { return Vec::new() };
    let names: BTreeSet<&str> =
        std::iter::once(name).chain(target.loaded.manifest.dependencies.provide.iter().map(String::as_str)).collect();
    running
        .iter()
        .filter(|s| s.loaded.name != name)
        .filter(|s| s.loaded.manifest.dependencies.require.iter().any(|d| names.contains(d.name.as_str())))
        .map(|s| s.loaded.name.clone())
        .collect()
}

fn ordered(set: Vec<Started>) -> Result<Vec<Started>, String> {
    if set.len() > ssx_content::delivery::MAX_RESOURCES {
        return Err(format!("{} resources, a server runs at most {}", set.len(), ssx_content::delivery::MAX_RESOURCES));
    }
    let bytes: u64 = set.iter().map(|s| s.loaded.bytes()).sum();
    if bytes > ssx_content::delivery::MAX_SERVER_BYTES {
        return Err(format!(
            "{bytes} bytes of files, a server sends at most {}",
            ssx_content::delivery::MAX_SERVER_BYTES
        ));
    }
    let manifests: Vec<Manifest> = set.iter().map(|s| s.loaded.manifest.clone()).collect();
    let order = start_order(&manifests)
        .map_err(|errors| errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))?;
    let mut slots: Vec<Option<Started>> = set.into_iter().map(Some).collect();
    Ok(order.into_iter().filter_map(|i| slots[i].take()).collect())
}

pub fn groups(running: &[Started]) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    for s in running {
        let g = s
            .loaded
            .manifest
            .dependencies
            .require
            .iter()
            .filter_map(|d| provider(running, &d.name))
            .map(|p| out.get(&p.loaded.name).copied().unwrap_or(0) + 1);
        out.insert(s.loaded.name.clone(), g.max().unwrap_or(0));
    }
    out
}

fn read_with_requirements(ctx: &Context, running: &[Started], name: &str) -> Result<Vec<Loaded>, String> {
    let mut read: Vec<Loaded> = Vec::new();
    let mut queue = vec![name.to_string()];
    let result = (|| {
        while let Some(n) = queue.pop() {
            if read.iter().any(|l| l.name == n) {
                continue;
            }
            if read.len() >= ssx_content::delivery::MAX_RESOURCES {
                return Err(format!("{name}: more than {} resources to start", ssx_content::delivery::MAX_RESOURCES));
            }
            let l = load(&ctx.folder, &n, &ctx.state, ctx.next_seq(), &ctx.store, &|h, s| ctx.disc_file(h, s))?;
            for d in &l.manifest.dependencies.require {
                let met = provider(running, &d.name).is_some()
                    || read.iter().any(|r| r.name == d.name || r.manifest.dependencies.provide.contains(&d.name));
                if !met && locate(&ctx.folder, &d.name).is_ok() {
                    queue.push(d.name.clone());
                }
            }
            read.push(l);
        }
        Ok(())
    })();
    match result {
        Ok(()) => Ok(read),
        Err(e) => {
            remove(read.iter().map(|l| l.dir.clone()));
            Err(e)
        }
    }
}

fn remove(dirs: impl Iterator<Item = PathBuf>) {
    for d in dirs {
        let _ = std::fs::remove_dir_all(d);
    }
}

pub fn run(ctx: &Context, command: &ResourceCommand, running: Vec<Started>) -> Result<Outcome, String> {
    let is_running = |n: &str| running.iter().any(|s| s.loaded.name == n);
    match command {
        ResourceCommand::Ensure(n) if is_running(n) => run(ctx, &ResourceCommand::Restart(n.clone()), running),
        ResourceCommand::Ensure(n) => run(ctx, &ResourceCommand::Start(n.clone()), running),
        ResourceCommand::Start(n) => {
            if is_running(n) {
                return Err(format!("{n} is running (restart reads it again)"));
            }
            let read = read_with_requirements(ctx, &running, n)?;
            let dirs: Vec<PathBuf> = read.iter().map(|l| l.dir.clone()).collect();
            let names: Vec<String> =
                read.iter().map(|l| format!("{} {}", l.name, l.manifest.resource.version)).collect();
            let mut set = running;
            set.extend(read.into_iter().map(|l| {
                let explicit = l.name == *n;
                Started { loaded: l, explicit }
            }));
            match ordered(set) {
                Ok(running) => Ok(Outcome {
                    running,
                    removed: Vec::new(),
                    said: vec![format!("started {}", names.join(", "))],
                    changed: true,
                }),
                Err(e) => {
                    remove(dirs.into_iter());
                    Err(format!("{n} not started: {e}"))
                }
            }
        }
        ResourceCommand::Restart(n) => {
            let Some(i) = running.iter().position(|s| s.loaded.name == *n) else {
                return Err(format!("{n} is not running"));
            };
            let l = load(&ctx.folder, n, &ctx.state, ctx.next_seq(), &ctx.store, &|h, s| ctx.disc_file(h, s))
                .map_err(|e| format!("{n} not restarted (the running one stays): {e}"))?;
            let new_dir = l.dir.clone();
            let version = l.manifest.resource.version;
            let mut set = running;
            let explicit = set[i].explicit;
            let old = std::mem::replace(&mut set[i], Started { loaded: l, explicit });
            match ordered(set) {
                Ok(running) => Ok(Outcome {
                    running,
                    removed: vec![old.loaded.dir],
                    said: vec![format!("restarted {n} {version}")],
                    changed: true,
                }),
                Err(e) => {
                    remove(std::iter::once(new_dir));
                    Err(format!("{n} not restarted (the running one stays): {e}"))
                }
            }
        }
        ResourceCommand::Stop(n) => {
            if !is_running(n) {
                return Err(format!("{n} is not running"));
            }
            let mut gone: BTreeSet<String> = BTreeSet::from([n.clone()]);
            let mut queue = vec![n.clone()];
            while let Some(x) = queue.pop() {
                for d in dependants(&running, &x) {
                    if gone.insert(d.clone()) {
                        queue.push(d);
                    }
                }
            }
            let mut set: Vec<Started> = running.iter().filter(|s| !gone.contains(&s.loaded.name)).cloned().collect();
            loop {
                let unneeded = set.iter().position(|s| !s.explicit && dependants(&set, &s.loaded.name).is_empty());
                let Some(i) = unneeded else { break };
                gone.insert(set.remove(i).loaded.name);
            }
            let removed =
                running.iter().filter(|s| gone.contains(&s.loaded.name)).map(|s| s.loaded.dir.clone()).collect();
            let said = vec![format!("stopped {}", gone.iter().cloned().collect::<Vec<_>>().join(", "))];
            Ok(Outcome { running: set, removed, said, changed: true })
        }
        ResourceCommand::Refresh => {
            let mut said = Vec::new();
            for s in &running {
                let n = &s.loaded.name;
                let now = locate(&ctx.folder, n).and_then(|(p, packed)| stamp(&p, packed));
                match now {
                    Ok(stamp) if stamp == s.loaded.source => {}
                    Ok(_) => said.push(format!("{n}: changed on disk (restart {n} sends the new version)")),
                    Err(e) => said.push(format!("{n}: {e} (it keeps running as read)")),
                }
            }
            let found = super::scan(&ctx.folder)?;
            let new: Vec<&String> = found.iter().filter(|n| !is_running(n)).collect();
            said.push(format!(
                "{} resources in {}; not running: {}",
                found.len(),
                ctx.folder.display(),
                if new.is_empty() {
                    "none".to_string()
                } else {
                    new.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
                }
            ));
            Ok(Outcome { running, removed: Vec::new(), said, changed: false })
        }
    }
}
