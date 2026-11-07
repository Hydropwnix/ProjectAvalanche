pub use ssx_content::delivery::config::{
    DEFAULT_FOLDER, MAX_NAME, MAX_START, Mods, RawMods, RawResources, Resources, parse_mods, parse_resources,
};

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::parse;
    use ssx_content::policy::ClientMods;

    fn err(text: &str) -> String {
        match parse(Path::new("server.toml"), text) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("accepted: {text}"),
        }
    }

    #[test]
    fn resources_and_mods_are_checked_with_their_place() {
        let d = parse(Path::new("s.toml"), "").expect("empty");
        assert_eq!((d.resources.clone(), d.mods), (Resources::default(), Mods::default()));
        let c = parse(
            Path::new("s.toml"),
            "[resources]\nfolder = \"/srv/res\"\nstart = [\"skins\", \"maps\"]\n\
             files_url = [\"http://127.0.0.1:8080/files\"]\n[mods]\nclient_mods = \"none\"\nclient_scripts = false\n",
        )
        .expect("valid");
        assert_eq!(c.resources.start, ["skins", "maps"]);
        assert_eq!(c.resources.mirrors[0].to_string(), "http://127.0.0.1:8080/files/");
        assert_eq!(c.mods, Mods { client_mods: ClientMods::None, client_scripts: false });
        assert!(err("[resources]\nstart = [\"Skins\"]\n").starts_with("server.toml:2:10: start: \"Skins\""));
        assert!(err("[resources]\nstart = [\"../x\"]\n").contains("not a resource name"));
        assert!(err("[resources]\nstart = [\"ssx\"]\n").contains("not a resource name"));
        let twice = err("[resources]\nstart = [\"a\", \"a\"]\n");
        assert!(twice.starts_with("server.toml:2:15: start: \"a\" twice"), "{twice}");
        assert!(err("[resources]\nfolder = \"\"\n").starts_with("server.toml:2:10: folder"));
        assert!(err("[resources]\nfolder = \"a\\u0000\"\n").contains("control"));
        assert!(err("[resources]\nfiles_url = [\"ftp://x/\"]\n").starts_with("server.toml:2:14: files_url"));
        assert!(err("[resources]\nfiles_url = [\"http://a/\", \"http://a\"]\n").contains("twice"));
        let five = (0..5).map(|i| format!("\"http://m{i}.example.org/\"")).collect::<Vec<_>>().join(", ");
        assert!(err(&format!("[resources]\nfiles_url = [{five}]\n")).contains("at most 4"));
        assert!(err("[resources]\nstart = \"skins\"\n").starts_with("server.toml:2:"));
        assert!(err("[resources]\nautostart = []\n").starts_with("server.toml:2:1"));
        assert!(err("[mods]\nclient_mods = \"some\"\n").starts_with("server.toml:2:15: client_mods = \"some\""));
        assert!(err("[mods]\nclient_scripts = \"yes\"\n").starts_with("server.toml:2:18"));
        assert!(err("[mods]\nrelay_cosmetics = true\n").starts_with("server.toml:2:1"));
    }
}
