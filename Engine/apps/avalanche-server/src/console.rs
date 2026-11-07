use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

const MAX_LINE: usize = 1024;
const _: () = assert!(MAX_LINE == ssx_script::host::server::rights::MAX_LINE);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Kick { who: String, reason: String },
    Say(String),
    Stop,
    Help,
    Resource(crate::resources::ResourceCommand),
    Blocklists,
    Operation(ssx_net_ops::Command),
    Scripts,
    Script(String),
}

pub const HELP: &str = "commands: status | kick <rider number | player id prefix | name> [reason] | say <text> | stop | \
                        start <resource> | stop <resource> | restart <resource> | ensure <resource> | refresh | \
                        blocklists | scripts | <a server script's command> | help";

pub fn parse(line: &str) -> Option<Result<Command, String>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (word, rest) = line.split_once(char::is_whitespace).map_or((line, ""), |(w, r)| (w, r.trim()));
    if let Some(r) = crate::resources::ResourceCommand::parse(word, rest) {
        return Some(r.map(Command::Resource));
    }
    Some(match word {
        "status" if rest.is_empty() => Ok(Command::Status),
        "blocklists" if rest.is_empty() => Ok(Command::Blocklists),
        "stop" if rest.is_empty() => Ok(Command::Stop),
        "help" => Ok(Command::Help),
        "say" if !rest.is_empty() => Ok(Command::Say(rest.to_string())),
        "kick" if !rest.is_empty() => {
            ssx_net_ops::command::kick(rest).map(|(who, reason)| Command::Kick { who, reason })
        }
        _ if let Some(parsed) = ssx_net_ops::command::parse(word, rest) => parsed.map(Command::Operation),
        "scripts" if rest.is_empty() => Ok(Command::Scripts),
        "status" | "stop" | "say" | "kick" | "scripts" | "refresh" => Err(format!("unknown command {line:?}; {HELP}")),
        _ if ssx_script::api::check_server_command_name(word).is_ok() => Ok(Command::Script(line.to_string())),
        _ => Err(format!("unknown command {line:?}; {HELP}")),
    })
}

fn skip_line(input: &mut impl BufRead) {
    loop {
        let Ok(buf) = input.fill_buf() else { return };
        if buf.is_empty() {
            return;
        }
        if let Some(i) = buf.iter().position(|&b| b == b'\n') {
            input.consume(i + 1);
            return;
        }
        let n = buf.len();
        input.consume(n);
    }
}

static STOP_AT_END: AtomicBool = AtomicBool::new(false);

pub fn stop_at_end_of_input() {
    STOP_AT_END.store(true, Ordering::Relaxed);
}

pub fn spawn() -> std::io::Result<mpsc::Receiver<String>> {
    spawn_from(std::io::stdin(), STOP_AT_END.load(Ordering::Relaxed))
}

pub fn spawn_from(input: impl Read + Send + 'static, stop_at_end: bool) -> std::io::Result<mpsc::Receiver<String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new().name("console".into()).spawn(move || {
        let mut lock = BufReader::new(input);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match lock.by_ref().take(MAX_LINE as u64).read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => {
                    if stop_at_end {
                        let _ = tx.send("stop".into());
                    }
                    break;
                }
                Ok(_) => {
                    if buf.last() != Some(&b'\n') && buf.len() == MAX_LINE {
                        skip_line(&mut lock);
                    }
                    if tx.send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    })?;
    Ok(rx)
}

pub fn attach(app: &mut bevy::app::App, input: impl Read + Send + 'static, stop_at_end: bool) -> std::io::Result<()> {
    let lines = spawn_from(input, stop_at_end)?;
    app.insert_resource(crate::systems::ConsoleLines(std::sync::Mutex::new(lines)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(parse("  status "), Some(Ok(Command::Status)));
        assert_eq!(parse("say Race in 5 minutes"), Some(Ok(Command::Say("Race in 5 minutes".into()))));
        assert_eq!(
            parse("kick 3 flooding the chat"),
            Some(Ok(Command::Kick { who: "3".into(), reason: "flooding the chat".into() }))
        );
        assert_eq!(parse("kick a1b2c3d4"), Some(Ok(Command::Kick { who: "a1b2c3d4".into(), reason: String::new() })));
        assert_eq!(parse(""), None);
        for bad in ["say", "kick", "stop now x", "status x"] {
            assert!(matches!(parse(bad), Some(Err(_))), "{bad}");
        }
        assert_eq!(parse("rm -rf /"), Some(Ok(Command::Script("rm -rf /".into()))));
        assert_eq!(parse(" scoreboard  top 3 "), Some(Ok(Command::Script("scoreboard  top 3".into()))));
        assert_eq!(parse("scripts"), Some(Ok(Command::Scripts)));
        assert!(matches!(parse("scripts now"), Some(Err(_))), "the monitor's word is the server's");
        for bad in ["no-such-command", "Top", &"x".repeat(33), "a.b"] {
            assert!(matches!(parse(bad), Some(Err(_))), "{bad}");
        }
        let names: Vec<String> = (0..=255u8)
            .map(|b| char::from(b).to_string())
            .chain(["x".repeat(32), "top_3".into(), "a-b".into()])
            .collect();
        for name in &names {
            if ssx_script::api::check_server_command_name(name).is_ok() {
                assert!(ssx_net_ops::acl::right(&format!("command.{name}")).is_ok(), "{name:?}");
            }
        }
        use crate::resources::ResourceCommand as R;
        assert_eq!(parse("stop"), Some(Ok(Command::Stop)), "the server's stop keeps its word");
        assert_eq!(parse("stop skins"), Some(Ok(Command::Resource(R::Stop("skins".into())))));
        assert_eq!(parse("start maps"), Some(Ok(Command::Resource(R::Start("maps".into())))));
        assert_eq!(parse("restart maps"), Some(Ok(Command::Resource(R::Restart("maps".into())))));
        assert_eq!(parse("ensure maps"), Some(Ok(Command::Resource(R::Ensure("maps".into())))));
        assert_eq!(parse("refresh"), Some(Ok(Command::Resource(R::Refresh))));
        for bad in ["start", "start ../x", "start Maps", "restart a b", "refresh now", "ensure ssx\u{1b}"] {
            assert!(matches!(parse(bad), Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn the_operation_commands_are_the_consoles() {
        assert_eq!(parse("instances"), Some(Ok(Command::Operation(ssx_net_ops::Command::Instances))));
        assert!(matches!(parse("ban 3 1d griefing"), Some(Ok(Command::Operation(_)))));
        assert!(matches!(parse("acl allow Admin command.kick"), Some(Ok(Command::Operation(_)))));
        assert_eq!(
            parse("kick \"Kick Proof\" read the rules"),
            Some(Ok(Command::Kick { who: "Kick Proof".into(), reason: "read the rules".into() }))
        );
        for bad in
            ["ban 3", "instances x", "acl", "aclrequest allow ../x all", "unban 0", "kick \"Kick", "kick 3 a\u{7}b"]
        {
            assert!(matches!(parse(bad), Some(Err(_))), "{bad}");
        }
    }
}
