use ssx_content::PackageSource;

use crate::host::Side;
use crate::host::limits::HostLimits;
use crate::host::package::PackageScripts;

mod remote;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub severity: &'static str,
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub message: ssx_core::text::Text,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Require {
    pub line: usize,
    pub name: Option<String>,
}

pub fn requires(src: &str) -> Vec<Require> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0usize, 1usize, Vec::new());
    let mut prev_dot = false;
    let long_open = |at: usize| -> Option<usize> {
        let mut k = at + 1;
        while b.get(k) == Some(&b'=') {
            k += 1;
        }
        (b.get(at) == Some(&b'[') && b.get(k) == Some(&b'[')).then_some(k - at - 1)
    };
    let skip_long = |at: usize, level: usize, line: &mut usize| -> (usize, String) {
        let start = at + level + 2;
        let close = format!("]{}]", "=".repeat(level));
        let end = src[start.min(src.len())..].find(&close).map_or(src.len(), |k| start + k);
        let text = src[start.min(end)..end].to_string();
        *line += text.matches('\n').count();
        ((end + close.len()).min(src.len()), text)
    };
    let skip_quoted = |at: usize, q: u8, line: &mut usize| -> (usize, String) {
        let mut k = at + 1;
        while k < b.len() && b[k] != q && b[k] != b'\n' {
            if b[k] == b'\\' {
                if b.get(k + 1) == Some(&b'\n') {
                    *line += 1;
                }
                k += 1;
            }
            k += 1;
        }
        let text = src.get(at + 1..k.min(b.len())).unwrap_or_default().to_string();
        ((k + 1).min(b.len()), text)
    };
    let skip_space = |mut k: usize, line: &mut usize| -> usize {
        loop {
            match b.get(k) {
                Some(b'\n') => {
                    *line += 1;
                    k += 1;
                }
                Some(c) if c.is_ascii_whitespace() => k += 1,
                Some(b'-') if b.get(k + 1) == Some(&b'-') => {
                    k += 2;
                    match long_open(k) {
                        Some(level) => k = skip_long(k, level, line).0,
                        None => {
                            while k < b.len() && b[k] != b'\n' {
                                k += 1;
                            }
                        }
                    }
                }
                _ => return k,
            }
        }
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => i = skip_space(i, &mut line),
            b'"' | b'\'' => {
                i = skip_quoted(i, c, &mut line).0;
                prev_dot = false;
            }
            b'`' => {
                let mut k = i + 1;
                while k < b.len() && b[k] != b'`' {
                    if b[k] == b'\\' {
                        k += 1;
                    } else if b[k] == b'\n' {
                        line += 1;
                    }
                    k += 1;
                }
                i = (k + 1).min(b.len());
                prev_dot = false;
            }
            b'[' if long_open(i).is_some() => {
                let level = long_open(i).unwrap_or(0);
                i = skip_long(i, level, &mut line).0;
                prev_dot = false;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                if &src[start..i] == "require" && !prev_dot {
                    let at_line = line;
                    let mut ahead = line;
                    let mut k = skip_space(i, &mut ahead);
                    let paren = b.get(k) == Some(&b'(');
                    if paren {
                        k = skip_space(k + 1, &mut ahead);
                    }
                    let name = match b.get(k) {
                        Some(&q @ (b'"' | b'\'')) => {
                            let (after, text) = skip_quoted(k, q, &mut ahead);
                            let after = skip_space(after, &mut ahead);
                            let closed = !paren || b.get(after) == Some(&b')');
                            (closed && !text.contains('\\')).then_some(text)
                        }
                        Some(b'[') if long_open(k).is_some() => {
                            let (after, text) = skip_long(k, long_open(k).unwrap_or(0), &mut ahead);
                            let after = skip_space(after, &mut ahead);
                            (!paren || b.get(after) == Some(&b')')).then_some(text)
                        }
                        _ => None,
                    };
                    out.push(Require { line: at_line, name });
                }
                prev_dot = false;
            }
            b'.' | b':' => {
                prev_dot = true;
                i += 1;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => {
                prev_dot = false;
                i += 1;
            }
        }
    }
    out
}

fn finding(severity: &'static str, file: &str, line: usize, message: impl Into<String>) -> Finding {
    let message = ssx_core::text::Text::from(message.into());
    Finding { severity, file: file.to_string(), line, column: 0, message }
}

fn compile_error(path: &str, e: &str) -> (usize, String) {
    let rest = e.strip_prefix(path).and_then(|r| r.strip_prefix(':')).unwrap_or(e);
    match rest.split_once(':') {
        Some((n, m)) if n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty() => {
            (n.parse().unwrap_or(0), m.trim().to_string())
        }
        _ => (0, rest.trim().to_string()),
    }
}

