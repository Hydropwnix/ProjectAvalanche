use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use ssx_content::manifest::{Files, Scripts};

use super::*;
use crate::host::limits::Budgets;
use crate::host::package::PackageScripts;
use crate::host::{HostConfig, Side};

struct Open(AtomicU64);

impl Rights for Open {
    fn check(&self, _: Holder<'_>, right: &str) -> Result<Decision, String> {
        let allowed = right.strip_prefix("function.").is_some_and(|f| !PRIVILEGED.contains(&f));
        Ok(Decision { allowed, why: "the defaults".into() })
    }

    fn generation(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    fn reason(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn a_call_costs_one_lookup() {
    let h = ScriptHost::new(HostConfig::new(Side::Server)).unwrap();
    let scripts = Scripts { client: vec![], server: vec!["server/*.luau".into()], shared: vec![] };
    let files = [("server/main.luau".to_string(), "x = 1".to_string())];
    let limits = HostConfig::new(Side::Server).limits;
    let pkg = PackageScripts::from_memory(
        "bench",
        Side::Server,
        &scripts,
        &Files::default(),
        toml::Table::new(),
        &files,
        &limits,
    )
    .unwrap();
    h.start(pkg).unwrap();
    let rights = Arc::new(Open(AtomicU64::new(0)));
    h.set_rights(rights.clone());
    assert_eq!(h.rights_counts().resolved, 1, "resolved when the rights were set");
    let index = crate::api::server_calls().iter().position(|s| s.name == "getRootElement").unwrap();
    const N: u32 = 1_000_000;
    let t = Instant::now();
    {
        let mut w = h.shared.lock();
        for _ in 0..N {
            black_box(call_allowed(&mut w, black_box("bench"), black_box(index))).unwrap();
        }
    }
    let lookup = t.elapsed() / N;
    let t = Instant::now();
    for _ in 0..N {
        let mut w = h.shared.lock();
        black_box(call_allowed(&mut w, black_box("bench"), black_box(index))).unwrap();
    }
    let locked = t.elapsed() / N;
    assert_eq!(h.rights_counts().resolved, 1, "no call resolved the set again");
    const M: u32 = 1000;
    let t = Instant::now();
    for _ in 0..M {
        rights.0.fetch_add(1, Ordering::AcqRel);
        let mut w = h.shared.lock();
        black_box(call_allowed(&mut w, "bench", index)).unwrap();
    }
    let resolve = t.elapsed() / M;
    assert_eq!(h.rights_counts().resolved, 1 + u64::from(M), "each move resolved the set once");
    println!(
        "call check: {lookup:?} a lookup, {locked:?} with the host's lock taken; a set of {} calls resolved again \
         {resolve:?} (this fake's checks; the server's ACL in avalanche-server's scripts::rights tests); the server's \
         tick budget {:?}",
        call_rights().len(),
        Budgets::SERVER.tick
    );
}
