use super::*;

fn line(text: &str) -> Option<Result<Command, String>> {
    let (word, rest) = text.split_once(' ').unwrap_or((text, ""));
    parse(word, rest)
}

#[test]
fn the_commands_read() {
    let id = "ab".repeat(32);
    assert_eq!(line("instances"), Some(Ok(Command::Instances)));
    assert_eq!(
        line("ban 3 7d griefing the start"),
        Some(Ok(Command::Ban {
            who: Who::Rider("3".into()),
            seconds: Some(604_800),
            reason: "griefing the start".into()
        }))
    );
    assert_eq!(
        line("ban \"Zoë Payne\" forever"),
        Some(Ok(Command::Ban { who: Who::Rider("Zoë Payne".into()), seconds: None, reason: String::new() }))
    );
    let Some(Ok(Command::Ban { who: Who::Target(t), .. })) = line("ban range:192.0.2.* 1h") else { panic!("a range") };
    assert_eq!(t.to_string(), "range:192.0.2.*");
    let Some(Ok(Command::Ban { who: Who::Target(t), .. })) = line(&format!("ban player:{} 1h", id.to_uppercase()))
    else {
        panic!("a player id in capitals")
    };
    assert_eq!(t.to_string(), format!("player:{id}"));
    assert_eq!(line("unban 2"), Some(Ok(Command::Unban(Unban::Number(2)))));
    assert!(matches!(line("unban ip:::ffff:192.0.2.7"), Some(Ok(Command::Unban(Unban::Target(_))))));
    assert_eq!(line("bans"), Some(Ok(Command::Bans { reload: false })));
    assert_eq!(line("bans reload"), Some(Ok(Command::Bans { reload: true })));
    assert!(matches!(line("mute 4 30m spam"), Some(Ok(Command::Mute { seconds: Some(1800), .. }))));
    assert!(matches!(line(&format!("unmute player:{id}")), Some(Ok(Command::Unmute(Who::Target(_))))));
    assert_eq!(line("mutes"), Some(Ok(Command::Mutes)));
    assert_eq!(line("acl list"), Some(Ok(Command::Acl(AclCommand::List))));
    assert_eq!(
        line("acl allow Moderator command.kick"),
        Some(Ok(Command::Acl(AclCommand::Allow { group: "Moderator".into(), right: "command.kick".into() })))
    );
    assert!(matches!(line("acl check player.* command.vote"), Some(Ok(Command::Acl(AclCommand::Check { .. })))));
    assert_eq!(
        line("aclrequest allow race_tools all"),
        Some(Ok(Command::AclRequest(RequestCommand::Allow { resource: "race_tools".into(), right: None })))
    );
    assert_eq!(line("aclrequest list"), Some(Ok(Command::AclRequest(RequestCommand::List(None)))));
    assert_eq!(line("status"), None, "not this module's word");
}

#[test]
fn bad_lines_are_refused_with_their_reason() {
    for bad in [
        "instances now",
        "ban",
        "ban 3",
        "ban 3 7",
        "ban 3 0d",
        "ban 3 7d \u{7}",
        "ban \"Zoë 1d",
        "ban \"\" 1d",
        "ban \"a\"b 1d",
        "ban ip:192.0.2.0/24 1d",
        "ban range:192.0.2.7 1d",
        "ban player:ab 1d",
        "ban key:00 1d",
        "mute ip:192.0.2.7 1d",
        "mute range:10.*.*.* 1d",
        "unmute ip:192.0.2.7",
        "unban",
        "unban 0",
        "unban 02",
        "unban 1 2",
        "unban 9999999",
        "bans now",
        "mutes x",
        "acl",
        "acl list x",
        "acl show",
        "acl show a b",
        "acl add Admin operator.*",
        "acl allow Admin commands",
        "acl allow \"Ad min\" command",
        "acl frobnicate",
        "aclrequest",
        "aclrequest allow ../x all",
        "aclrequest allow r",
        "aclrequest deny r function.",
        "aclrequest list a b",
    ] {
        assert!(matches!(line(bad), Some(Err(_))), "{bad:?}: {:?}", line(bad));
    }
}
