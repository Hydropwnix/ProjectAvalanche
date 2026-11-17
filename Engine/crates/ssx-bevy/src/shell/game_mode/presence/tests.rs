use super::*;

fn lines(details: &str, state: Option<&str>, competing: bool, timer: Timer) -> Lines {
    Lines { details: details.into(), state: state.map(str::to_string), competing, timer }
}

#[test]
fn the_activity_has_the_lines_the_logo_and_both_buttons() {
    let a = activity(
        &lines("WORLD TOUR · RACE IT!", Some("Corkscrew, Kilimanjaro"), false, Timer::Run),
        Some(1_763_250_000),
    );
    assert_eq!(
        a.to_json(),
        serde_json::json!({
            "type": 0,
            "details": "WORLD TOUR · RACE IT!",
            "state": "Corkscrew, Kilimanjaro",
            "timestamps": { "start": 1_763_250_000u64 },
            "assets": { "large_image": "logo", "large_text": LARGE_TEXT },
            "buttons": [
                { "label": "Get Project Avalanche", "url": "https://github.com/MnkyArts/ProjectAvalanche/releases" },
                { "label": "Join the Discord", "url": "https://discord.com/invite/XrFGQEfwdJ" },
            ],
        })
    );
    let a = activity(&lines("Online · Race", None, true, Timer::Run), None);
    let j = a.to_json();
    assert_eq!(j["type"], 5);
    assert!(j.get("state").is_none() && j.get("timestamps").is_none());
    assert_eq!(a.buttons().len(), 2);
}

#[test]
fn a_hostile_line_is_cleaned_and_bounded() {
    let hostile = format!("\u{202E}Big\u{0}Air\u{200B} {}", "x".repeat(500));
    let a = activity(&lines("Map · Race", Some(&hostile), false, Timer::Run), None);
    let state = a.state.as_ref().map(|t| t.as_str().to_string()).unwrap();
    assert!(state.starts_with("BigAir x") && state.len() <= 128, "{state:?}");
}

#[test]
fn the_constants_are_valid() {
    assert!(ssx_discord::payload::valid_client_id(APPLICATION_ID));
    assert!(AssetKey::new(LARGE_IMAGE).is_some());
    assert!(BUTTONS.iter().all(|(l, u)| Button::new(l, u).is_some()));
}

#[cfg(unix)]
#[test]
fn off_opens_no_socket_and_on_connects() {
    use std::os::unix::net::UnixListener;
    use std::time::{Duration, Instant};
    let dir = std::env::temp_dir().join(format!("sp-off-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("discord-ipc-0");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = Config { client_id: APPLICATION_ID.into(), endpoints: vec![path.clone()], timing: Timing::default() };
    assert!(client(DiscordPresence::Off, config.clone()).is_none());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(listener.accept().map(|_| ()).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    let on = client(DiscordPresence::On, config).unwrap().unwrap();
    let started = Instant::now();
    let connected = loop {
        match listener.accept() {
            Ok(_) => break true,
            Err(_) if started.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break false,
        }
    };
    assert!(connected);
    assert!(on.stop(Duration::from_secs(5)));
    let _ = std::fs::remove_dir_all(&dir);
}
