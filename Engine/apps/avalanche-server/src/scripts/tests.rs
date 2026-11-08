use std::sync::mpsc::sync_channel;
use std::time::Duration;

use ssx_content::manifest::{Files, Scripts};
use ssx_script::host::Side;
use ssx_script::host::limits::HostLimits;
use ssx_script::host::package::PackageScripts;

use super::ServerScripts;
use super::lifecycle::Transaction;
use super::thread::Job;

struct Folder(std::path::PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(tag: &str) -> Folder {
    let dir = std::env::temp_dir().join(format!("ssx-scripts-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Folder(dir)
}

fn package(name: &str) -> PackageScripts {
    package_with(name, "local started = true")
}

pub(super) fn package_with(name: &str, source: &str) -> PackageScripts {
    let scripts = Scripts { client: vec![], server: vec!["server/*.luau".into()], shared: vec![] };
    let files = [("server/main.luau".to_string(), source.to_string())];
    PackageScripts::from_memory(
        name,
        Side::Server,
        &scripts,
        &Files::default(),
        toml::Table::new(),
        &files,
        &HostLimits::SERVER,
    )
    .unwrap()
}

fn said(s: &ServerScripts, wait: Duration) -> Vec<String> {
    let done = s.done.lock().unwrap();
    let mut out = Vec::new();
    while let Ok(d) = done.recv_timeout(wait) {
        out.extend(d.said.into_iter().map(|(_, l)| l));
    }
    out
}

#[test]
fn a_change_answered_in_time_stays() {
    let dir = folder("kept");
    let s = ServerScripts::spawn(&dir.0).unwrap();
    let (tx, rx) = sync_channel(0);
    s.handle.send(Job::Transact(Transaction::starting(vec![package("kept")]), tx)).unwrap();
    let answer = rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    assert_eq!(answer, ["kept: its server scripts started"]);
    assert!(said(&s, Duration::from_millis(500)).is_empty());
}

#[test]
fn a_change_the_server_gave_up_is_undone() {
    let dir = folder("given-up");
    let s = ServerScripts::spawn(&dir.0).unwrap();
    let (tx, rx) = sync_channel(0);
    drop(rx);
    s.handle.send(Job::Transact(Transaction::starting(vec![package("late")]), tx)).unwrap();
    let lines = said(&s, Duration::from_secs(5));
    assert_eq!(
        lines,
        [
            "scripts: a resource change was given up by the server: undone",
            "scripts: late: its server scripts stopped (the change was given up)",
        ]
    );
}

#[test]
fn a_console_word_reaches_only_a_script_that_registered_it() {
    let dir = folder("console");
    let s = ServerScripts::spawn(&dir.0).unwrap();
    let src = r#"addCommandHandler("hello", function(_, _, who) print("hello " .. tostring(who)) end)"#;
    let (tx, rx) = sync_channel(0);
    s.handle.send(Job::Transact(Transaction::starting(vec![package_with("greet", src)]), tx)).unwrap();
    rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    let (reply, answer) = tokio::sync::oneshot::channel();
    super::console(&s, "hello you", Some(reply)).unwrap();
    let lines = said(&s, Duration::from_millis(500));
    assert_eq!(lines, ["scripts: ran hello you", "scripts: greet: hello you"], "{lines:?}");
    assert_eq!(answer.blocking_recv().unwrap(), (true, vec!["scripts: ran hello you".to_string()]));
    super::console(&s, "rm -rf /", None).unwrap();
    let lines = said(&s, Duration::from_millis(500));
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with("unknown command rm -rf /: no server script registered it; commands:"), "{lines:?}");
}

#[test]
fn a_print_line_reaches_the_log_whole_up_to_the_vms_bound() {
    let dir = folder("print");
    let s = ServerScripts::spawn(&dir.0).unwrap();
    let long = "a welcome line of a server resource that is longer than sixty-four characters, as players write";
    let src = format!(
        r#"addCommandHandler("say", function() print("{long}"); print(string.rep("x", 9000)); print("a\27[2Jb"); print(string.rep("\27", 5000)) end)"#
    );
    let (tx, rx) = sync_channel(0);
    s.handle.send(Job::Transact(Transaction::starting(vec![package_with("words", &src)]), tx)).unwrap();
    rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
    super::console(&s, "say", None).unwrap();
    let lines = said(&s, Duration::from_millis(500));
    assert_eq!(lines.len(), 5, "{lines:?}");
    assert_eq!(lines[1], format!("scripts: words: {long}"));
    let x = "x".repeat(ssx_script::MAX_OUTPUT_LINE);
    assert_eq!(lines[2], format!("scripts: words: {x}… ({} bytes cut)", 9000 - ssx_script::MAX_OUTPUT_LINE));
    assert_eq!(lines[3], "scripts: words: a\\u{1b}[2Jb");
    let hostile = lines[4].strip_prefix("scripts: words: ").unwrap();
    assert!(hostile.starts_with("\\u{1b}\\u{1b}") && hostile.ends_with(" bytes)"), "{hostile:.80}");
    assert!(hostile.chars().count() <= super::thread::PRINT_CHARS + 32, "{}", hostile.chars().count());
}

#[test]
fn many_slow_clients_stay_within_the_bound() {
    use std::collections::BTreeSet;

    use ssx_net::server::{ScriptLink, TOTAL_QUEUE_BYTES};

    use super::thread::Done;
    use super::world::{ScriptStats, queue_answer};

    let clients = 64u32;
    let item: ssx_net::server::Shared = vec![7u8; 64 << 10].into();
    let mut owned: Vec<ScriptLink> = (1..=clients).map(ScriptLink::new).collect();
    let mut resync = BTreeSet::new();
    let mut stats = ScriptStats::default();
    let mut d = Done::default();
    d.firsts.push((1, vec![vec![1u8; 1 << 20].into()]));
    let mut ends: Vec<(u32, &str)> = Vec::new();
    let started = std::time::Instant::now();
    let mut peak = 0usize;
    for answer in 0..64u64 {
        let to: Vec<u32> = (1..=clients).filter(|c| !ends.iter().any(|(e, _)| e == c)).collect();
        d.out = vec![(to, item.clone(), 0)];
        let now = Duration::from_millis(33 * (answer + 1));
        let mut links: Vec<(u32, &mut ScriptLink)> = owned.iter_mut().map(|l| (l.client, l)).collect();
        ends.extend(queue_answer(&d, &mut links, &mut resync, &mut stats, now));
        d.firsts.clear();
        let total: usize = links.iter().map(|(_, l)| l.held()).sum();
        assert!(total <= TOTAL_QUEUE_BYTES + clients as usize * item.len() + 32, "answer {answer}: {total}");
        peak = peak.max(total);
        if let Some((_, l)) = links.iter_mut().find(|(c, _)| *c == 2) {
            l.take_wires(usize::MAX, now);
            assert_eq!(l.held(), 0);
        }
    }
    let took = started.elapsed();
    assert_eq!(ends.first().map(|(c, _)| *c), Some(1), "the snapshot holder that waited longest is sent away");
    assert!(stats.budget_resyncs > 0 && !resync.contains(&2), "{stats:?}");
    assert!(stats.queues_peak <= TOTAL_QUEUE_BYTES, "after each answer within the bound: {}", stats.queues_peak);
    println!(
        "bound {TOTAL_QUEUE_BYTES}: {clients} slow clients, 64 answers of 64 KiB to each (256 MiB unbounded): peak \
         {peak} B within answers, {} after them; {} resynced, {} sent away; {:?} in all",
        stats.queues_peak, stats.budget_resyncs, stats.budget_ends, took
    );
}
