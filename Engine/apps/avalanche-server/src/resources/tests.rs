use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use ssx_content::ContentHash;
use ssx_content::delivery::{FileRole, FileStore};

use super::set::{Started, groups, run};
use super::{Context, ResourceCommand, Settled, config::Mods, publish};

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(tag: &str) -> Folder {
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ssx-resources-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Folder(dir)
}

fn package(root: &Path, name: &str, version: &str, extra: &str, files: &[(&str, &[u8])]) {
    let dir = root.join("resources").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = format!("[resource]\nname = \"{name}\"\nversion = \"{version}\"\napi = 1\n{extra}");
    std::fs::write(dir.join("resource.toml"), manifest).unwrap();
    for (path, bytes) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
}

fn context(root: &Path, data_index: HashMap<(u64, ContentHash), String>) -> Context {
    let state = root.join("state");
    Context {
        folder: root.join("resources"),
        store: FileStore::open(&state.join("store")).unwrap(),
        state,
        seq: AtomicU64::new(0),
        data_index,
        ids: None,
        mods: Mods::default(),
        mirrors: Vec::new(),
        settled: Settled::default(),
        blocked: Default::default(),
        scripts: None,
    }
}

fn names(running: &[Started]) -> Vec<(String, bool)> {
    running.iter().map(|s| (s.loaded.name.clone(), s.explicit)).collect()
}

fn start(ctx: &Context, running: Vec<Started>, name: &str) -> Result<Vec<Started>, String> {
    run(ctx, &ResourceCommand::Start(name.into()), running).map(|o| o.running)
}

