use std::collections::BTreeMap;

use super::{Finding, finding};

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Name(String),
    Text(Option<String>),
    Punct(u8),
    Other,
}

fn tokens(src: &str) -> Vec<(usize, Token)> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0usize, 1usize, Vec::new());
    let long_open = |at: usize| -> Option<usize> {
        let mut k = at + 1;
        while b.get(k) == Some(&b'=') {
            k += 1;
        }
        (b.get(at) == Some(&b'[') && b.get(k) == Some(&b'[')).then_some(k - at - 1)
    };
    let skip_long = |at: usize, level: usize, line: &mut usize| -> (usize, String) {
        let start = (at + level + 2).min(src.len());
        let close = format!("]{}]", "=".repeat(level));
        let end = src[start..].find(&close).map_or(src.len(), |k| start + k);
        let text = src[start..end].to_string();
        *line += text.matches('\n').count();
        ((end + close.len()).min(src.len()), text)
    };
    while i < b.len() {
        let c = b[i];
        let at = line;
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i += 2;
                match long_open(i) {
                    Some(level) => i = skip_long(i, level, &mut line).0,
                    None => {
                        while i < b.len() && b[i] != b'\n' {
                            i += 1;
                        }
                    }
                }
            }
            b'"' | b'\'' => {
                let mut k = i + 1;
                let mut escaped = false;
                while k < b.len() && b[k] != c && b[k] != b'\n' {
                    if b[k] == b'\\' {
                        escaped = true;
                        if b.get(k + 1) == Some(&b'\n') {
                            line += 1;
                        }
                        k += 1;
                    }
                    k += 1;
                }
                let text = src.get(i + 1..k.min(b.len())).unwrap_or_default().to_string();
                out.push((at, Token::Text((!escaped).then_some(text))));
                i = (k + 1).min(b.len());
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
                out.push((at, Token::Text(None)));
                i = (k + 1).min(b.len());
            }
            b'[' if long_open(i).is_some() => {
                let (after, text) = skip_long(i, long_open(i).unwrap_or(0), &mut line);
                out.push((at, Token::Text(Some(text))));
                i = after;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push((at, Token::Name(src[start..i].to_string())));
            }
            c if c.is_ascii_digit() => {
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.' || b[i] == b'_') {
                    i += 1;
                }
                out.push((at, Token::Other));
            }
            c if c.is_ascii_whitespace() => i += 1,
            b'(' | b')' | b'{' | b'}' | b'[' | b']' | b',' | b'.' | b':' => {
                out.push((at, Token::Punct(c)));
                i += 1;
            }
            _ => {
                out.push((at, Token::Other));
                i += 1;
            }
        }
    }
    out
}

struct Call {
    line: usize,
    args: Vec<Option<Token>>,
}

fn calls(toks: &[(usize, Token)], func: &str) -> Vec<Call> {
    let mut out = Vec::new();
    for (k, (line, t)) in toks.iter().enumerate() {
        if *t != Token::Name(func.to_string()) {
            continue;
        }
        if k > 0 && matches!(toks[k - 1].1, Token::Punct(b'.' | b':')) {
            continue;
        }
        match toks.get(k + 1).map(|t| &t.1) {
            Some(Token::Punct(b'(')) => {}
            Some(t @ Token::Text(_)) => {
                out.push(Call { line: *line, args: vec![Some(t.clone())] });
                continue;
            }
            _ => continue,
        }
        let (mut depth, mut args, mut current) = (0usize, Vec::new(), Vec::new());
        for (_, t) in &toks[k + 2..] {
            match t {
                Token::Punct(b'(' | b'{' | b'[') => {
                    depth += 1;
                    current.push(t.clone());
                }
                Token::Punct(b')' | b'}' | b']') if depth > 0 => {
                    depth -= 1;
                    current.push(t.clone());
                }
                Token::Punct(b')') => break,
                Token::Punct(b',') if depth == 0 => args.push(std::mem::take(&mut current)),
                _ => current.push(t.clone()),
            }
        }
        if !current.is_empty() {
            args.push(current);
        }
        let args = args.into_iter().map(|a| if a.len() == 1 { a.into_iter().next() } else { None }).collect();
        out.push(Call { line: *line, args });
    }
    out
}

