use super::*;

const CHILD: &str = "SSX_CORE_MASTER_CHILD";
const VARIABLE: &str = "SSX_DEV_MASTER";

#[test]
fn child_reports_the_master() {
    if std::env::var_os(CHILD).is_some() {
        println!("MASTER={}", master().url().unwrap_or("none"));
    }
}

fn in_a_child(value: Option<&str>) -> String {
    use std::io::Read;
    use std::process::Stdio;
    let exe = std::env::current_exe().expect("the test binary");
    let mut command = std::process::Command::new(exe);
    command.args(["master::tests::child_reports_the_master", "--exact", "--nocapture", "--test-threads=1"]);
    command.env(CHILD, "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    match value {
        Some(v) => command.env(VARIABLE, v),
        None => command.env_remove(VARIABLE),
    };
    let mut child = command.spawn().expect("the child runs");
    let mut out = child.stdout.take().expect("its output");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = out.read_to_string(&mut text);
        text
    });
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().expect("the child's state") {
            break status;
        }
        if std::time::Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the child did not end within 60 s");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stdout = reader.join().expect("its output read");
    assert!(status.success(), "the child failed: {status}");
    stdout
        .lines()
        .find_map(|l| l.split_once("MASTER=").map(|(_, v)| v.to_string()))
        .unwrap_or_else(|| panic!("the child reported no master: {stdout}"))
}

#[cfg(not(feature = "dev-master"))]
#[test]
fn a_release_build_has_the_projects_master_whatever_the_environment_says() {
    assert_eq!(master().url(), Some(OFFICIAL_MASTER));
    assert!(master().is_official());
    assert_eq!(master().describe(), "https://masterlist.liamrbsn.com (the project's master server)");
    for value in [None, Some("http://127.0.0.1:27900"), Some("http://[::1]:9"), Some("https://other.example.org")] {
        assert_eq!(in_a_child(value), OFFICIAL_MASTER, "{value:?}");
    }
}

#[cfg(feature = "dev-master")]
#[test]
fn a_development_build_has_none_or_a_master_on_this_computer() {
    use std::ffi::OsStr;
    assert_eq!(dev_master(None).url(), None);
    assert!(!dev_master(None).is_official());
    assert!(dev_master(None).describe().contains("SSX_DEV_MASTER names no master"));
    for (given, taken) in [
        ("http://127.0.0.1:27900", "http://127.0.0.1:27900"),
        ("http://127.3.2.1:1", "http://127.3.2.1:1"),
        ("http://[::1]:65535", "http://[::1]:65535"),
        ("http://[0:0:0:0:0:0:0:1]:8", "http://[::1]:8"),
    ] {
        let m = dev_master(Some(OsStr::new(given)));
        assert_eq!(m.url(), Some(taken), "{given}");
        assert!(!m.is_official());
    }
    for refused in [
        "",
        "127.0.0.1:27900",
        "https://127.0.0.1:27900",
        "HTTP://127.0.0.1:27900",
        OFFICIAL_MASTER,
        "http://masterlist.liamrbsn.com:80",
        "http://localhost:27900",
        "http://10.0.0.1:27900",
        "http://192.168.1.2:27900",
        "http://0.0.0.0:27900",
        "http://[::]:27900",
        "http://[::ffff:127.0.0.1]:27900",
        "http://127.0.0.1",
        "http://127.0.0.1:",
        "http://127.0.0.1:0",
        "http://127.0.0.1:65536",
        "http://127.0.0.1:080",
        "http://127.0.0.1:+80",
        "http://127.0.0.1:27900/",
        "http://127.0.0.1:27900/v1/servers",
        "http://user@127.0.0.1:27900",
        "http://127.000.0.1:27900",
        "http://[::1:27900",
        "http://[::1]27900",
        " http://127.0.0.1:27900",
        "http://127.0.0.1:27900\n",
    ] {
        let m = dev_master(Some(OsStr::new(refused)));
        assert_eq!(m.url(), None, "{refused:?} taken");
        assert!(m.describe().contains("refused"), "{refused:?}: {}", m.describe());
    }
    let long = format!("http://\u{1b}[2J{}", "a".repeat(5000));
    let shown = dev_master(Some(OsStr::new(&long))).describe();
    assert!(shown.len() < 400 && !shown.contains('\u{1b}'), "{shown}");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert_eq!(dev_master(Some(OsStr::from_bytes(b"http://127.0.0.1:1\xff"))).url(), None, "not UTF-8");
    }
}

#[cfg(feature = "dev-master")]
#[test]
fn a_development_process_takes_the_variable_it_started_with() {
    assert_eq!(in_a_child(None), "none");
    assert_eq!(in_a_child(Some("http://127.0.0.1:27900")), "http://127.0.0.1:27900");
    assert_eq!(in_a_child(Some(OFFICIAL_MASTER)), "none", "the project's master is never a development master");
    assert_eq!(in_a_child(Some("http://203.0.113.5:27900")), "none");
}
