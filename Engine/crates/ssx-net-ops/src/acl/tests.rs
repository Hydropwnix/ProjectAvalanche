use ssx_net_proto::rcon::OperatorSecret;

use super::*;

const NOW: u64 = 1_793_987_125;

fn acl_with(admin: &OperatorKey, moderator: &OperatorKey) -> Acl {
    let mut acl = Acl::default();
    acl.create_group("Admin").unwrap();
    acl.add_object("Admin", Object::Operator(*admin)).unwrap();
    acl.set_right("Admin", "command", Access::Allow).unwrap();
    acl.set_right("Admin", "command.stop", Access::Deny).unwrap();
    acl.create_group("Moderator").unwrap();
    acl.add_object("Moderator", Object::Operator(*moderator)).unwrap();
    for r in ["command.kick", "command.mute", "command.status"] {
        acl.set_right("Moderator", r, Access::Allow).unwrap();
    }
    acl
}

#[test]
fn rights_walk_from_the_most_specific_level_and_deny_beats_allow() {
    let (a, m, x) =
        (OperatorSecret::generate().public(), OperatorSecret::generate().public(), OperatorSecret::generate().public());
    let mut acl = acl_with(&a, &m);
    assert!(acl.has_operators() && acl.is_operator(&a) && acl.is_operator(&m) && !acl.is_operator(&x));
    assert!(acl.allows(Subject::Operator(&a), "command.ban"), "the root's allow");
    assert!(!acl.allows(Subject::Operator(&a), "command.stop"), "a more specific deny");
    assert_eq!(acl.check(Subject::Operator(&a), "command.stop").source, Source::Group("Admin".into()));
    assert!(acl.allows(Subject::Operator(&m), "command.kick") && !acl.allows(Subject::Operator(&m), "command.ban"));
    assert!(!acl.allows(Subject::Operator(&x), "command.status"), "an unknown key holds nothing");
    assert!(acl.allows(Subject::Console, "command.stop") && acl.allows(Subject::Console, "anything at all"));
    assert!(!acl.allows(Subject::Operator(&a), "command..x") && !acl.allows(Subject::Operator(&a), "commands"));
    acl.create_group("Probation").unwrap();
    acl.add_object("Probation", Object::Operator(m)).unwrap();
    acl.set_right("Probation", "command.kick", Access::Deny).unwrap();
    assert!(!acl.allows(Subject::Operator(&m), "command.kick"));
    acl.set_right("Probation", "command.kick", Access::Allow).unwrap();
    assert!(acl.allows(Subject::Operator(&m), "command.kick"));
    acl.revoke("Probation", "command.kick").unwrap();
    assert!(acl.revoke("Probation", "command.kick").is_err());
    assert!(acl.delete_group("probation").is_ok(), "names without regard to case");
    assert!(acl.create_group("ADMIN").is_err());
}

#[test]
fn players_and_addresses_are_objects() {
    let mut acl = Acl::default();
    acl.create_group("Everyone").unwrap();
    acl.add_object("Everyone", Object::Players).unwrap();
    acl.set_right("Everyone", "command.vote", Access::Allow).unwrap();
    acl.create_group("Lan").unwrap();
    acl.add_object("Lan", Object::parse("ip.192.168.*.*").unwrap()).unwrap();
    acl.set_right("Lan", "command.vote", Access::Deny).unwrap();
    let id = [5u8; 32];
    let p = |ip: &str| Subject::Player { id: &id, ip: Some(ip.parse().unwrap()) };
    assert!(acl.allows(p("203.0.113.1"), "command.vote"));
    assert!(!acl.allows(p("192.168.4.4"), "command.vote"));
    assert!(!acl.allows(p("::ffff:192.168.4.4"), "command.vote"), "a mapped address is its IPv4 address");
    assert!(!acl.allows(Subject::Resource("race"), "command.vote"), "a resource is not a player");
}

#[test]
fn resources_get_mtas_default_denials_and_their_requests_wait() {
    let mut acl = Acl::default();
    let r = Subject::Resource("race_tools");
    assert!(acl.allows(r, "function.spawnRider"), "the API's functions");
    for denied in RESOURCE_DENIED {
        assert!(!acl.allows(r, denied), "{denied}");
    }
    assert!(!acl.allows(r, "function.acl.setRight"), "below a denied right");
    assert!(!acl.allows(r, "command.stop") && !acl.allows(r, "resource.start"), "nothing speaks: denied");
    assert_eq!(acl.request("race_tools", "function.fetchRemote", NOW), Ok(RequestState::Pending));
    assert_eq!(acl.request("race_tools", "general.ModifyOtherObjects", NOW), Ok(RequestState::Pending));
    assert!(!acl.allows(r, "function.fetchRemote"));
    assert_eq!(
        acl.decide("race_tools", Some("function.fetchRemote"), true, "operator 0123456789abcdef", NOW + 5),
        Ok(1)
    );
    assert!(acl.allows(r, "function.fetchRemote"), "an explicit allow beats the default deny of its level");
    assert_eq!(acl.check(r, "function.fetchRemote").source, Source::Request);
    assert!(!acl.allows(Subject::Resource("other"), "function.fetchRemote"), "for that resource only");
    assert_eq!(acl.request("race_tools", "function.fetchRemote", NOW + 9), Ok(RequestState::Allowed));
    assert_eq!(acl.decide("race_tools", None, false, "console", NOW + 10), Ok(1));
    let q = acl.requests().iter().find(|q| q.right == "general.ModifyOtherObjects").unwrap();
    assert_eq!((q.state, q.by.as_str(), q.at), (RequestState::Denied, "console", NOW + 10));
    assert!(acl.decide("race_tools", None, false, "console", NOW).is_err(), "nothing pending");
    acl.create_group("NoNet").unwrap();
    acl.add_object("NoNet", Object::Resources).unwrap();
    acl.set_right("NoNet", "function.fetchRemote", Access::Deny).unwrap();
    assert!(!acl.allows(r, "function.fetchRemote"));
    assert!(acl.request("../x", "function.a", NOW).is_err() && acl.request("x", "nope", NOW).is_err());
}

