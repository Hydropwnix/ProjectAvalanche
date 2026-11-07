use std::net::IpAddr;
use std::sync::Arc;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use ssx_net_ops::acl::{Access, Object, RequestState};
use ssx_script::host::server::rights::{self, Holder, PRIVILEGED, Rights};
use ssx_script::host::{ClientId, CommandOrigin, HostConfig, ScriptHost, Side};

use super::{Identities, OperationRights, OrderStats, record_requests};
use crate::operation::Operation;
use crate::operation::commands::OperationRes;
use crate::operation::store::unix_now;
use crate::resources::PermissionRequests;

struct Folder(std::path::PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn folder(tag: &str) -> Folder {
    let dir = std::env::temp_dir().join(format!("ssx-rights-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Folder(dir)
}

fn change(op: &Operation, f: impl FnOnce(&mut ssx_net_ops::Acl) -> Result<(), ssx_net_ops::acl::AclError>) {
    op.change_acl(|a| f(a).map_err(|e| e.to_string())).unwrap();
}

#[test]
fn the_acl_decides_and_a_pending_request_grants_nothing() {
    let dir = folder("acl");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let rights = OperationRights { operation: Arc::clone(&op), players: Identities::default() };
    let check = |right: &str| rights.check(Holder::Package("admin"), right).unwrap();
    for name in PRIVILEGED {
        let d = check(&format!("function.{name}"));
        assert!(!d.allowed, "{name}");
        assert_eq!(d.why, format!("the built-in defaults deny function.{name} to every resource"));
    }
    assert!(check("function.createMarker").allowed, "the rest of the API stays open");
    let kick = "function.kickPlayer";
    let now = unix_now();
    change(&op, |a| a.request("admin", kick, now).map(|s| assert_eq!(s, RequestState::Pending)));
    let d = check(kick);
    assert!(!d.allowed, "pending is not granted");
    assert!(d.why.ends_with("its request is pending until an operator's `aclrequest allow admin function.kickPlayer`"));
    change(&op, |a| a.decide("admin", Some(kick), true, "console", now).map(|n| assert_eq!(n, 1)));
    assert_eq!(check(kick).why, "the operator allowed its request for function.kickPlayer");
    assert!(check(kick).allowed);
    change(&op, |a| {
        a.create_group("Probation")?;
        a.add_object("Probation", Object::Resource("admin".into()))?;
        a.set_right("Probation", kick, Access::Deny)
    });
    let d = check(kick);
    assert_eq!((d.allowed, d.why.as_str()), (false, "the group Probation denies function.kickPlayer"));
    assert!(rights.check(Holder::Package("admin"), "function..x").is_err());
    assert!(rights.check(Holder::Package("Admin"), kick).is_err());
}

#[test]
fn players_are_checked_by_id_and_address_and_reasons_by_the_operators_rule() {
    let dir = folder("players");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let ids = Identities::default();
    let rights = OperationRights { operation: Arc::clone(&op), players: ids.clone() };
    let (id, ip): ([u8; 32], IpAddr) = ([7; 32], "203.0.113.9".parse().unwrap());
    ids.insert(4, id, "::ffff:203.0.113.9".parse().unwrap());
    change(&op, |a| {
        a.create_group("Moderators")?;
        a.add_object("Moderators", Object::Player(id))?;
        a.set_right("Moderators", "command.kick", Access::Allow)?;
        a.create_group("Lan")?;
        a.add_object("Lan", Object::Ip(ssx_net_ops::Range::address(ip)))?;
        a.set_right("Lan", "command.say", Access::Allow)
    });
    let player = |c: u32, right: &str| rights.check(Holder::Player(ClientId(c)), right);
    assert!(player(4, "command.kick").unwrap().allowed);
    assert!(player(4, "command.say").unwrap().allowed, "the address is the canonical (mapped) one");
    assert!(!player(4, "command.ban").unwrap().allowed);
    assert_eq!(player(5, "command.kick").unwrap_err(), "the player is not connected");
    ids.forget(4);
    assert!(player(4, "command.kick").is_err());
    assert!(rights.reason("read the rules").is_ok() && rights.reason("").is_ok());
    assert!(rights.reason("a\u{7}b").is_err() && rights.reason(&"x".repeat(201)).is_err());
}

#[test]
fn permission_requests_become_pending_requests_once() {
    let dir = folder("requests");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let mut app = App::new();
    app.insert_resource(OperationRes(Arc::clone(&op))).init_resource::<OrderStats>();
    let mut requests = PermissionRequests::default();
    let rights = ["function.kickPlayer", "function.fetchRemote", "custom.thing"].map(String::from).to_vec();
    requests.by_resource.insert("admin".into(), (ssx_content::Version::new(1, 0, 0), rights));
    requests.generation = 1;
    app.insert_resource(requests);
    app.world_mut().run_system_once(record_requests).unwrap();
    let pending = |op: &Operation| op.acl().requests().iter().filter(|q| q.state == RequestState::Pending).count();
    assert_eq!(pending(&op), 2);
    let written = std::fs::metadata(op.acl_path()).unwrap().modified().unwrap();
    change(&op, |a| a.decide("admin", Some("function.kickPlayer"), true, "console", unix_now()).map(|_| ()));
    let decided = std::fs::metadata(op.acl_path()).unwrap().modified().unwrap();
    assert!(decided >= written);
    app.world_mut().resource_mut::<PermissionRequests>().generation = 2;
    app.world_mut().run_system_once(record_requests).unwrap();
    assert_eq!(pending(&op), 1, "the allowed request keeps its state");
    assert_eq!(std::fs::metadata(op.acl_path()).unwrap().modified().unwrap(), decided, "nothing new: not written");
    assert_eq!(app.world().resource::<OrderStats>().requests_recorded, 2);
}

#[test]
fn a_resource_holds_every_call_but_the_privileged_and_resolve_is_check() {
    let dir = folder("calls");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let rights = OperationRights { operation: Arc::clone(&op), players: Identities::default() };
    let calls = rights::call_rights();
    let decisions = rights.resolve("board", calls).unwrap();
    let denied: Vec<&str> = calls.iter().zip(&decisions).filter(|(_, d)| !d.allowed).map(|(r, _)| r.as_str()).collect();
    assert_eq!(denied, PRIVILEGED.map(|p| format!("function.{p}")), "of {} calls", calls.len());
    let open = decisions.iter().find(|d| d.allowed).unwrap();
    assert_eq!(open.why, "the built-in defaults allow function to every resource");
    let now = unix_now();
    change(&op, |a| {
        a.create_group("Quiet")?;
        a.add_object("Quiet", Object::Resources)?;
        a.set_right("Quiet", "function.playSoundFor", Access::Deny)?;
        a.create_group("Admin")?;
        a.add_object("Admin", Object::Resource("board".into()))?;
        a.set_right("Admin", "function.kickPlayer", Access::Allow)?;
        a.set_right("Admin", "function.getPlayers", Access::Deny)?;
        a.request("board", "function.banPlayer", now)?;
        a.request("board", "function.mutePlayer", now)?;
        a.request("board", "function.getRootElement", now)?;
        a.request("other", "function.executeCommand", now)?;
        a.decide("board", Some("function.mutePlayer"), true, "console", now)?;
        a.decide("board", Some("function.getRootElement"), false, "console", now).map(|_| ())
    });
    for package in ["board", "other", "spare"] {
        let all = rights.resolve(package, calls).unwrap();
        for (right, d) in calls.iter().zip(all) {
            assert_eq!(d, rights.check(Holder::Package(package), right).unwrap(), "{package} {right}");
        }
    }
    let one = |package: &str, right: &str| {
        let k = calls.iter().position(|r| r == right).unwrap();
        rights.resolve(package, calls).unwrap().swap_remove(k)
    };
    assert_eq!(one("board", "function.playSoundFor").why, "the group Quiet denies function.playSoundFor");
    assert!(one("board", "function.kickPlayer").allowed && one("board", "function.mutePlayer").allowed);
    assert!(one("board", "function.banPlayer").why.ends_with("`aclrequest allow board function.banPlayer`"));
    assert_eq!(
        one("board", "function.getRootElement").why,
        "the operator denied its request for function.getRootElement"
    );
    assert!(one("other", "function.executeCommand").why.ends_with("`aclrequest allow other function.executeCommand`"));
    assert!(rights.resolve("Board", calls).is_err(), "a name outside the rule");
    assert!(rights.resolve("board", &["function..x".to_string()]).is_err(), "a right outside the rule");
}

#[test]
fn a_more_specific_allow_overrides_a_group_deny_and_deny_beats_allow_on_a_level() {
    let dir = folder("levels");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let rights = OperationRights { operation: Arc::clone(&op), players: Identities::default() };
    let calls = rights::call_rights();
    let held = |package: &str, right: &str| {
        let k = calls.iter().position(|r| r == right).unwrap();
        rights.resolve(package, calls).unwrap().swap_remove(k)
    };
    change(&op, |a| {
        a.create_group("Locked")?;
        a.add_object("Locked", Object::Resources)?;
        a.set_right("Locked", "function", Access::Deny)?;
        a.create_group("Board")?;
        a.add_object("Board", Object::Resource("board".into()))?;
        a.set_right("Board", "function.getPlayers", Access::Allow)
    });
    let d = held("board", "function.getPlayers");
    assert_eq!((d.allowed, d.why.as_str()), (true, "the group Board allows function.getPlayers"));
    let d = held("board", "function.playSoundFor");
    assert_eq!((d.allowed, d.why.as_str()), (false, "the group Locked denies function"));
    assert!(!held("other", "function.getPlayers").allowed, "the allow is the board's only");
    change(&op, |a| {
        a.create_group("Quiet")?;
        a.add_object("Quiet", Object::Resource("board".into()))?;
        a.set_right("Quiet", "function.getPlayers", Access::Deny)
    });
    let d = held("board", "function.getPlayers");
    assert_eq!((d.allowed, d.why.as_str()), (false, "the group Quiet denies function.getPlayers"));
    change(&op, |a| {
        a.delete_group("Locked")?;
        a.set_right("Board", "function", Access::Allow)
    });
    let d = held("board", "function.kickPlayer");
    assert_eq!(
        (d.allowed, d.why.as_str()),
        (false, "the built-in defaults deny function.kickPlayer to every resource")
    );
}

fn host_with(op: &Arc<Operation>, name: &str, src: &str) -> ScriptHost {
    let host = ScriptHost::new(HostConfig::new(Side::Server)).unwrap();
    host.set_rights(Arc::new(OperationRights { operation: Arc::clone(op), players: Identities::default() }));
    host.start(crate::scripts::tests::package_with(name, src)).unwrap();
    host
}

fn run(host: &ScriptHost, word: &str, name: &str) -> Vec<String> {
    host.execute_command(CommandOrigin::Console, word).unwrap();
    let lines = host.take_output(name).map(|o| o.lines).unwrap_or_default();
    lines.into_iter().filter(|l| !l.contains(": refused: ")).collect()
}

#[test]
fn the_calls_follow_the_acl_mid_round() {
    let dir = folder("mid-round");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let src = "addCommandHandler('go', function()\n\
                 local ok, err = pcall(playSoundFor)\n\
                 print(ok or tostring(err):find('access denied', 1, true) and tostring(err) or 'other')\n\
                 print(hasObjectPermissionTo(resourceRoot, 'function.playSoundFor'), \
                       hasObjectPermissionTo(resourceRoot, 'function.kickPlayer'))\n\
               end)";
    let host = host_with(&op, "board", src);
    assert_eq!(run(&host, "go", "board"), ["other", "true\tfalse"], "the defaults");
    assert_eq!(host.rights_counts().resolved, 1);
    change(&op, |a| {
        a.create_group("Muted")?;
        a.add_object("Muted", Object::Resource("board".into()))?;
        a.set_right("Muted", "function.playSoundFor", Access::Deny)
    });
    let lines = run(&host, "go", "board");
    let refused = "runtime error: playSoundFor: access denied: board does not hold function.playSoundFor: the group \
                   Muted denies function.playSoundFor\n";
    assert!(lines[0].starts_with(refused), "{lines:?}");
    assert_eq!(lines[1], "false\tfalse", "hasObjectPermissionTo reads the set");
    assert_eq!((host.rights_counts().calls_refused, host.rights_counts().resolved), (1, 2));
    host.tick(std::time::Duration::from_millis(33));
    run(&host, "go", "board");
    assert_eq!(host.rights_counts().resolved, 2, "nothing moved: the set stands");
    ssx_net_ops::Acl::default().save(op.acl_path()).unwrap();
    assert_eq!(op.reload_acl().unwrap(), (0, 0));
    host.tick(std::time::Duration::from_millis(66));
    assert_eq!(host.rights_counts().resolved, 3);
    assert_eq!(run(&host, "go", "board"), ["other", "true\tfalse"]);
    let now = unix_now();
    change(&op, |a| a.request("board", "function.kickPlayer", now).map(|_| ()));
    assert_eq!(run(&host, "go", "board")[1], "true\tfalse", "pending grants nothing");
    change(&op, |a| a.decide("board", Some("function.kickPlayer"), true, "console", now).map(|_| ()));
    assert_eq!(run(&host, "go", "board")[1], "true\ttrue");
    assert_eq!((host.rights_counts().calls_refused, host.rights_counts().resolved), (2, 5));
}

#[test]
fn the_check_and_the_resolution_cost() {
    use std::time::Instant;
    let calls = rights::call_rights();
    let cases = [
        ("empty", 0, 1, 0, 0),
        ("typical", 8, 1, 25, 50),
        ("rights' limits", 256, 1, 1024, 4096),
        ("rights' limits and 512 objects a group", 256, 512, 1024, 4096),
    ];
    for (tag, groups, objects, each, requests) in cases {
        let dir = folder(&format!("cost-{groups}-{objects}"));
        let op = Arc::new(Operation::load(&dir.0).unwrap());
        let now = unix_now();
        change(&op, |a| {
            for g in 0..groups {
                let name = format!("G{g}");
                a.create_group(&name)?;
                if objects == 1 {
                    a.add_object(&name, Object::Resources)?;
                } else {
                    for k in 1..objects {
                        a.add_object(&name, Object::Resource(format!("other{k}")))?;
                    }
                    a.add_object(&name, Object::Resource("board".into()))?;
                }
                for k in 0..each {
                    let access = if k % 2 == 0 { Access::Allow } else { Access::Deny };
                    a.set_right(&name, &format!("function.custom{g}.right{k}"), access)?;
                }
            }
            for k in 0..requests {
                a.request("board", &format!("function.asked.right{k}"), now)?;
            }
            Ok(())
        });
        let rights = OperationRights { operation: Arc::clone(&op), players: Identities::default() };
        let n = if groups > 100 { 5 } else { 200 };
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(rights.resolve("board", calls).unwrap());
        }
        let all = t.elapsed() / n;
        let t = Instant::now();
        for _ in 0..n {
            let each: Vec<_> = calls.iter().map(|r| rights.check(Holder::Package("board"), r).unwrap()).collect();
            std::hint::black_box(each);
        }
        let walk = t.elapsed() / n;
        println!(
            "resolve on the {tag} ACL ({groups} groups x {objects} objects x {each} rights, {requests} requests), {} \
             calls: check_all {all:?}, a check of each {walk:?}",
            calls.len()
        );
    }
    let dir = folder("cost-call");
    let op = Arc::new(Operation::load(&dir.0).unwrap());
    let src = "addCommandHandler('bench', function()\n\
                 local t = os.clock()\n\
                 for i = 1, 100000 do getRootElement() end\n\
                 print(string.format('%.1f', (os.clock() - t) * 1e9 / 100000))\n\
               end)";
    let host = host_with(&op, "bench", src);
    let ns = run(&host, "bench", "bench");
    println!(
        "a getRootElement() call end to end: {} ns (the check's lookup in ssx-script's host::server::rights tests); \
         the server's tick budget {:?}",
        ns.first().map_or("?", String::as_str),
        ssx_script::host::limits::Budgets::SERVER.tick
    );
}
