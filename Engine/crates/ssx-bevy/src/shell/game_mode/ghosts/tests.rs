use std::path::{Path, PathBuf};

use super::*;
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock, RECORDER_BUILD, UserHeader};
use ssx_formats::t3db::{Database, MetaSchema};
use ssx_physics::session::recorder::GhostRecorder;

const DROP: u32 = 0xAE9C_36C8;
const OTHER: u32 = 0x1234_5678;

fn schema() -> SaveSchema {
    let field = |name: &str, short: &str, depth: u32, low: &str, high: &str| {
        format!(
            "<field name=\"{name}\" shortname=\"{short}\" type=\"FIELDTYPE_INTEGER\" depth=\"{depth}\" \
             rangelow=\"{low}\" rangehigh=\"{high}\" />"
        )
    };
    let xml = format!(
        "<database name=\"x\" shortname=\"y\" version=\"6\"><table name=\"common_explore_scores\" shortname=\"tggg\" \
         maxinsert=\"300\" savegroups=\"SSXV_COMMON\"><fields>{}{}{}</fields></table></database>",
        field("TRACKID", "f001", 32, "-2147483648", "2147483647"),
        field("SCORE", "f002", 32, "-2147483648", "2147483647"),
        field("GHOSTID", "f003", 9, "0", "500"),
    );
    let meta = MetaSchema::parse(&xml).unwrap();
    let database = Database::from_meta(&meta, &["tggg"]).unwrap();
    SaveSchema { database, meta, riders: Vec::new(), items: Default::default(), drops: Vec::new(), audio: None }
}

fn block(p: [f32; 3]) -> PoseBlock {
    let mut b = [0u8; POSE_BLOCK_SIZE];
    b[12..16].copy_from_slice(&1.0f32.to_be_bytes());
    for (k, v) in p.into_iter().enumerate() {
        b[0x10 + 4 * k..0x14 + 4 * k].copy_from_slice(&v.to_be_bytes());
    }
    PoseBlock::parse(&b).unwrap()
}

fn recording(seconds: u32, score: u32) -> GhostFile {
    let mut rec = GhostRecorder::default();
    let mut ms = 0u32;
    rec.feed(ms);
    rec.on_race_start();
    let mut k = 0;
    while !rec.halted() {
        let p = bevy::math::Vec3::new(k as f32, -(k as f32), 2.0);
        rec.sample(p, k as f32);
        rec.after_execution(Some(block(p.to_array())));
        k += 1;
        if k == seconds * 30 {
            rec.on_results();
        }
        ms += 33;
        rec.feed(ms);
    }
    let header = UserHeader { gamertag: "Rider".into(), gear_flags: 1, score, build: RECORDER_BUILD };
    rec.recording(header).expect("a complete recording")
}