fn installed(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join("state/installed"))
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[test]
fn dependencies_start_first_within_their_ranges() {
    let t = folder("deps");
    package(&t.0, "base", "1.2.0", "", &[("data/a.txt", b"base's file, long enough to be compared at all......")]);
    package(&t.0, "mid", "1.0.0", "[dependencies]\nrequire = [\"base >= 1.0, < 2\"]\n", &[("data/m.txt", b"m")]);
    package(&t.0, "top", "0.1.0", "[dependencies]\nrequire = [\"mid >= 1.0\"]\n", &[]);
    package(&t.0, "old", "0.1.0", "[dependencies]\nrequire = [\"base >= 2\"]\n", &[]);
    let ctx = context(&t.0, HashMap::new());
    let running = start(&ctx, Vec::new(), "top").unwrap();
    assert_eq!(names(&running), [("base".into(), false), ("mid".into(), false), ("top".into(), true)]);
    let g = groups(&running);
    assert_eq!((g["base"], g["mid"], g["top"]), (0, 1, 2));
    let before = installed(&t.0);
    let e = start(&ctx, running.clone(), "old").unwrap_err();
    assert!(e.contains("old not started") && e.contains("base"), "{e}");
    assert_eq!(installed(&t.0), before);
    let ids = super::Ids::default();
    let s = publish::snapshot(&ctx, &running, &ids).unwrap();
    assert_eq!(s.manifest.resources.iter().map(|r| r.group).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(s.manifest.content_id, None);
    assert_eq!(ssx_content::delivery::from_json(&s.json).unwrap(), s.manifest);
    for (h, n) in &s.files {
        assert!(ctx.store.has(h, *n), "{h} is in the store");
    }
    assert!(s.manifest.resources[0].files.iter().any(|f| f.role == FileRole::Manifest));
}

#[test]
fn a_cycle_refuses_all_of_it() {
    let t = folder("cycle");
    package(&t.0, "ping", "1.0.0", "[dependencies]\nrequire = [\"pong\"]\n", &[]);
    package(&t.0, "pong", "1.0.0", "[dependencies]\nrequire = [\"ping\"]\n", &[]);
    let ctx = context(&t.0, HashMap::new());
    let e = start(&ctx, Vec::new(), "ping").unwrap_err();
    assert!(e.contains("cycle"), "{e}");
    assert_eq!(installed(&t.0), Vec::<String>::new());
}

#[test]
fn restart_reads_again_and_a_broken_one_keeps_the_running_one() {
    let t = folder("restart");
    package(&t.0, "pack", "1.0.0", "", &[("data/x.txt", b"first")]);
    let ctx = context(&t.0, HashMap::new());
    let running = start(&ctx, Vec::new(), "pack").unwrap();
    let first = running[0].loaded.files.clone();
    package(&t.0, "pack", "1.0.1", "", &[("data/x.txt", b"second")]);
    let o = run(&ctx, &ResourceCommand::Restart("pack".into()), running).unwrap();
    assert_eq!(o.removed.len(), 1);
    assert_eq!(o.running[0].loaded.manifest.resource.version.to_string(), "1.0.1");
    let changed: Vec<&str> =
        o.running[0].loaded.files.iter().filter(|f| !first.contains(f)).map(|f| f.path.as_str()).collect();
    assert_eq!(changed, ["data/x.txt", "resource.toml"]);
    std::fs::write(t.0.join("resources/pack/resource.toml"), "[resource]\nname = \"pack\"\n").unwrap();
    let running = o.running;
    let e = run(&ctx, &ResourceCommand::Restart("pack".into()), running.clone()).unwrap_err();
    assert!(e.contains("not restarted (the running one stays)"), "{e}");
    assert!(run(&ctx, &ResourceCommand::Ensure("pack".into()), running.clone()).is_err());
    let r = run(&ctx, &ResourceCommand::Refresh, running).unwrap();
    assert!(!r.changed);
    assert!(r.said.iter().any(|l| l.contains("pack: changed on disk")), "{:?}", r.said);
}

#[test]
fn stop_takes_its_dependants_and_unneeded_requirements() {
    let t = folder("stop");
    package(&t.0, "lib", "1.0.0", "", &[]);
    package(&t.0, "user", "1.0.0", "[dependencies]\nrequire = [\"lib\"]\n", &[]);
    package(&t.0, "other", "1.0.0", "", &[]);
    let ctx = context(&t.0, HashMap::new());
    let running = start(&ctx, Vec::new(), "user").unwrap();
    let running = start(&ctx, running, "other").unwrap();
    let o = run(&ctx, &ResourceCommand::Stop("lib".into()), running.clone()).unwrap();
    assert_eq!(names(&o.running), [("other".into(), true)]);
    assert_eq!(o.removed.len(), 2);
    let o = run(&ctx, &ResourceCommand::Stop("user".into()), running).unwrap();
    assert_eq!(names(&o.running), [("other".into(), true)]);
    assert_eq!(o.said, ["stopped lib, user"]);
}

#[test]
fn what_a_server_never_sends_is_refused_at_the_start() {
    let t = folder("refused");
    let ctx_bytes = b"a file of the data folder's own index, longer than the 64 bytes a refusal needs........";
    package(&t.0, "scripted", "1.0.0", "[scripts]\nserver = [\"server/*.luau\"]\n", &[("server/main.luau", b"")]);
    package(&t.0, "copied", "1.0.0", "", &[("data/copy.bin", ctx_bytes)]);
    package(&t.0, "named", "1.0.0", "", &[]);
    std::fs::rename(t.0.join("resources/named"), t.0.join("resources/renamed")).unwrap();
    package(
        &t.0,
        "shader",
        "1.0.0",
        "[override]\n\"ssx:shader/terrain\" = \"shaders/terrain.wgsl\"\n",
        &[("shaders/terrain.wgsl", b"fn main() {}\n")],
    );
    let index = HashMap::from([((ctx_bytes.len() as u64, ContentHash::of(ctx_bytes)), "big/x/data/copy.bin".into())]);
    let ctx = context(&t.0, index);
    let e = start(&ctx, Vec::new(), "copied").unwrap_err();
    assert!(e.contains("data/copy.bin is the disc's big/x/data/copy.bin"), "{e}");
    let e = start(&ctx, Vec::new(), "renamed").unwrap_err();
    assert!(e.contains("its resource.toml names it \"named\""), "{e}");
    let e = start(&ctx, Vec::new(), "shader").unwrap_err();
    assert!(e.contains("shader"), "{e}");
    assert_eq!(installed(&t.0), Vec::<String>::new());
    let running = start(&ctx, Vec::new(), "scripted").unwrap();
    assert!(running[0].loaded.files.iter().all(|f| !f.path.starts_with("server/")));
    assert_eq!(running[0].loaded.kept_back, 1);
    assert!(ResourceCommand::parse("start", "../x").unwrap().is_err());
    assert!(ResourceCommand::parse("start", "ssx").unwrap().is_err());
}

#[test]
fn every_levels_id_is_a_plain_line_a_drop_with_changed_records_too() {
    use ssx_content::data::SimContentId;
    use ssx_content::delivery::{LevelId, Row};
    let row = |id: &str| Row { id: id.into(), class: "simulation".into(), hash: ContentHash::of(id.as_bytes()) };
    let level = |level: &str, rows: Vec<Row>| LevelId {
        level: level.into(),
        id: SimContentId([1; 32]),
        files: ContentHash::of(b"files"),
        rows,
    };
    let rows: Vec<Row> = ["ssx:tuning/a", "ssx:tuning/b", "ssx:scoring/c", "ssx:tuning/\u{1b}d"].map(row).into();
    let ids = super::levels::Ids {
        levels: vec![
            level("drop:exp_x", Vec::new()),
            level("drop:exp_y", rows.clone()),
            level("map:m:map/x@race", rows),
        ],
        content_id: Some(SimContentId([2; 32])),
        problem: None,
    };
    let lines = super::ids_lines(&ids);
    assert_eq!(lines.len(), 1 + 3, "{lines:?}");
    assert!(lines.iter().all(|(warning, _)| !warning), "{lines:?}");
    assert!(lines[2].1.starts_with("resources: drop:exp_y sim content id "), "{}", lines[2].1);
    assert!(lines[3].1.starts_with("resources: map:m:map/x@race sim content id "), "{}", lines[3].1);
}

#[test]
fn the_wire_states_the_delivery_caps() {
    use ssx_content::delivery;
    use ssx_content::policy::ClientMods;
    use ssx_net_proto::resources as wire;
    assert_eq!(wire::MAX_RESOURCES as usize, delivery::MAX_RESOURCES);
    assert_eq!(wire::MAX_DOWNLOAD, delivery::MAX_SERVER_BYTES);
    assert_eq!(wire::MAX_MANIFEST_BYTES, delivery::MAX_BODY as u64);
    let names: Vec<&str> = [ClientMods::Presentation, ClientMods::Any, ClientMods::None].map(ClientMods::name).into();
    assert_eq!(wire::CLIENT_MODS.as_slice(), names.as_slice());
    for n in wire::CLIENT_MODS {
        assert!(ClientMods::parse(n).is_some_and(|m| m.name() == n), "{n}");
    }
}

#[test]
fn a_restart_says_when_it_took_effect_or_what_stays() {
    use ssx_content::data::SimContentId;
    let t = folder("restart_lines");
    package(&t.0, "pack", "1.0.0", "", &[("data/x.txt", b"first")]);
    let ctx = context(&t.0, HashMap::new());
    let running = start(&ctx, Vec::new(), "pack").unwrap();
    let (was, now) = (Some(SimContentId([1; 32])), Some(SimContentId([2; 32])));
    let before = Some(ssx_content::ContentHash([0xab; 32]));
    let done = super::restart_done("pack", &running, (now, was), before);
    let digest = running[0].loaded.package.to_string();
    assert!(
        done.starts_with(&format!(
            "resources: restart pack done: pack 1.0.0 (package {}, was abababababababab) runs; the server's content id 0202",
            &digest[..16]
        )),
        "{done}"
    );
    assert!(done.contains("(was 0101") && done.ends_with("commands typed from here on run on it"), "{done}");
    let failed = super::restart_failed("pack", &running, "its scripts: boom\u{1b}[2J");
    assert!(failed.starts_with("resources: restart pack failed, the running pack 1.0.0 stays"), "{failed}");
    assert!(failed.contains("its scripts: boom") && !failed.contains('\u{1b}'), "{failed}");
    let none = super::restart_failed("gone", &running, "gone is not running");
    assert_eq!(none, "resources: restart gone failed, nothing changed: gone is not running");
    let gone = super::restart_done("pack", &[], (None, was), before);
    assert!(gone.contains("pack does not run; the server's content id none"), "{gone}");
}

#[test]
fn after_a_read_that_ended_only_the_running_sets_copies_stay() {
    let t = folder("ended");
    package(&t.0, "pack", "1.0.0", "", &[("data/x.txt", b"first")]);
    package(&t.0, "more", "1.0.0", "", &[("data/y.txt", b"more")]);
    let ctx = context(&t.0, HashMap::new());
    let running = start(&ctx, Vec::new(), "pack").unwrap();
    let kept = installed(&t.0);
    package(&t.0, "pack", "1.0.1", "", &[("data/x.txt", b"second")]);
    assert!(run(&ctx, &ResourceCommand::Restart("pack".into()), running.clone()).is_ok());
    assert!(run(&ctx, &ResourceCommand::Start("more".into()), running.clone()).is_ok());
    assert_eq!(installed(&t.0).len(), 3);
    <Context as super::reader::Reading>::remount(&ctx, &running).unwrap();
    assert_eq!(installed(&t.0), kept);
    assert_eq!(kept.len(), 1);
}