#[derive(Default)]
pub(super) struct Side {
    remote: BTreeMap<String, (String, usize)>,
    local: BTreeMap<String, (String, usize)>,
    sends: Vec<(String, usize, Option<String>, &'static str)>,
}

impl Side {
    pub(super) fn scan(&mut self, path: &str, source: &str, send: &[(&'static str, usize)]) {
        let toks = tokens(source);
        for c in calls(&toks, "addEvent") {
            let Some(Some(Token::Text(Some(name)))) = c.args.first() else { continue };
            let remote = matches!(c.args.get(1), Some(Some(Token::Name(t))) if t == "true");
            let to = if remote { &mut self.remote } else { &mut self.local };
            to.entry(name.clone()).or_insert_with(|| (path.to_string(), c.line));
        }
        for (func, index) in send {
            for c in calls(&toks, func) {
                let name = match c.args.get(*index) {
                    Some(Some(Token::Text(Some(n)))) => Some(n.clone()),
                    _ => None,
                };
                self.sends.push((path.to_string(), c.line, name, func));
            }
        }
    }
}

pub(super) fn reached(p: &crate::host::package::PackageScripts) -> Vec<(String, &str)> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut queue: Vec<String> = p.scripts.iter().map(|(path, _)| path.clone()).collect();
    let mut out = Vec::new();
    while let Some(path) = queue.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let Some(source) = p.modules.get(&path) else { continue };
        for r in super::requires(source) {
            if let Some(found) = r.name.and_then(|n| crate::resolve(&p.modules, &n).ok()) {
                queue.push(found);
            }
        }
        out.push((path, source.as_str()));
    }
    out
}

pub(super) const CLIENT_SENDS: &[(&str, usize)] = &[("triggerServerEvent", 0), ("triggerLatentServerEvent", 0)];
pub(super) const SERVER_SENDS: &[(&str, usize)] = &[("triggerClientEvent", 1), ("triggerLatentClientEvent", 1)];

pub(super) fn check(client: &Side, server: &Side, out: &mut Vec<Finding>) {
    for (from, to, to_name) in [(client, server, "server"), (server, client, "client")] {
        for (path, line, name, func) in &from.sends {
            let Some(name) = name else {
                let message =
                    format!("{func} of a name that is not a string literal: its declaration is checked at run time");
                out.push(finding("note", path, *line, message));
                continue;
            };
            if to.remote.contains_key(name) {
                continue;
            }
            let why = if to.local.contains_key(name) { "declares it, but not remote" } else { "never declares it" };
            let message = format!(
                "{func}(\"{name}\"): the package's {to_name} side {why} (addEvent(\"{name}\", true)): the {to_name} \
                 refuses it"
            );
            out.push(finding("error", path, *line, message));
        }
    }
    for (name, (path, line)) in &server.remote {
        if !client.sends.iter().any(|(_, _, n, _)| n.as_deref().is_none_or(|n| n == name)) {
            let message = format!(
                "the event \"{name}\" is declared remote, but no client script of the package triggers it: every \
                 client may (declare it without `true` if only the server raises it)"
            );
            out.push(finding("warning", path, *line, message));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_and_their_literal_arguments_are_found() {
        let src = "addEvent('a', true) -- addEvent('no', true)\nx.addEvent('no')\naddEvent(\"b\")\n\
                   triggerClientEvent(getPlayers(), 'c', root, { 1, 2 })\ntriggerServerEvent(`d{x}`, me)\n\
                   triggerServerEvent [[e]]\nlocal s = \"addEvent('no', true)\"\n";
        let mut side = Side::default();
        side.scan("f.luau", src, SERVER_SENDS);
        assert_eq!(side.remote.keys().collect::<Vec<_>>(), ["a"]);
        assert_eq!(side.local.keys().collect::<Vec<_>>(), ["b"]);
        assert_eq!(side.sends, [("f.luau".to_string(), 4, Some("c".to_string()), "triggerClientEvent")]);
        let mut client = Side::default();
        client.scan("g.luau", src, CLIENT_SENDS);
        let sent: Vec<_> = client.sends.iter().map(|(_, l, n, _)| (*l, n.clone())).collect();
        assert_eq!(sent, [(5, None), (6, Some("e".to_string()))]);
    }

    #[test]
    fn a_send_the_other_side_does_not_declare_remote_is_an_error() {
        let mut client = Side::default();
        client.scan("client/main.luau", "addEvent('score', true)\ntriggerServerEvent('hello', me)\n", CLIENT_SENDS);
        let mut server = Side::default();
        server.scan(
            "server/main.luau",
            "addEvent('hello')\naddEvent('open', true)\ntriggerClientEvent(p, 'score', root)\n\
             triggerClientEvent(p, 'nobody', root)\n",
            SERVER_SENDS,
        );
        let mut out = Vec::new();
        check(&client, &server, &mut out);
        let got: Vec<_> = out.iter().map(|f| (f.severity, f.file.as_str(), f.line)).collect();
        assert_eq!(
            got,
            [("error", "client/main.luau", 2), ("error", "server/main.luau", 4), ("warning", "server/main.luau", 2)]
        );
        assert!(out[0].message.to_string().contains("declares it, but not remote"));
    }
}