pub fn check_package(src: &PackageSource) -> Vec<Finding> {
    let m = &src.manifest;
    let mut out = Vec::new();
    let level = crate::api::API_LEVEL;
    if m.resource.api > level {
        let message = format!("the package needs API level {}; this build has {level}", m.resource.api);
        out.push(finding("error", ssx_content::manifest::MANIFEST_FILE, 0, message));
    }
    for f in src.tree().files().filter(|f| f.ends_with(".d.luau")) {
        let side_matched = [&m.scripts.client, &m.scripts.server, &m.scripts.shared].iter().any(|list| {
            list.iter().any(|p| {
                ssx_content::glob::Pattern::parse(p)
                    .is_ok_and(|p| p.matches(f, &mut ssx_content::glob::Budget(1_000_000)).unwrap_or(false))
            })
        });
        if side_matched {
            let message = "a type definitions file is never a script or a module (left out): keep it out of [scripts]";
            out.push(finding("error", f, 0, message));
        }
    }
    let mut compiled = std::collections::BTreeSet::new();
    let (mut client_events, mut server_events) = (remote::Side::default(), remote::Side::default());
    for side in [Side::Client, Side::Server] {
        let limits = match side {
            Side::Client => HostLimits::CLIENT,
            Side::Server => HostLimits::SERVER,
        };
        let p = match PackageScripts::from_source(src, side, &limits) {
            Ok(p) => p,
            Err(e) => {
                out.push(finding(
                    "error",
                    ssx_content::manifest::MANIFEST_FILE,
                    0,
                    format!("{} side: {e}", side.name()),
                ));
                continue;
            }
        };
        let bytes: usize = p.modules.values().map(String::len).sum();
        out.push(finding(
            "note",
            "",
            0,
            format!(
                "{} side: {} scripts, {} modules, {bytes} bytes (at most {} files and {} bytes)",
                side.name(),
                p.scripts.len(),
                p.modules.len(),
                limits.source_files,
                limits.source_bytes
            ),
        ));
        for (path, source) in remote::reached(&p) {
            match side {
                Side::Client => client_events.scan(&path, source, remote::CLIENT_SENDS),
                Side::Server => server_events.scan(&path, source, remote::SERVER_SENDS),
            }
        }
        for (path, source) in &p.modules {
            if compiled.insert(path.clone())
                && let Err(e) = crate::compile::bytecode(path, source.as_bytes())
            {
                let (line, message) = compile_error(path, &e.0);
                out.push(finding("error", path, line, message));
            }
            for r in requires(source) {
                match &r.name {
                    Some(name) => {
                        if let Err(e) = crate::resolve(&p.modules, name) {
                            out.push(finding("error", path, r.line, format!("{} side: {e}", side.name())));
                        }
                    }
                    None => out.push(finding(
                        "note",
                        path,
                        r.line,
                        "a require of a computed name: resolved at run time (only the package's own modules)",
                    )),
                }
            }
        }
    }
    remote::check(&client_events, &server_events, &mut out);
    out.sort_by(|a, b| (&a.file, a.line, a.severity).cmp(&(&b.file, b.line, b.severity)));
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(src: &str) -> Vec<(usize, Option<String>)> {
        requires(src).into_iter().map(|r| (r.line, r.name)).collect()
    }

    #[test]
    fn requires_are_found_outside_comments_and_strings() {
        let src = "local a = require(\"lib/a\")\n-- require('no')\n--[[ require('no')\n]] local b = require 'lib/b'\n\
                   local s = \"require('no')\" local t = [==[ require('no') ]==]\nlocal c = require [[lib/c]]\n\
                   x.require('no') y:require('no')\nlocal d = require(name)\nlocal e = require(`lib/{x}`)\n\
                   local f = require(\n  'lib/f'\n)\n";
        assert_eq!(
            names(src),
            [
                (1, Some("lib/a".into())),
                (4, Some("lib/b".into())),
                (6, Some("lib/c".into())),
                (8, None),
                (9, None),
                (10, Some("lib/f".into())),
            ]
        );
        assert_eq!(names("local x = require('a' .. 'b')"), [(1, None)]);
        assert_eq!(names("require('unterminated"), [(1, None)]);
        assert_eq!(names("--[==[ unclosed"), []);
    }

    #[test]
    fn compile_errors_give_their_line() {
        assert_eq!(compile_error("a/b.luau", "a/b.luau:3: Expected identifier"), (3, "Expected identifier".into()));
        assert_eq!(compile_error("a/b.luau", "something else"), (0, "something else".into()));
    }
}
