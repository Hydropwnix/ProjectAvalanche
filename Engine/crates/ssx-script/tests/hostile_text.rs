use ssx_core::text::{LINE_CHARS, is_hidden};
use ssx_script::host::package::PackageScripts;
use ssx_script::host::{HostConfig, ScriptHost, Side};

const H: &str = "\u{1b}]0;PWNED\u{7}\u{1b}[2J\u{9b}31m\u{202e}evil\u{2066}\u{200b}\u{feff}\u{2028}\u{2029}ok\u{e0041}";

fn lua_h() -> String {
    format!("\"{H}\\r\\nforged: ok\"")
}

fn clean(what: &str, line: &str, max: usize) {
    assert!(!line.chars().any(is_hidden), "{what}: a character the rule escapes in {:?}", cut(line));
    assert!(line.chars().count() <= max, "{what}: {} characters", line.chars().count());
}

fn cut(m: &str) -> String {
    m.chars().take(200).collect()
}

fn package(tag: &str, files: &[(&str, String)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ssx-script-hostile-text-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, text) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("folder");
        std::fs::write(p, text).expect("write");
    }
    dir
}

const MANIFEST: &str = "[resource]\nname = \"hx\"\nversion = \"1.0.0\"\napi = 1\n[scripts]\nclient = [\"client/*.luau\"]\n\
                        [files]\ninclude = [\"lib/*\"]\n";

#[test]
fn the_check_quotes_hostile_source_escaped() {
    let big = H.repeat((400 << 10) / H.len());
    let dir = package(
        "check",
        &[
            ("resource.toml", MANIFEST.to_string()),
            ("client/syntax.luau", format!("local x = 1\nlocal {H} = 2\n")),
            ("client/string.luau", format!("local s = \"{H}\nreturn s\n")),
            ("client/escaped.luau", format!("local s = {}\nlocal t = require({})\n", lua_h(), lua_h())),
            ("client/call.luau", format!("{H}(1)\n")),
            ("client/require.luau", format!("local a = require(\"{H}\")\nlocal b = require(\"lib/{H}\")\n")),
            ("client/big.luau", format!("local b = require(\"{big}\")\nlocal {big} = 1\n")),
            ("lib/ok.luau", "return 1\n".to_string()),
        ],
    );
    let src = ssx_content::PackageSource::open(&dir).expect("package");
    let findings = ssx_script::check::check_package(&src);
    let _ = std::fs::remove_dir_all(&dir);
    let mut quoting = 0;
    for f in &findings {
        clean("finding", &f.message, LINE_CHARS);
        let row = ssx_core::print::row_line(&[&f.severity, &f.file, &f.line, &f.column, &f.message]);
        let fields: Vec<&str> = row.split('\t').collect();
        assert_eq!(fields.len(), 5, "one row a finding: {}", cut(&row));
        fields.iter().for_each(|field| clean("row", field, LINE_CHARS));
        quoting += usize::from(f.message.contains("PWNED") || f.message.contains("\\u{1b}"));
    }
    let errors = findings.iter().filter(|f| f.severity == "error").count();
    assert!(errors >= 4 && quoting >= 3, "{errors} errors, {quoting} quoting: {findings:?}");
}

#[test]
fn print_error_and_stops_are_bounded_and_shown_escaped() {
    let big = H.repeat((400 << 10) / H.len());
    let main = format!(
        "print({h})\nprint(\"{big}\")\nprint(pcall(require, {h}))\nprint(pcall(error, \"{big}\"))\n\
         print(\"{H}\", {h}, 1, nil)\nerror({h})\n",
        h = lua_h()
    );
    let dir = package("run", &[("resource.toml", MANIFEST.to_string()), ("client/main.luau", main)]);
    let src = ssx_content::PackageSource::open(&dir).expect("package");
    let host = ScriptHost::new(HostConfig::new(Side::Client)).expect("host");
    let pkg = PackageScripts::from_source(&src, Side::Client, host.limits()).expect("scripts");
    let _ = std::fs::remove_dir_all(&dir);
    let started = host.start(pkg);
    let last = host.take_last_output().into_iter().filter(|(name, _)| name == "hx").flat_map(|(_, o)| o.lines);
    let out: Vec<String> =
        host.take_output("hx").map(|o| o.lines).unwrap_or_default().into_iter().chain(last).collect();
    assert!(out.len() >= 4, "{out:?}");
    for line in &out {
        assert!(line.len() <= ssx_script::MAX_OUTPUT_LINE + 64, "{} bytes", line.len());
    }
    assert!(out.iter().any(|l| l.contains('\u{1b}')), "the output is the script's own");
    let error = started.err().unwrap_or_default();
    assert!(error.len() <= ssx_script::MAX_ERROR_LEN + 64, "{} bytes", error.len());
    for line in out.iter().chain([&error]) {
        clean("shown", &ssx_core::text::shown_max(line, 300).to_string(), 300 + 32);
    }
    host.stop("hx", H);
    let report = host.monitor().render();
    for line in report.lines() {
        clean("monitor", line, LINE_CHARS + 64);
    }
    assert!(report.contains("stopped hx") && report.contains("PWNED"), "the stop's reason is shown: {report}");
}
