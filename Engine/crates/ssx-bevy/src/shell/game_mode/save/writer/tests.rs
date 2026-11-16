use ssx_content::ContentId;
use ssx_content::profile_state::StateValue;

use super::*;

fn job(why: &str, path: &Path, bytes: &[u8]) -> Job {
    Job {
        why: why.into(),
        path: path.to_path_buf(),
        bytes: bytes.to_vec(),
        state: ModState::default(),
        summary: String::new(),
        asked: Instant::now(),
        made: Duration::ZERO,
    }
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssx-save-writer-{tag}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn write_job_writes_the_file_then_the_mod_state() {
    let dir = folder("write-job");
    let save = dir.join("Saves").join("ssxvprofile");
    let state = ssx_content::profile_state::state_path(&save);
    let asides = Asides::default();
    write_job(job("first", &save, b"profile one"), &asides);
    assert_eq!(std::fs::read(&save).unwrap(), b"profile one");
    assert!(!state.exists(), "no entries and no file: no state file");

    let mut j = job("second", &save, b"profile two");
    let id = ContentId::parse("tunes:music/one").unwrap();
    assert!(j.state.set(&id, "enabled", StateValue::Bool(false)));
    let text = j.state.write();
    write_job(j, &asides);
    assert_eq!(std::fs::read(&save).unwrap(), b"profile two");
    assert_eq!(std::fs::read_to_string(&state).unwrap(), text);

    *asides.state_unread.lock().unwrap() = Some("a test's unreadable state".into());
    write_job(job("third", &save, b"profile three"), &asides);
    assert_eq!(std::fs::read_to_string(state.with_extension("toml.unread1")).unwrap(), text, "kept aside");
    assert!(!state.exists(), "the third save has no entries");
    assert!(asides.state_unread.lock().unwrap().is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_damaged_file_is_kept_aside_by_the_writer_before_the_new_profile() {
    let dir = folder("damaged");
    let save = dir.join("ssxvprofile");
    let state = ssx_content::profile_state::state_path(&save);
    std::fs::write(&save, b"damaged bytes").unwrap();
    std::fs::write(&state, b"its mod state").unwrap();
    std::fs::write(save.with_extension("damaged1"), b"an older one").unwrap();
    let asides = Asides::default();
    asides.damaged.store(true, Ordering::Release);
    write_job(job("a new profile", &save, b"new profile"), &asides);
    assert_eq!(std::fs::read(&save).unwrap(), b"new profile");
    assert_eq!(std::fs::read(save.with_extension("damaged2")).unwrap(), b"damaged bytes");
    assert_eq!(std::fs::read(save.with_extension("damaged1")).unwrap(), b"an older one");
    let state_aside = ssx_content::profile_state::state_path(&save.with_extension("damaged2"));
    assert_eq!(std::fs::read(state_aside).unwrap(), b"its mod state");
    assert!(!state.exists(), "the new profile has no mod state entries");
    assert!(!asides.damaged.load(Ordering::Acquire));
    std::fs::remove_dir_all(&dir).unwrap();
}