#[test]
fn the_file_reads_back_exactly() {
    let (a, m) = (OperatorSecret::generate().public(), OperatorSecret::generate().public());
    let mut acl = acl_with(&a, &m);
    acl.add_object("Moderator", Object::parse("ip.2001:db8::/32").unwrap()).unwrap();
    acl.add_object("Moderator", Object::parse(&format!("player.{}", "0f".repeat(32))).unwrap()).unwrap();
    acl.create_group("Empty").unwrap();
    acl.request("race", "function.callRemote", NOW).unwrap();
    acl.request("race", "function.load", NOW).unwrap();
    acl.decide("race", Some("function.load"), false, "console", NOW + 1).unwrap();
    let text = acl.to_toml();
    assert_eq!(Acl::parse(&text), Ok(acl.clone()), "{text}");
    assert_eq!(Acl::parse(""), Ok(Acl::default()));
}

#[test]
fn hostile_files_and_objects_are_refused() {
    let key = OperatorSecret::generate().public().to_hex();
    let good =
        format!("[[group]]\nname = \"Admin\"\nobjects = [\"operator.{key}\"]\nallow = [\"command\"]\ndeny = []\n");
    assert!(Acl::parse(&good).is_ok());
    for (bad, line) in [
        (good.replace("Admin", "Ad min"), 2),
        (good.replace(&key, &key.to_uppercase()), 3),
        (good.replace(&key, "*"), 3),
        (good.replace(&format!("operator.{key}"), "player.ab"), 3),
        (good.replace(&format!("operator.{key}"), "ip.192.0.2.1/24"), 3),
        (good.replace(&format!("operator.{key}"), "resource.../x"), 3),
        (good.replace(&format!("operator.{key}"), "group.x"), 3),
        (good.replace("[\"command\"]", "[\"command\", \"command\"]"), 4),
        (good.replace("deny = []", "deny = [\"command\"]"), 5),
        (good.replace("[\"command\"]", "[\"commands.kick\"]"), 4),
        (good.replace("[\"command\"]", "[\"command.\"]"), 4),
        (good.replace("deny = []\n", ""), 1),
        (format!("{good}{}", good.replace("Admin", "admin")), 7),
        (format!("{good}x = 1\n"), 6),
        (
            format!(
                "{good}[[request]]\nresource = \"r\"\nright = \"function.a\"\nstate = \"maybe\"\nasked = 1\nby = \"\"\nat = 0\n"
            ),
            9,
        ),
        (
            format!(
                "{good}[[request]]\nresource = \"r\"\nright = \"function.a\"\nstate = \"pending\"\nasked = 1\nby = \"console\"\nat = 0\n"
            ),
            11,
        ),
        (
            format!(
                "{good}[[request]]\nresource = \"r\"\nright = \"function.a\"\nstate = \"allowed\"\nasked = 1\nby = \"console\"\nat = 0\n"
            ),
            12,
        ),
    ] {
        let e = Acl::parse(&bad).expect_err(&bad);
        assert_eq!(e.line, line, "{e} in\n{bad}");
    }
    for bad in ["operator.*", "player", "ip.", "resource.", "resource.A", &format!("operator.{}", "00".repeat(32))] {
        assert!(Object::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn check_all_gives_the_verdicts_check_gives() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let resources = ["board", "warden", "x.y", "spare"];
    let pool = [
        "function",
        "function.getPlayers",
        "function.playSoundFor",
        "function.json",
        "function.json.encode",
        "function.kickPlayer",
        "function.startResource",
        "function.load",
        "general",
        "general.ModifyOtherObjects",
        "command",
        "command.kick",
        "resource.board",
    ];
    let mut rights: Vec<String> = pool.iter().map(|r| r.to_string()).collect();
    rights.extend(["function..x", "nothing", "function.a.b.c.d.e.f.g.h.i"].map(String::from));
    let id = [7u8; 32];
    for _ in 0..2000 {
        let mut acl = Acl::default();
        for g in 0..next(6) {
            let name = format!("G{g}");
            acl.create_group(&name).unwrap();
            for _ in 0..next(4) {
                let object = match next(6) {
                    0 => Object::Resources,
                    1 => Object::Players,
                    k => Object::Resource(resources[k % resources.len()].to_string()),
                };
                let _ = acl.add_object(&name, object);
            }
            for _ in 0..next(7) {
                let access = if next(2) == 0 { Access::Allow } else { Access::Deny };
                let _ = acl.set_right(&name, pool[next(pool.len())], access);
            }
        }
        for _ in 0..next(8) {
            let (resource, right) = (resources[next(resources.len())], pool[next(pool.len())]);
            acl.request(resource, right, NOW).unwrap();
            match next(3) {
                0 => {}
                k => {
                    let _ = acl.decide(resource, Some(right), k == 1, "console", NOW);
                }
            }
        }
        let mut subjects: Vec<Subject<'_>> = resources.iter().map(|r| Subject::Resource(r)).collect();
        subjects.push(Subject::Player { id: &id, ip: None });
        subjects.push(Subject::Console);
        for subject in subjects {
            let each: Vec<Verdict> = rights.iter().map(|r| acl.check(subject, r)).collect();
            assert_eq!(acl.check_all(subject, &rights), each, "{acl:?} {subject:?}");
        }
    }
}
