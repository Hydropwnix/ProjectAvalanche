use std::sync::Arc;

use ssx_content::policy::ClientMods;
use ssx_content::{Asset, ContentId, Layer, LoadError, MemorySource};

use super::super::store::GameContent;

fn store(key: &str, mounts: Vec<(Layer, MemorySource)>) -> Arc<GameContent> {
    let root = std::env::temp_dir().join(format!("ssx-net8-{key}-{}-absent", std::process::id()));
    let content = GameContent::of(&root);
    let mut r = content.registry.write().unwrap();
    for (layer, source) in mounts {
        r.mount(layer, 0, Box::new(source));
    }
    drop(r);
    content
}

fn gamemode(limit_s: u32) -> Asset {
    let text = format!(
        "[gamemode]\ntitle = \"Race Clock\"\nevent = \"race_clock\"\ntime_limit_s = {limit_s}.0\nany_map = true\n"
    );
    Asset::Value(Arc::new(ssx_content::gamemode::parse_toml(text.as_bytes()).expect("a gamemode")))
}

const SKIPPED: &str = "a client mod's, and a network session rides the server's maps and gamemodes only";

#[test]
fn a_session_rides_the_servers_gamemode_and_map_never_a_client_mods() {
    let g: ContentId = "gm:gamemode/race_clock".parse().unwrap();
    let only_client: ContentId = "mine:gamemode/race_clock".parse().unwrap();
    let map: ContentId = "mine:map/course".parse().unwrap();
    let content = store(
        "session-skip",
        vec![
            (
                Layer::ClientMods,
                MemorySource::new("client")
                    .with(g.clone(), gamemode(30))
                    .with(only_client.clone(), gamemode(45))
                    .with(map.clone(), Asset::Record(toml::Table::new())),
            ),
            (Layer::Server, MemorySource::new("server").with(g.clone(), gamemode(90))),
        ],
    );
    let limit = |id: &ContentId| content.gamemode(id).map(|f| (f.layer, f.source.clone(), f.value.time_limit_ms));
    let refused = |e: LoadError| e.to_string();

    assert_eq!(content.session_policy(), None);
    assert_eq!(limit(&g).unwrap(), (Layer::Server, "server".into(), Some(90_000)));
    assert_eq!(limit(&only_client).unwrap(), (Layer::ClientMods, "client".into(), Some(45_000)));
    let e = refused(content.map(&map).map(|_| ()).unwrap_err());
    assert!(e.contains("not a decoded map"), "{e}");

    for policy in [ClientMods::Any, ClientMods::Presentation, ClientMods::None] {
        content.set_session_policy(Some(policy));
        assert_eq!(limit(&g).unwrap(), (Layer::Server, "server".into(), Some(90_000)), "{policy:?}");
        let e = refused(content.gamemode(&only_client).map(|_| ()).unwrap_err());
        assert!(e.contains(&format!("from client: {SKIPPED}")), "{policy:?}: {e}");
        let e = refused(content.map(&map).map(|_| ()).unwrap_err());
        assert!(e.contains(&format!("from client: {SKIPPED}")), "{policy:?}: {e}");
    }

    content.set_session_policy(None);
    assert_eq!(limit(&only_client).unwrap(), (Layer::ClientMods, "client".into(), Some(45_000)));
}
