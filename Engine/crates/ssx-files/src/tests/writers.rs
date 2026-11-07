use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::Scratch;
use crate::{Error, Options, replace, update};

fn content(writer: u8) -> Vec<u8> {
    vec![b'a' + writer; 64 << 10]
}

fn whole(bytes: &[u8], writers: u8) -> bool {
    bytes.len() == 64 << 10 && bytes.iter().all(|&b| b == bytes[0]) && (b'a'..b'a' + writers).contains(&bytes[0])
}

#[test]
fn writers_at_once_leave_one_whole_content() {
    const WRITERS: u8 = 8;
    let s = Scratch::new("writers");
    let path = Arc::new(s.join("settings.json"));
    replace(&path, &content(0), &Options::new()).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (path, stop) = (Arc::clone(&path), Arc::clone(&stop));
        std::thread::spawn(move || {
            let mut reads = 0u32;
            while !stop.load(Ordering::Relaxed) {
                let bytes = std::fs::read(&*path).expect("the file is always there");
                assert!(whole(&bytes, WRITERS), "a read saw {} bytes, not one writer's whole content", bytes.len());
                reads += 1;
            }
            reads
        })
    };
    let writers: Vec<_> = (0..WRITERS)
        .map(|w| {
            let path = Arc::clone(&path);
            std::thread::spawn(move || {
                for _ in 0..40 {
                    replace(&path, &content(w), &Options::new()).expect("a write");
                }
            })
        })
        .collect();
    for w in writers {
        w.join().expect("a writer");
    }
    stop.store(true, Ordering::Relaxed);
    assert!(reader.join().expect("the reader") > 0);
    assert!(whole(&std::fs::read(&*path).unwrap(), WRITERS));
    assert_eq!(s.names(), ["settings.json"]);
}

#[test]
fn updates_at_once_lose_nothing() {
    let s = Scratch::new("updates");
    let path = Arc::new(s.join("exports.tsv"));
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let path = Arc::clone(&path);
            std::thread::spawn(move || {
                for i in 0..25 {
                    update(&path, &Options::new(), 1 << 20, |old| -> Result<Vec<u8>, Error> {
                        let mut text = old.unwrap_or_default();
                        text.extend_from_slice(format!("{t} {i}\n").as_bytes());
                        Ok(text)
                    })
                    .expect("an update");
                }
            })
        })
        .collect();
    for t in threads {
        t.join().expect("an updater");
    }
    let text = std::fs::read_to_string(&*path).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines.len(), 200, "every update kept");
    assert_eq!(s.names(), ["exports.tsv"]);
}
