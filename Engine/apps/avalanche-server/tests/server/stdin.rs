use std::net::Ipv4Addr;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use avalanche_server::{ServerOptions, ServerThread};

use crate::common::temp_dir;

fn start(input: std::io::PipeReader, stop_at_end: bool) -> ServerThread {
    let mut config = crate::common::defaults();
    config.bind = Ipv4Addr::LOCALHOST.into();
    config.port = 0;
    let options = ServerOptions {
        config,
        data_dir: temp_dir("stdin"),
        console: false,
        logging: false,
        log_to_file: false,
        signals: false,
        conditions: None,
        relay_rates: None,
        extend: Some(Box::new(move |app| {
            avalanche_server::console::attach(app, input, stop_at_end).expect("the console's thread");
        })),
        rules: avalanche_server::rules::RulesSource::Unchecked,
    };
    avalanche_server::spawn(options).expect("the server starts")
}

fn ends_within(server: ServerThread, within: Duration) -> Option<Result<(), String>> {
    let shared = std::sync::Arc::clone(&server.started.shared);
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(server.join());
    });
    match rx.recv_timeout(within) {
        Ok(result) => Some(result),
        Err(_) => {
            shared.stop_requested.store(true, Ordering::Release);
            let _ = rx.recv_timeout(Duration::from_secs(10));
            None
        }
    }
}

#[test]
fn the_end_of_the_input_stops_a_server_run_with_stop_with_input() {
    let (input, held) = std::io::pipe().expect("a pipe");
    let server = start(input, true);
    drop(held);
    assert_eq!(ends_within(server, Duration::from_secs(10)), Some(Ok(())), "the server stops gracefully");
}

#[test]
fn without_the_flag_the_end_of_the_input_ends_only_the_console() {
    let (input, held) = std::io::pipe().expect("a pipe");
    let server = start(input, false);
    drop(held);
    let until = Instant::now() + Duration::from_secs(2);
    while Instant::now() < until {
        assert!(!server.started.shared.stop_requested.load(Ordering::Acquire), "without the flag the server runs on");
        std::thread::sleep(Duration::from_millis(50));
    }
    server.stop();
    assert_eq!(ends_within(server, Duration::from_secs(10)), Some(Ok(())));
}

#[test]
fn the_binary_takes_the_flag_and_refuses_a_start_without_the_rules_data() {
    let dir = temp_dir("stdin-bin");
    let config = dir.join("server.toml");
    std::fs::write(&config, "[server]\nbind = \"127.0.0.1\"\n").expect("server.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_avalanche-server"))
        .arg("run")
        .arg("--stop-with-input")
        .arg("--config")
        .arg(&config)
        .arg("--data")
        .arg(dir.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .spawn()
        .expect("the binary starts");
    let until = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break Some(status);
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut said = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut err, &mut said);
    }
    let status = status.expect("the binary ends without its rules data");
    assert!(!status.success(), "{status}");
    assert!(said.contains("[data] folder") && said.contains("avalanche-server setup"), "{said}");
}
