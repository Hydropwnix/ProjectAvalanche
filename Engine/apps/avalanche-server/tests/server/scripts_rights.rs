use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ssx_net_join::{Granted, JoinFailure, JoinParams};
use ssx_net_proto::join::{JoinContent, RefusalCode};

use crate::common::{LOCAL, Player, connected, get, step_until, temp_dir, udp_client};
use crate::operation::{ops_with, ran};

const WARDEN: &str = r#"
addEventHandler("onPlayerConnecting", root, function(name, keyId, join)
    if name == "Starter" then
        local ok, why = startResource("spare")
        refuseJoin(join, tostring(ok) .. " " .. tostring(why))
    end
end)
addEventHandler("onPlayerJoin", root, function()
    local name = getPlayerName(source)
    if name and string.sub(name, 1, 4) == "Kick" then
        local ok, why = kickPlayer(source, "the warden says no")
        print("kickPlayer", ok, why)
    end
end)
"#;

fn resources(root: &Path) {
    let warden = root.join("warden");
    std::fs::create_dir_all(warden.join("server")).unwrap();
    let manifest = "[resource]\nname = \"warden\"\nversion = \"1.0.0\"\napi = 1\n\n[scripts]\nserver = [\"server/*.luau\"]\n\n\
                    [permissions]\nrequest = [\"function.startResource\", \"function.kickPlayer\"]\n";
    std::fs::write(warden.join("resource.toml"), manifest).unwrap();
    std::fs::write(warden.join("server/main.luau"), WARDEN).unwrap();
    let spare = root.join("spare");
    std::fs::create_dir_all(spare.join("data")).unwrap();
    std::fs::write(spare.join("resource.toml"), "[resource]\nname = \"spare\"\nversion = \"1.0.0\"\napi = 1\n")
        .unwrap();
    std::fs::write(spare.join("data/one.bin"), [7u8; 64]).unwrap();
}

fn running(addr: std::net::SocketAddr) -> Vec<String> {
    match ssx_net_join::resources::fetch_manifest(addr, &AtomicBool::new(false)) {
        Ok(Some(f)) => f.manifest.resources.iter().map(|r| r.name.clone()).collect(),
        Ok(None) | Err(_) => Vec::new(),
    }
}

fn join(player: &mut Player, server: SocketAddr, name: &str) -> Result<Granted, JoinFailure> {
    let (status, body) = get(server, LOCAL, "/resources");
    assert_eq!(status, 200, "the manifest");
    let content = JoinContent { content_id: None, manifest: Some(ssx_content::ContentHash::of(&body).0) };
    let params = JoinParams { name, password: b"", cancel: None, content };
    ssx_net_join::join(server, &player.identity, &mut player.known, &params)
}

fn refusal_text(r: Result<Granted, JoinFailure>) -> String {
    match r {
        Err(JoinFailure::Refused(r)) if r.code == RefusalCode::Scripts => r.text,
        other => panic!("refused by the scripts, not {:?}", other.map(|g| g.info.name)),
    }
}

