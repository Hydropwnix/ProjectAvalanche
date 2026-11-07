use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use ssx_files::{Options, replace};

const WRITERS: u8 = 4;
const SIZE: usize = 64 << 10;
const DEADLINE: Duration = Duration::from_secs(120);

fn content(byte: u8) -> Vec<u8> {
    vec![byte; SIZE]
}

fn whole(bytes: &[u8]) -> bool {
    bytes.len() == SIZE && bytes.iter().all(|&b| b == bytes[0]) && (b'a'..=b'a' + WRITERS).contains(&bytes[0])
}

fn writer(path: &Path, byte: u8) {
    for _ in 0..50 {
        replace(path, &content(byte), &Options::new()).expect("a write");
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .expect("a listing")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into())
        .collect();
    out.sort();
    out
}

fn main() {
    if let (Some(path), Some(byte)) =
        (std::env::var_os("SSX_FILES_WRITER_PATH"), std::env::var("SSX_FILES_WRITER_BYTE").ok())
    {
        writer(Path::new(&path), byte.parse().expect("a byte"));
        return;
    }
    let mut args = libtest_mimic::Arguments::from_args();
    args.test_threads = Some(1);
    let trial = libtest_mimic::Trial::test("two_processes", || {
        two_processes();
        Ok(())
    });
    libtest_mimic::run(&args, vec![trial]).exit();
}

fn two_processes() {
    let dir = std::env::temp_dir().join(format!("ssx-files-processes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path: PathBuf = dir.join("settings.json");
    replace(&path, &content(b'a' + WRITERS), &Options::new()).expect("the first version");
    let exe = std::env::current_exe().expect("this program");
    let mut children: Vec<Child> = (0..WRITERS)
        .map(|w| {
            Command::new(&exe)
                .env("SSX_FILES_WRITER_PATH", &path)
                .env("SSX_FILES_WRITER_BYTE", (b'a' + w).to_string())
                .spawn()
                .expect("a writer process")
        })
        .collect();
    let start = Instant::now();
    let mut reads = 0u64;
    let mut running = children.len();
    while running > 0 {
        let bytes = std::fs::read(&path).expect("the file is always there");
        assert!(whole(&bytes), "a read saw {} bytes, not one writer's whole content", bytes.len());
        reads += 1;
        running = 0;
        for c in &mut children {
            match c.try_wait().expect("a writer's state") {
                Some(status) => assert!(status.success(), "a writer failed: {status}"),
                None => running += 1,
            }
        }
        if start.elapsed() > DEADLINE {
            children.iter_mut().for_each(|c| drop(c.kill()));
            panic!("the writers did not finish in {} s", DEADLINE.as_secs());
        }
    }
    assert!(whole(&std::fs::read(&path).expect("the last version")));
    assert_eq!(names(&dir), ["settings.json"], "no partial file left");
    std::fs::remove_dir_all(&dir).expect("cleaned up");
    println!("two_processes: {WRITERS} writers x 50, {reads} reads while they wrote, every one whole");
}
