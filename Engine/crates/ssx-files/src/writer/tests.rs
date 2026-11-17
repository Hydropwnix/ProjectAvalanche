use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use super::*;

fn held() -> (Writer<&'static str>, Sender<()>, Receiver<(&'static str, String)>) {
    let (go, wait) = mpsc::channel::<()>();
    let (done, written) = mpsc::channel();
    let writer = Writer::start("test-save", move |job: &'static str| {
        let _ = wait.recv();
        let name = std::thread::current().name().unwrap_or_default().to_string();
        let _ = done.send((job, name));
    })
    .unwrap();
    (writer, go, written)
}

fn taken<J: Send + 'static>(writer: &Writer<J>) {
    while writer.shared.lock().next.is_some() {
        std::thread::yield_now();
    }
}

#[test]
fn a_job_is_written_on_the_writer_while_the_asker_goes_on() {
    let (writer, go, written) = held();
    assert!(matches!(writer.ask("the front end's save"), Asked::Queued));
    assert!(writer.writing(), "asked, not yet written");
    assert!(written.try_recv().is_err(), "the ask returned before the write");
    go.send(()).unwrap();
    writer.settle();
    assert!(!writer.writing());
    assert_eq!(written.try_recv().unwrap(), ("the front end's save", "test-save".to_string()));
}

#[test]
fn a_waiting_job_gives_way_to_a_newer_one() {
    let (writer, go, written) = held();
    writer.ask("a");
    taken(&writer);
    assert!(matches!(writer.ask("b"), Asked::Queued));
    assert!(matches!(writer.ask("c"), Asked::Replaced("b")));
    go.send(()).unwrap();
    go.send(()).unwrap();
    writer.settle();
    let order: Vec<&str> = written.try_iter().map(|(job, _)| job).collect();
    assert_eq!(order, ["a", "c"]);
}

#[test]
fn the_exit_writes_every_job_asked_for() {
    let (writer, go, written) = held();
    writer.ask("results");
    let opener = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        go.send(()).unwrap();
    });
    drop(writer);
    opener.join().unwrap();
    assert_eq!(written.try_recv().unwrap().0, "results");
}

#[test]
fn a_finished_writer_refuses_a_job() {
    let (mut writer, go, written) = held();
    writer.ask("last");
    go.send(()).unwrap();
    assert!(writer.finish());
    assert_eq!(written.try_recv().unwrap().0, "last");
    assert!(matches!(writer.ask("late"), Asked::Ended("late")));
    assert!(!writer.writing());
}

#[test]
fn a_panicking_write_ends_the_writer_without_a_hang() {
    let mut writer = Writer::start("test-save", |job: &'static str| panic!("test: {job}")).unwrap();
    writer.ask("first");
    writer.settle();
    assert!(!writer.writing());
    assert!(matches!(writer.ask("second"), Asked::Ended("second")));
    assert!(!writer.writing(), "nothing waits for a writer that has ended");
    assert!(!writer.finish(), "the worker ended with a panic");
}
