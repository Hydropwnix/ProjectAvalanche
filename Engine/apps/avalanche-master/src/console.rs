use std::io::{BufRead, Read};

const MAX_LINE: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Status,
    Servers,
    Reload,
    Stop,
    Help,
}

pub const HELP: &str = "commands: status | servers | reload (the blocklists and the certificate) | stop | help";

pub fn parse(line: &str) -> Option<Result<Command, String>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(match line {
        "status" => Ok(Command::Status),
        "servers" => Ok(Command::Servers),
        "reload" => Ok(Command::Reload),
        "stop" => Ok(Command::Stop),
        "help" => Ok(Command::Help),
        _ => Err(format!("unknown command {:?}; {HELP}", line.chars().take(64).collect::<String>())),
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

pub fn spawn() -> std::io::Result<tokio::sync::mpsc::Receiver<String>> {
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    std::thread::Builder::new().name("console".into()).spawn(move || {
        let stdin = std::io::stdin();
        let mut lock = stdin.lock();
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match lock.by_ref().take(MAX_LINE as u64).read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if buf.last() != Some(&b'\n') && buf.len() == MAX_LINE {
                        skip_line(&mut lock);
                    }
                    if tx.blocking_send(String::from_utf8_lossy(&buf).into_owned()).is_err() {
                        break;
                    }
                }
            }
        }
    })?;
    Ok(rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(parse(" status "), Some(Ok(Command::Status)));
        assert_eq!(parse("servers"), Some(Ok(Command::Servers)));
        assert_eq!(parse("reload"), Some(Ok(Command::Reload)));
        assert_eq!(parse(""), None);
        for bad in ["stop now", "kick 1", "rm -rf /", &"x".repeat(5000)] {
            assert!(matches!(parse(bad), Some(Err(e)) if e.len() < 200), "{}", &bad[..bad.len().min(20)]);
        }
    }
}
