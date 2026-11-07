use std::path::{Path, PathBuf};

use super::*;
use crate::update::install::game_name;

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("ssx-journal-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn server_name() -> &'static str {
    if cfg!(windows) { "avalanche-server.exe" } else { "avalanche-server" }
}

type Planned = (&'static str, Option<&'static [u8]>, &'static [u8]);

fn plan() -> Vec<Planned> {
    vec![
        (launcher_name(), Some(b"old launcher"), b"new launcher"),
        (game_name(), Some(b"old game"), b"new game"),
        (server_name(), None, b"new server"),
        ("THIRD_PARTY_NOTICES.txt", Some(b"notices"), b"notices"),
    ]
}

fn fonts_plan() -> Vec<Planned> {
    vec![
        (launcher_name(), Some(b"old launcher"), b"new launcher"),
        (game_name(), Some(b"old game"), b"new game"),
        ("fonts/NotoSans-Regular.ttf", None, b"new font"),
        ("fonts/OFL.txt", None, b"license"),
    ]
}

fn fonts_again_plan() -> Vec<Planned> {
    vec![
        (launcher_name(), Some(b"old launcher"), b"new launcher"),
        ("fonts/NotoSans-Regular.ttf", Some(b"old font"), b"new font"),
        ("fonts/NotoSansThai-Regular.ttf", None, b"new thai"),
    ]
}

fn fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf, Vec<String>) {
    fixture_of(tag, &plan())
}

fn fixture_of(tag: &str, plan: &[Planned]) -> (PathBuf, PathBuf, PathBuf, Vec<String>) {
    let root = scratch(tag);
    let install = root.join("install");
    let staged = install.join(".update-0.3.0-0");
    std::fs::create_dir_all(&staged).unwrap();
    let write = |at: PathBuf, bytes: &[u8]| {
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, bytes).unwrap();
    };
    for (name, old, new) in plan {
        if let Some(old) = old {
            write(path_of(&install, name), old);
        }
        write(path_of(&staged, name), new);
    }
    (root, install.clone(), staged, plan.iter().map(|(n, ..)| n.to_string()).collect())
}

#[derive(Debug, PartialEq, Eq)]
enum Held {
    Old,
    New,
}

fn held(install: &Path) -> Held {
    held_of(install, &plan())
}

fn held_of(install: &Path, plan: &[Planned]) -> Held {
    let read = |name: &str| std::fs::read(path_of(install, name)).ok();
    let old = plan.iter().all(|(n, old, _)| read(n) == old.map(<[u8]>::to_vec));
    let new = plan.iter().all(|(n, _, new)| read(n) == Some(new.to_vec()));
    assert!(!there(&install.join(BACKUP)), "the backup folder is left");
    for entry in std::fs::read_dir(install).unwrap().flatten() {
        let name = entry.file_name().into_string().unwrap();
        let known = plan.iter().any(|(n, ..)| n.split('/').next() == Some(name.as_str())) || is_staging_name(&name);
        assert!(known, "{name} is left in the install");
    }
    if let Ok(fonts) = std::fs::read_dir(install.join(FONTS)) {
        let mut any = false;
        for entry in fonts.flatten() {
            let name = format!("{FONTS}/{}", entry.file_name().into_string().unwrap());
            assert!(plan.iter().any(|(n, ..)| *n == name), "{name} is left in the install");
            any = true;
        }
        assert!(any, "an empty fonts folder is left");
    }
    match (old, new) {
        (true, false) => Held::Old,
        (false, true) => Held::New,
        _ => panic!("a mixed install: {:?}", plan.iter().map(|(n, ..)| (n, read(n))).collect::<Vec<_>>()),
    }
}

fn crash_install(at: usize, install: &Path, staged: &Path, files: &[String]) -> bool {
    crash::crash_at(Some(at));
    let result = super::install(staged, install, files);
    let crashed = crash::happened();
    crash::crash_at(None);
    if !crashed {
        assert_eq!(result, Ok(None));
    }
    crashed
}