fn folder(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ssx-own-ghosts-{tag}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn boot(path: &Path) -> (GhostSave, String) {
    GhostSave::load(path.to_path_buf(), &schema()).unwrap()
}

fn own(save: &mut GhostSave, drop: u32, best: Option<u32>) -> Option<(usize, GhostReplay)> {
    let slot = save.own_slot(drop, best?)?;
    match save.load_slot(slot, &schema()) {
        SlotLoad::Loaded(replay) => Some((slot, replay)),
        _ => None,
    }
}

#[test]
fn the_ghosts_file_sits_beside_the_profile() {
    assert_eq!(ghosts_path(Path::new("Saves/ssxvprofile")), Path::new("Saves/ssxvghosts"));
    assert_eq!(ghosts_path(Path::new("a/profile.sav")), Path::new("a/profile.sav.ghosts"));
}

#[test]
fn a_kept_ghost_round_trips_and_follows_the_best() {
    let dir = folder("round-trip");
    let path = dir.join("ssxvghosts");
    let (mut save, note) = boot(&path);
    assert!(note.contains("does not exist"), "{note}");
    assert!(own(&mut save, DROP, Some(70_000)).is_none());
    let first = recording(20, 70_000);
    assert_eq!(save.keep(DROP, 70_000, &first).unwrap(), 0);
    drop(save);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.len(), group::slot_offset(1));
    let region = &bytes[group::slot_offset(0)..group::slot_offset(1)];
    let back = ssx_formats::ghost::read_any(group::slot_container(region).unwrap()).unwrap();
    assert_eq!(back.write().unwrap(), first.write().unwrap());

    let (mut save, note) = boot(&path);
    assert!(note.contains("own ghosts of 1 drops"), "{note}");
    let (slot, replay) = own(&mut save, DROP, Some(70_000)).expect("the own ghost");
    assert_eq!((slot, replay.header.score, replay.header.gamertag.as_str()), (0, 70_000, "Rider"));
    assert_eq!(replay.times.len(), first.positions().unwrap().len());
    assert!(own(&mut save, DROP, Some(69_000)).is_none());
    assert!(own(&mut save, DROP, None).is_none());
    assert!(own(&mut save, OTHER, Some(70_000)).is_none());

    let better = recording(18, 64_000);
    assert_eq!(save.keep(DROP, 64_000, &better).unwrap(), 0);
    assert_eq!(save.keep(OTHER, 9_000, &recording(3, 9_000)).unwrap(), 1);
    drop(save);
    let (mut save, _) = boot(&path);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), group::slot_offset(2) as u64);
    assert!(own(&mut save, DROP, Some(70_000)).is_none(), "the old best's ghost is gone");
    let (slot, replay) = own(&mut save, DROP, Some(64_000)).expect("the better run");
    assert_eq!((slot, replay.header.score), (0, 64_000));
    assert_eq!(own(&mut save, OTHER, Some(9_000)).map(|(s, _)| s), Some(1));
    drop(save);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_damaged_file_is_refused() {
    let dir = folder("damaged");
    let path = dir.join("ssxvghosts");
    let (mut save, _) = boot(&path);
    save.keep(DROP, 70_000, &recording(10, 70_000)).unwrap();
    save.keep(OTHER, 5_000, &recording(3, 5_000)).unwrap();
    drop(save);
    let good = std::fs::read(&path).unwrap();

    let mut bad = good.clone();
    bad[group::slot_offset(0) + 0x100] ^= 0x40;
    std::fs::write(&path, &bad).unwrap();
    let (mut save, note) = boot(&path);
    assert!(!note.contains("ERROR"), "{note}");
    assert_eq!(save.read_state(), GhostsRead::Loaded);
    assert!(matches!(save.load_slot(0, &schema()), SlotLoad::Damaged(_)));
    assert!(own(&mut save, OTHER, Some(5_000)).is_some());
    assert!(matches!(save.load_slot(7, &schema()), SlotLoad::Damaged(_)));
    drop(save);

    let mut bad = good.clone();
    bad[group::common_offset() + 0x500] ^= 0x01;
    std::fs::write(&path, &bad).unwrap();
    let (mut save, note) = boot(&path);
    assert!(note.contains("ERROR"), "{note}");
    assert_eq!(save.read_state(), GhostsRead::Corrupt);
    assert!(save.own_slot(DROP, 70_000).is_none(), "nothing loaded into memory");
    assert!(matches!(save.load_slot(1, &schema()), SlotLoad::Damaged(_)));
    assert_eq!(save.keep(DROP, 60_000, &recording(5, 60_000)).unwrap(), 0);
    drop(save);
    assert_eq!(std::fs::read(dir.join("ssxvghosts.unread1")).unwrap(), bad);
    let (mut save, note) = boot(&path);
    assert!(note.contains("own ghosts of 1 drops"), "{note}");
    assert!(own(&mut save, DROP, Some(60_000)).is_some());
    drop(save);

    for (what, bytes) in [
        ("cut", good[..good.len() - 1].to_vec()),
        ("header", {
            let mut b = good.clone();
            b[0] ^= 0x20;
            b
        }),
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let (mut save, note) = boot(&path);
        assert!(note.contains("ERROR"), "{what}: {note}");
        assert!(own(&mut save, DROP, Some(60_000)).is_none(), "{what}");
    }
    let f = std::fs::File::create(&path).unwrap();
    f.set_len(group::MAX_FILE_LEN as u64 + 1).unwrap();
    drop(f);
    let (_, note) = boot(&path);
    assert!(note.contains("longer than the group's"), "{note}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_failed_sign_in_load_saves_the_group_anew() {
    let dir = folder("sign-in");
    let path = dir.join("ssxvghosts");
    let (mut save, _) = boot(&path);
    assert_eq!(save.read_state(), GhostsRead::Missing);
    assert!(matches!(save.load_slot(0, &schema()), SlotLoad::Missing));
    save.write_new().unwrap();
    drop(save);
    let fresh = std::fs::read(&path).unwrap();
    assert_eq!(fresh.len(), group::slot_offset(0));
    let (mut save, note) = boot(&path);
    assert!(note.contains("own ghosts of 0 drops"), "{note}");
    save.keep(DROP, 70_000, &recording(5, 70_000)).unwrap();
    drop(save);

    let mut bad = std::fs::read(&path).unwrap();
    bad[group::common_offset() + 0x20] ^= 0x08;
    std::fs::write(&path, &bad).unwrap();
    let (mut save, _) = boot(&path);
    assert_eq!(save.reread(&schema(), true), GhostsRead::Corrupt, "Retry: still damaged");
    save.write_new().unwrap();
    drop(save);
    assert_eq!(std::fs::read(dir.join("ssxvghosts.unread1")).unwrap(), bad);
    assert_eq!(std::fs::read(&path).unwrap(), fresh);

    let (mut save, _) = boot(&path);
    let good = save_with(&dir, DROP, 64_000);
    std::fs::write(&path, &good).unwrap();
    assert_eq!(save.reread(&schema(), true), GhostsRead::Loaded);
    assert_eq!(save.own_slot(DROP, 64_000), Some(0));
    drop(save);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn save_with(dir: &Path, drop: u32, score: u32) -> Vec<u8> {
    let other = dir.join("with");
    std::fs::create_dir_all(&other).unwrap();
    let path = other.join("ssxvghosts");
    let (mut save, _) = boot(&path);
    save.keep(drop, score, &recording(4, score)).unwrap();
    std::mem::drop(save);
    std::fs::read(&path).unwrap()
}

#[test]
fn without_saving_a_kept_ghost_lives_in_memory_until_a_load() {
    let dir = folder("no-saving");
    let path = dir.join("ssxvghosts");
    std::fs::write(&path, save_with(&dir, DROP, 70_000)).unwrap();
    let on_disk = std::fs::read(&path).unwrap();
    let (mut save, _) = boot(&path);
    save.not_loaded();
    assert!(save.own_slot(DROP, 70_000).is_none(), "no load filled the chunk");
    assert_eq!(save.keep_in_memory(OTHER, 5_000).unwrap(), 0);
    assert_eq!(save.own_slot(OTHER, 5_000), Some(0));
    assert_eq!(std::fs::read(&path).unwrap(), on_disk, "nothing written");
    let SlotLoad::Loaded(replay) = save.load_slot(0, &schema()) else { panic!("slot 0 loads") };
    assert_eq!(replay.header.score, 70_000);
    assert!(save.own_slot(OTHER, 5_000).is_none());
    assert_eq!(save.own_slot(DROP, 70_000), Some(0));
    drop(save);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_frame_never_waits_for_the_ghosts_writer() {
    use std::sync::mpsc;
    let dir = folder("held");
    let path = dir.join("ssxvghosts");
    let (go, wait) = mpsc::channel::<()>();
    let (done, written) = mpsc::channel::<(&'static str, String)>();
    let held = move |job: WriteJob, aside: &Mutex<Option<String>>| {
        let _ = wait.recv();
        let name = std::thread::current().name().unwrap_or_default().to_string();
        let why = job.why;
        write_job(job, aside);
        let _ = done.send((why, name));
    };
    let (mut save, _) = GhostSave::load_with(path.clone(), &schema(), held).unwrap();
    assert_eq!(save.keep(DROP, 70_000, &recording(5, 70_000)).unwrap(), 0);
    assert!(save.writing());
    assert_eq!(save.keep(OTHER, 9_000, &recording(3, 9_000)).unwrap(), 1);
    assert!(own(&mut save, DROP, Some(70_000)).is_some(), "the slot's ghost from the bytes in memory");
    assert_eq!(save.reread(&schema(), true), GhostsRead::Loaded, "Retry reads what the writer was handed");
    assert_eq!(save.own_slot(OTHER, 9_000), Some(1));
    save.write_new().unwrap();
    save.keep(OTHER, 8_000, &recording(3, 8_000)).unwrap();
    assert!(!path.exists(), "nothing written yet: every call above returned without the disk");
    go.send(()).unwrap();
    go.send(()).unwrap();
    drop(save);
    let order: Vec<(&str, String)> = written.try_iter().collect();
    assert_eq!(order.iter().map(|(w, _)| *w).collect::<Vec<_>>(), ["a kept run", "a kept run"]);
    assert!(order.iter().all(|(_, t)| t == "ghosts-save"), "{order:?}");
    let (mut save, note) = boot(&path);
    assert!(note.contains("own ghosts of 2 drops"), "{note}");
    assert!(own(&mut save, OTHER, Some(8_000)).is_some());
    drop(save);
    std::fs::remove_dir_all(&dir).unwrap();
}
