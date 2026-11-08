use std::collections::BTreeSet;
use std::path::Path;

use avalanche_server::data::guard::{self, Configured};

use crate::operation::{Ops, ops_with_data, ran};

const LIST: &str = "my-blocklist.txt";

#[test]
fn a_runs_data_folder_passes_the_guard() {
    let o = ops_with_data(
        |c| c.discovery.blocklist = Some(LIST.into()),
        "",
        |data, _| std::fs::write(data.join(LIST), "Project Avalanche blocked resources v1\n").expect("the list"),
    )
    .expect("the server starts");
    ran(&o, "ban ip:192.0.2.7 2h a ban the run writes");
    let Ops { server, data, .. } = o;
    server.stop();
    server.join().expect("the server stops");
    let mut written: Vec<String> = std::fs::read_dir(&data)
        .expect("the data folder")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    for entry in ["acl.toml", "bans.toml", "identity.key", "netcode.key", "resource-state", "script-data", LIST] {
        assert!(written.iter().any(|w| w == entry), "{entry} after the run: {written:?}");
    }
    let configured = Configured {
        resources: Path::new("resources"),
        rules: Some(Path::new("big")),
        blocklist: Some(Path::new(LIST)),
    };
    guard::prepare(&data, &BTreeSet::new(), &configured).unwrap_or_else(|e| panic!("{e} ({written:?})"));
    let e = guard::prepare(&data, &BTreeSet::new(), &Configured::resources(Path::new("resources"))).unwrap_err();
    assert!(e.contains(&format!("not a server's ({LIST})")), "{e}");
    let _ = std::fs::remove_dir_all(&data);
}