#[test]
fn a_privileged_call_is_denied_until_the_operator_allows_its_request() {
    let folder = temp_dir("scripts-rights");
    resources(&folder);
    let o = ops_with(
        |c| {
            c.resources.folder = folder.clone();
            c.resources.start = vec!["warden".into()];
        },
        "",
    );
    let mut player = Player::new();
    let up = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(500));
        ran(&o, "scripts").iter().any(|l| l.contains("warden"))
    });
    assert!(up, "the warden runs");
    let requests = ran(&o, "aclrequest list");
    for right in ["function.startResource", "function.kickPlayer"] {
        assert!(requests.iter().any(|l| l.contains("warden") && l.contains(right)), "{right}: {requests:?}");
    }
    std::thread::sleep(Duration::from_millis(500));
    let denied = refusal_text(join(&mut player, o.addr, "Starter"));
    assert!(
        denied.starts_with(
            "false warden does not hold function.startResource: the built-in defaults deny function.startResource to \
             every resource; its request is pending until an operator's `aclrequest allow warden"
        ),
        "{denied}"
    );
    assert!(!running(o.addr).contains(&"spare".to_string()));
    let allowed = ran(&o, "aclrequest allow warden function.startResource");
    assert!(!allowed.is_empty(), "{allowed:?}");
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(refusal_text(join(&mut player, o.addr, "Starter")), "true nil");
    let started = (0..20).any(|_| {
        std::thread::sleep(Duration::from_millis(500));
        running(o.addr).contains(&"spare".to_string())
    });
    assert!(started, "the allowed order started the resource");
    std::thread::sleep(Duration::from_millis(1100));
    let first = join(&mut Player::new(), o.addr, "Kickstay").expect("joined");
    let (mut stays, e1) = udp_client(&first.token, o.addr, LOCAL, None);
    assert!(step_until(&mut [&mut stays], Duration::from_secs(10), |a| connected(a[0], e1)));
    ran(&o, "aclrequest allow warden function.kickPlayer");
    std::thread::sleep(Duration::from_millis(1100));
    let second = join(&mut Player::new(), o.addr, "Kickgo").expect("joined");
    let (mut goes, e2) = udp_client(&second.token, o.addr, LOCAL, None);
    let told = step_until(&mut [&mut goes, &mut stays], Duration::from_secs(10), |a| {
        a[0].world()
            .resource::<ssx_net::ServerSaid>()
            .goodbye
            .as_ref()
            .is_some_and(|(_, t)| t.contains("the warden says no"))
    });
    assert!(told, "the kicked rider is told the script's reason");
    let _ = e2;
    assert!(connected(&stays, e1), "the rider joined while the right was pending stays");
    o.server.stop();
}

const PROBE: &str = r#"
addEventHandler("onPlayerConnecting", root, function(name, keyId, join)
    local ok, err = pcall(getPlayers)
    refuseJoin(join, ok and "held" or string.match(tostring(err), "access denied: [^\n]*") or "other")
end)
"#;

#[test]
fn a_call_the_operator_denies_is_refused_until_the_deny_is_revoked() {
    let folder = temp_dir("scripts-calls");
    let probe = folder.join("probe");
    std::fs::create_dir_all(probe.join("server")).unwrap();
    let manifest =
        "[resource]\nname = \"probe\"\nversion = \"1.0.0\"\napi = 1\n\n[scripts]\nserver = [\"server/*.luau\"]\n";
    std::fs::write(probe.join("resource.toml"), manifest).unwrap();
    std::fs::write(probe.join("server/main.luau"), PROBE).unwrap();
    let o = ops_with(
        |c| {
            c.resources.folder = folder.clone();
            c.resources.start = vec!["probe".into()];
        },
        "",
    );
    let mut player = Player::new();
    let up = (0..40).any(|_| {
        std::thread::sleep(Duration::from_millis(500));
        ran(&o, "scripts").iter().any(|l| l.contains("probe"))
    });
    assert!(up, "the probe runs");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(refusal_text(join(&mut player, o.addr, "First")), "held", "the defaults");
    for line in ["acl create Quiet", "acl add Quiet resource.probe", "acl deny Quiet function.getPlayers"] {
        let said = ran(&o, line);
        assert!(!said.is_empty(), "{line}: {said:?}");
    }
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(
        refusal_text(join(&mut player, o.addr, "Second")),
        "access denied: probe does not hold function.getPlayers: the group Quiet denies function.getPlayers"
    );
    assert!(!ran(&o, "acl revoke Quiet function.getPlayers").is_empty());
    std::thread::sleep(Duration::from_millis(1100));
    assert_eq!(refusal_text(join(&mut player, o.addr, "Third")), "held", "revoked");
    o.server.stop();
}