#[test]
fn a_crash_at_any_step_leaves_the_old_install_or_the_new_one() {
    let mut outcomes = Vec::new();
    for at in 0.. {
        let (root, install, staged, files) = fixture(&format!("crash-{at}"));
        if !crash_install(at, &install, &staged, &files) {
            assert_eq!(held(&install), Held::New);
            std::fs::remove_dir_all(&root).unwrap();
            break;
        }
        let recovered = recover(&install).unwrap();
        let now = held(&install);
        match recovered {
            Recovered::Nothing | Recovered::RolledBack => assert_eq!(now, Held::Old, "step {at}"),
            Recovered::Finished => assert_eq!(now, Held::New, "step {at}"),
            Recovered::Cleared => {}
            Recovered::Busy => panic!("step {at}: a crashed install's lock is held"),
        }
        assert_eq!(recover(&install), Ok(Recovered::Nothing));
        outcomes.push((at, recovered));
        std::fs::remove_dir_all(&root).unwrap();
    }
    assert!(outcomes.len() >= 17, "{outcomes:?}");
    assert!(outcomes.contains(&(9, Recovered::Finished)), "{outcomes:?}");
    assert!(outcomes.iter().any(|(_, r)| *r == Recovered::RolledBack), "{outcomes:?}");
    assert!(outcomes.iter().any(|(_, r)| *r == Recovered::Finished), "{outcomes:?}");
}

#[test]
fn a_crash_of_the_recovery_is_recovered_at_the_next_start() {
    let mut runs = 0;
    for at in 0.. {
        let (root, install, staged, files) = fixture(&format!("again-{at}"));
        let crashed = crash_install(at, &install, &staged, &files);
        std::fs::remove_dir_all(&root).unwrap();
        if !crashed {
            break;
        }
        for again in 0.. {
            let (root, install, staged, files) = fixture(&format!("again-{at}-{again}"));
            assert!(crash_install(at, &install, &staged, &files));
            crash::crash_at(Some(again));
            let first = recover(&install);
            let crashed = crash::happened();
            crash::crash_at(None);
            let recovered = if crashed { recover(&install).unwrap() } else { first.unwrap() };
            let now = held(&install);
            match recovered {
                Recovered::RolledBack if !crashed => assert_eq!(now, Held::Old, "{at}/{again}"),
                Recovered::Finished if !crashed => assert_eq!(now, Held::New, "{at}/{again}"),
                Recovered::Busy => panic!("{at}/{again}: a crashed recovery's lock is held"),
                _ => {}
            }
            std::fs::remove_dir_all(&root).unwrap();
            runs += 1;
            if !crashed {
                break;
            }
        }
    }
    assert!(runs >= 128, "{runs}");
}

