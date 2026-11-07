use std::sync::{Barrier, Mutex};

use super::*;

#[derive(Default)]
struct Fake {
    panic_always: bool,
    remounts: Mutex<usize>,
    hold: Option<Barrier>,
}

impl Reading for Fake {
    fn read(&self, command: &ResourceCommand, _: Vec<Started>, _: bool) -> Result<Read, String> {
        if let Some(b) = &self.hold {
            b.wait();
        }
        if self.panic_always || *command == ResourceCommand::Start("boom".into()) {
            panic!("boom panics in its read");
        }
        Err(format!("read {command:?}"))
    }

    fn remount(&self, _: &[Started]) -> Result<(), String> {
        *self.remounts.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        Ok(())
    }
}

fn answer(r: &mut Reader) -> Answer {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(a) = r.poll() {
            return a;
        }
        assert!(Instant::now() < end, "no answer within 10 s");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn boom() -> ResourceCommand {
    ResourceCommand::Start("boom".into())
}

#[test]
fn a_read_that_panics_fails_its_command_and_a_new_thread_mounts_the_running_set_first() {
    let fake = Arc::new(Fake::default());
    let mut r = Reader::start(Arc::clone(&fake) as Arc<dyn Reading>).expect("starts");
    let t0 = Instant::now();
    assert!(r.read(boom(), Vec::new(), false, t0).is_ok());
    match answer(&mut r) {
        Answer::Ended(Some(c), why) => {
            assert_eq!(c, boom(), "the failure is the command's");
            assert!(why.starts_with("the reading thread ended without its result"), "{why}");
            assert!(why.contains("boom panics in its read"), "{why}");
        }
        _ => panic!("the thread's end is reported"),
    }
    assert!(!r.busy() && r.needs_remount());
    assert!(r.remount(Vec::new(), t0).is_ok());
    assert_eq!(r.started(), 2, "a new thread");
    assert!(matches!(answer(&mut r), Answer::Remounted(Ok(()))));
    assert_eq!(*fake.remounts.lock().unwrap_or_else(PoisonError::into_inner), 1);
    assert!(!r.needs_remount());
    assert!(r.read(ResourceCommand::Refresh, Vec::new(), true, t0).is_ok());
    match answer(&mut r) {
        Answer::Read(ResourceCommand::Refresh, Err(e)) => assert_eq!(e, "read Refresh"),
        _ => panic!("the next command is read"),
    }
    assert_eq!(r.started(), 2, "by the same thread");
}

#[test]
fn the_replacements_are_bounded_within_their_window() {
    let fake = Arc::new(Fake { panic_always: true, ..Fake::default() });
    let mut r = Reader::start(Arc::clone(&fake) as Arc<dyn Reading>).expect("starts");
    let t0 = Instant::now();
    for i in 0..=MAX_RESTARTS {
        assert!(r.read(boom(), Vec::new(), false, t0).is_ok(), "read {i}");
        assert!(matches!(answer(&mut r), Answer::Ended(Some(_), _)));
        if i < MAX_RESTARTS {
            assert!(r.remount(Vec::new(), t0).is_ok(), "replacement {}", i + 1);
            assert!(matches!(answer(&mut r), Answer::Remounted(Ok(()))));
        }
    }
    assert_eq!(r.started(), 1 + MAX_RESTARTS as u64);
    let refused = r.remount(Vec::new(), t0 + RESTART_WINDOW / 2).expect_err("past the bound");
    assert!(refused.contains("refused"), "{refused}");
    assert!(r.needs_remount() && !r.busy(), "nothing in a thread, the remount still owed");
    assert!(r.remount(Vec::new(), t0 + RESTART_WINDOW).is_ok());
    assert!(matches!(answer(&mut r), Answer::Remounted(Ok(()))));
    assert_eq!(r.started(), 2 + MAX_RESTARTS as u64);
}

#[test]
fn dropping_the_reader_ends_its_thread_after_the_read_it_is_in() {
    let fake = Arc::new(Fake { hold: Some(Barrier::new(2)), ..Fake::default() });
    let mut r = Reader::start(Arc::clone(&fake) as Arc<dyn Reading>).expect("starts");
    assert!(r.read(ResourceCommand::Refresh, Vec::new(), true, Instant::now()).is_ok());
    drop(r);
    assert_eq!(Arc::strong_count(&fake), 2, "the thread is in its read");
    if let Some(b) = &fake.hold {
        b.wait();
    }
    let end = Instant::now() + Duration::from_secs(10);
    while Arc::strong_count(&fake) > 1 {
        assert!(Instant::now() < end, "the thread ends once its read is done");
        std::thread::sleep(Duration::from_millis(1));
    }
}