#[test]
fn an_error_at_any_step_puts_the_old_files_back_or_ends_the_install() {
    for at in 0.. {
        let (root, install, staged, files) = fixture(&format!("fail-{at}"));
        crash::fail_at(at);
        let result = super::install(&staged, &install, &files);
        let reached = crash::reached();
        crash::crash_at(None);
        if !reached {
            assert_eq!(result, Ok(None));
            assert_eq!(held(&install), Held::New);
            std::fs::remove_dir_all(&root).unwrap();
            break;
        }
        if there(&install.join(BACKUP)) {
            assert!(recover(&install).is_ok(), "step {at}: {result:?}");
        }
        let expected = if result.is_ok() { Held::New } else { Held::Old };
        assert_eq!(held(&install), expected, "step {at}: {result:?}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}

fn cut_after_the_game(tag: &str) -> (PathBuf, PathBuf, PathBuf, Vec<String>) {
    let (root, install, staged, files) = fixture(tag);
    let at = (0..).find(|&at| {
        let (r, i, s, f) = fixture(&format!("{tag}-probe-{at}"));
        crash_install(at, &i, &s, &f);
        let renamed = std::fs::read(i.join(game_name())).unwrap() == b"new game";
        std::fs::remove_dir_all(&r).unwrap();
        renamed
    });
    assert!(crash_install(at.unwrap(), &install, &staged, &files));
    assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"new game");
    (root, install, staged, files)
}

#[test]
fn a_held_lock_is_left_alone() {
    let (root, install, staged, files) = cut_after_the_game("held");
    let other = std::fs::File::open(install.join(BACKUP).join(LOCK)).unwrap();
    other.try_lock().unwrap();
    assert_eq!(recover(&install), Ok(Recovered::Busy));
    assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"new game");
    let refused = super::install(&staged, &install, &files).unwrap_err();
    assert!(refused.contains("did not finish"), "{refused}");
    drop(other);
    assert_eq!(recover(&install), Ok(Recovered::RolledBack));
    assert_eq!(held(&install), Held::Old);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_file_neither_old_nor_new_keeps_its_backup() {
    let (root, install, ..) = cut_after_the_game("third");
    std::fs::write(install.join(game_name()), b"a third game").unwrap();
    let refused = recover(&install).unwrap_err();
    assert!(refused.contains("neither"), "{refused}");
    assert_eq!(std::fs::read(install.join(BACKUP).join(game_name())).unwrap(), b"old game");
    assert!(there(&install.join(BACKUP).join(JOURNAL)));
    std::fs::write(install.join(game_name()), b"new game").unwrap();
    assert_eq!(recover(&install), Ok(Recovered::RolledBack));
    assert_eq!(held(&install), Held::Old);
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_journal_reads_back_as_written() {
    let journal = Journal {
        staged: ".update-0.3.0-2".into(),
        made: Vec::new(),
        entries: vec![
            Entry { name: game_name().into(), old: Some([7; 32]), new: [9; 32] },
            Entry { name: launcher_name().into(), old: None, new: [0xab; 32] },
        ],
    };
    assert_eq!(Journal::parse(&journal.text()), Ok(journal));
    let fonts = Journal {
        staged: ".update-0.3.0-2".into(),
        made: vec![FONTS.into()],
        entries: vec![
            Entry { name: "fonts/NotoSansCJKjp-Regular.otf".into(), old: Some([1; 32]), new: [2; 32] },
            Entry { name: "fonts/NotoSansDevanagari-Regular.ttf".into(), old: Some([1; 32]), new: [2; 32] },
            Entry { name: launcher_name().into(), old: Some([3; 32]), new: [4; 32] },
        ],
    };
    assert_eq!(Journal::parse(&fonts.text()), Ok(fonts));
    let mut all: Vec<Entry> = allowed_files()
        .iter()
        .filter(|n| **n != launcher_name())
        .map(|n| Entry { name: n.to_string(), old: Some([0xee; 32]), new: [0xff; 32] })
        .collect();
    all.push(Entry { name: launcher_name().into(), old: Some([0xee; 32]), new: [0xff; 32] });
    let longest = Journal { staged: ".update-10000.10000.10000-99".into(), made: vec![FONTS.into()], entries: all };
    assert!(longest.text().len() as u64 <= MAX_JOURNAL, "{}", longest.text().len());
    assert_eq!(Journal::parse(&longest.text()), Ok(longest));
}

#[test]
fn a_journal_that_is_not_the_launchers_is_left_alone() {
    let h = "ab".repeat(32);
    let (l, g) = (launcher_name(), game_name());
    let bad = [
        "garbage\n".to_string(),
        format!("{MAGIC}\nstaged ../x-0.3.0-0\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-backup\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - evil.sh\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} {h} {l}\nfile {h} - {g}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - {l}\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {} - {l}\n", h.to_uppercase()),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - {l}"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\n\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - {l}\n{}\n", "x".repeat(5000)),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfolder ..\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfolder fonts\nfolder fonts\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - {g}\nfolder fonts\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - fonts/../{l}\nfile {h} - {l}\n"),
        format!("{MAGIC}\nstaged .update-0.3.0-0\nfile {h} - fonts/evil.ttf\nfile {h} - {l}\n"),
    ];
    for (i, text) in bad.iter().enumerate() {
        let root = scratch(&format!("foreign-{i}"));
        let dir = root.join(BACKUP);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(root.join(l), b"launcher").unwrap();
        std::fs::write(dir.join(LOCK), b"").unwrap();
        std::fs::write(dir.join(JOURNAL), text).unwrap();
        std::fs::write(dir.join(l), b"backup").unwrap();
        assert!(recover(&root).is_err(), "{i}");
        assert_eq!(std::fs::read(dir.join(JOURNAL)).unwrap(), text.as_bytes(), "{i}");
        assert_eq!(std::fs::read(dir.join(l)).unwrap(), b"backup", "{i}");
        assert_eq!(std::fs::read(root.join(l)).unwrap(), b"launcher", "{i}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}

#[test]
fn a_backup_without_a_journal_is_removed_only_when_it_holds_nothing() {
    let root = scratch("no-journal");
    let dir = root.join(BACKUP);
    std::fs::create_dir(&dir).unwrap();
    assert_eq!(recover(&root), Ok(Recovered::Cleared), "made, then cut short before the lock");
    std::fs::create_dir(&dir).unwrap();
    for name in [LOCK, COMMITTED, "journal.partial"] {
        std::fs::write(dir.join(name), b"").unwrap();
    }
    assert_eq!(recover(&root), Ok(Recovered::Cleared));
    assert!(!there(&dir));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join(LOCK), b"").unwrap();
    std::fs::write(dir.join(game_name()), b"only copy").unwrap();
    assert!(recover(&root).is_err());
    assert_eq!(std::fs::read(dir.join(game_name())).unwrap(), b"only copy");
    std::fs::remove_file(dir.join(LOCK)).unwrap();
    assert!(recover(&root).is_err(), "files but no lock");
    assert_eq!(std::fs::read(dir.join(game_name())).unwrap(), b"only copy");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_backup_of_the_updater_0_2_0_is_never_lost() {
    let root = scratch("legacy");
    let (one, two) = (root.join(".update-backup-77"), root.join(".update-backup-78"));
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    std::fs::write(root.join(launcher_name()), b"new launcher").unwrap();
    std::fs::write(one.join(launcher_name()), b"old launcher").unwrap();
    std::fs::write(one.join(game_name()), b"old game").unwrap();
    std::fs::write(one.join("evil.sh"), b"x").unwrap();
    std::fs::write(two.join("README.txt"), b"old readme").unwrap();
    let notes = recover_legacy(&root);
    assert_eq!(std::fs::read(root.join(game_name())).unwrap(), b"old game", "{notes:?}");
    assert_eq!(std::fs::read(root.join("README.txt")).unwrap(), b"old readme");
    assert_eq!(std::fs::read(root.join(launcher_name())).unwrap(), b"new launcher");
    assert_eq!(std::fs::read(one.join(launcher_name())).unwrap(), b"old launcher");
    assert!(one.join("evil.sh").exists());
    assert!(!two.exists(), "an emptied folder goes");
    assert_eq!(notes.len(), 4, "{notes:?}");
    assert_eq!(recover(&root), Ok(Recovered::Nothing), "a 0.2.0 folder is not the install's");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_lock_is_one_holders() {
    let root = scratch("lock");
    assert_eq!(Lock::take(&root).err().map(|e| e.kind()), Some(ErrorKind::NotFound));
    let first = Lock::create(&root).unwrap();
    assert!(Lock::create(&root).is_err());
    assert!(Lock::take(&root).unwrap().is_none());
    first.release().unwrap();
    assert!(!there(&root.join(LOCK)));
    std::fs::write(root.join(LOCK), b"").unwrap();
    let again = Lock::take(&root).unwrap().expect("a lock nobody holds");
    assert!(Lock::take(&root).unwrap().is_none());
    again.release().unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn only_windows_waits_for_a_file_held_open() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION,
        };
        for (code, held) in [
            (ERROR_SHARING_VIOLATION, true),
            (ERROR_LOCK_VIOLATION, true),
            (ERROR_ACCESS_DENIED, true),
            (ERROR_FILE_NOT_FOUND, false),
        ] {
            assert_eq!(held_open(&std::io::Error::from_raw_os_error(code as i32)), held, "{code}");
        }
    }
    #[cfg(not(windows))]
    assert!(!held_open(&std::io::Error::from(ErrorKind::PermissionDenied)));
    let mut tries = 0;
    let once: Result<(), std::io::Error> = retried(|| {
        tries += 1;
        Err(std::io::Error::from(ErrorKind::NotFound))
    });
    assert!(once.is_err());
    assert_eq!(tries, 1, "an error no handle causes is not tried again");
}

#[test]
fn a_crash_at_any_step_with_the_fonts_leaves_the_old_install_or_the_new_one() {
    for (tag, plan) in [("fonts", fonts_plan()), ("fonts-again", fonts_again_plan())] {
        let mut outcomes = Vec::new();
        for at in 0.. {
            let (root, install, staged, files) = fixture_of(&format!("{tag}-{at}"), &plan);
            if !crash_install(at, &install, &staged, &files) {
                assert_eq!(held_of(&install, &plan), Held::New);
                std::fs::remove_dir_all(&root).unwrap();
                break;
            }
            let recovered = recover(&install).unwrap();
            let now = held_of(&install, &plan);
            match recovered {
                Recovered::Nothing | Recovered::RolledBack => assert_eq!(now, Held::Old, "{tag} step {at}"),
                Recovered::Finished => assert_eq!(now, Held::New, "{tag} step {at}"),
                Recovered::Cleared => {}
                Recovered::Busy => panic!("{tag} step {at}: a crashed install's lock is held"),
            }
            assert_eq!(recover(&install), Ok(Recovered::Nothing));
            outcomes.push((at, recovered));
            std::fs::remove_dir_all(&root).unwrap();
        }
        assert!(outcomes.iter().any(|(_, r)| *r == Recovered::RolledBack), "{tag} {outcomes:?}");
        assert!(outcomes.iter().any(|(_, r)| *r == Recovered::Finished), "{tag} {outcomes:?}");
    }
}

#[test]
fn a_crash_of_the_recovery_with_the_fonts_is_recovered_at_the_next_start() {
    for (tag, plan) in [("fonts-again", fonts_plan()), ("fonts-twice", fonts_again_plan())] {
        for at in 0.. {
            let (root, install, staged, files) = fixture_of(&format!("{tag}-{at}"), &plan);
            let crashed = crash_install(at, &install, &staged, &files);
            std::fs::remove_dir_all(&root).unwrap();
            if !crashed {
                break;
            }
            for again in 0.. {
                let (root, install, staged, files) = fixture_of(&format!("{tag}-{at}-{again}"), &plan);
                assert!(crash_install(at, &install, &staged, &files));
                crash::crash_at(Some(again));
                let first = recover(&install);
                let crashed = crash::happened();
                crash::crash_at(None);
                let recovered = if crashed { recover(&install).unwrap() } else { first.unwrap() };
                let now = held_of(&install, &plan);
                match recovered {
                    Recovered::RolledBack if !crashed => assert_eq!(now, Held::Old, "{tag} {at}/{again}"),
                    Recovered::Finished if !crashed => assert_eq!(now, Held::New, "{tag} {at}/{again}"),
                    Recovered::Busy => panic!("{tag} {at}/{again}: a crashed recovery's lock is held"),
                    _ => {}
                }
                std::fs::remove_dir_all(&root).unwrap();
                if !crashed {
                    break;
                }
            }
        }
    }
}

#[test]
fn an_error_at_any_step_with_the_fonts_puts_the_old_files_back_or_ends_the_install() {
    for (tag, plan) in [("fonts-fail", fonts_plan()), ("fonts-again-fail", fonts_again_plan())] {
        for at in 0.. {
            let (root, install, staged, files) = fixture_of(&format!("{tag}-{at}"), &plan);
            crash::fail_at(at);
            let result = super::install(&staged, &install, &files);
            let reached = crash::reached();
            crash::crash_at(None);
            if !reached {
                assert_eq!(result, Ok(None));
                assert_eq!(held_of(&install, &plan), Held::New);
                std::fs::remove_dir_all(&root).unwrap();
                break;
            }
            if there(&install.join(BACKUP)) {
                assert!(recover(&install).is_ok(), "{tag} step {at}: {result:?}");
            }
            let expected = if result.is_ok() { Held::New } else { Held::Old };
            assert_eq!(held_of(&install, &plan), expected, "{tag} step {at}: {result:?}");
            std::fs::remove_dir_all(&root).unwrap();
        }
    }
}

#[test]
fn the_fonts_folder_is_never_taken_over() {
    let plan = fonts_plan();
    let (root, install, staged, files) = fixture_of("fonts-file", &plan);
    std::fs::write(install.join(FONTS), b"a file").unwrap();
    let refused = super::install(&staged, &install, &files).unwrap_err();
    assert!(refused.contains("not a plain folder"), "{refused}");
    assert_eq!(std::fs::read(install.join(FONTS)).unwrap(), b"a file");
    assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"old game");
    assert!(!there(&install.join(BACKUP)));
    std::fs::remove_dir_all(&root).unwrap();
    #[cfg(unix)]
    {
        let (root, install, staged, files) = fixture_of("fonts-link", &plan);
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, install.join(FONTS)).unwrap();
        let refused = super::install(&staged, &install, &files).unwrap_err();
        assert!(refused.contains("not a plain folder"), "{refused}");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        std::fs::remove_dir_all(&root).unwrap();
    }
    let at = (0..)
        .find(|&at| {
            let (r, i, s, f) = fixture_of(&format!("fonts-probe-{at}"), &plan);
            crash_install(at, &i, &s, &f);
            let renamed = there(&path_of(&i, "fonts/NotoSans-Regular.ttf"));
            std::fs::remove_dir_all(&r).unwrap();
            renamed
        })
        .unwrap();
    let (root, install, staged, files) = fixture_of("fonts-stranger", &plan);
    assert!(crash_install(at, &install, &staged, &files));
    std::fs::write(install.join(FONTS).join("mine.txt"), b"mine").unwrap();
    assert_eq!(recover(&install), Ok(Recovered::RolledBack));
    assert!(!there(&path_of(&install, "fonts/NotoSans-Regular.ttf")));
    assert_eq!(std::fs::read(install.join(FONTS).join("mine.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(install.join(game_name())).unwrap(), b"old game");
    std::fs::remove_dir_all(&root).unwrap();
}
