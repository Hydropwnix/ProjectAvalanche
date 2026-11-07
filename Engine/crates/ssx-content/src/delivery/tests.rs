use std::collections::BTreeSet;
use std::time::SystemTime;

use super::*;
use crate::manifest::Manifest;
use crate::path::PackageTree;
use crate::test_dir::TestDir;

fn h(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn body(files: &str, extra_level: &str) -> String {
    let id = SimContentId([7; 32]);
    let content = content_id(&[("drop:exp_alps_vorassay_drop_3".into(), id)]).unwrap();
    format!(
        r#"{{"format":1,"client_mods":"presentation","client_scripts":false,"mirrors":["http://127.0.0.1:8080/ssx/"],"content_id":"{content}",
        "resources":[{{"name":"skins","version":"1.0.0","title":"Skins","group":0,"scripts":false,"files":[{files}]}}],
        "levels":[{{"level":"drop:exp_alps_vorassay_drop_3","id":"{id}","files":"{}","rows":[]}}{extra_level}]}}"#,
        h(3)
    )
}

fn file(path: &str, size: u64, role: &str) -> String {
    format!(r#"{{"path":"{path}","hash":"{}","size":{size},"role":"{role}","lazy":false}}"#, h(1))
}

fn manifest_file() -> String {
    file("resource.toml", 40, "manifest")
}

#[test]
fn a_manifest_reads_back_as_written() {
    let m = from_json(body(&format!("{},{}", manifest_file(), file("t/a.png", 9, "content")), "").as_bytes())
        .expect("valid");
    assert_eq!((m.resources.len(), m.file_count(), m.required_bytes()), (1, 2, 49));
    assert_eq!(m.client_mods, ClientMods::Presentation);
    assert_eq!(from_json(m.to_json().as_bytes()), Ok(m));
}

#[test]
fn hostile_manifests_are_refused_with_the_member() {
    let refuse = |files: String, level: &str, what: &str| {
        let e = from_json(body(&files, level).as_bytes()).expect_err(what).to_string();
        assert!(e.contains(what), "{e} (expected {what})");
    };
    let m = manifest_file();
    refuse(format!("{m},{}", file("../etc/passwd", 1, "data")), "", "etc/passwd");
    refuse(format!("{m},{}", file("/abs", 1, "data")), "", "abs");
    refuse(format!("{m},{}", file("C:/x", 1, "data")), "", "C:/x");
    refuse(format!("{m},{}", file("con.png", 1, "data")), "", "con.png");
    refuse(format!("{m},{},{}", file("a.png", 1, "data"), file("A.png", 1, "data")), "", "twice");
    refuse(format!("{m},{}", file("a.png", MAX_FILE_BYTES_FOR_TEST + 1, "data")), "", "at most");
    refuse(format!("{m},{}", file("a.png", 1, "shader")), "", "role");
    refuse(file("a.png", 1, "data"), "", "no resource.toml");
    refuse(format!("{m},{}", file("resource.toml", 1, "data")), "", "twice");
    refuse(format!("{m},{},{}", file("a", 1, "data"), file("A/b.png", 1, "data")), "", "a file and a folder");
    refuse(m.clone(), r#",{"level":"drop:../x","id":"00","files":"00","rows":[]}"#, "levels");
    let wrong = from_json(body(&m, "").replace(r#""content_id":""#, r#""content_id":"0"#).as_bytes());
    assert!(wrong.is_err());
    let content = content_id(&[("drop:exp_alps_vorassay_drop_3".into(), SimContentId([7; 32]))]).unwrap();
    let none = body(&m, "").replace(&format!(r#""content_id":"{content}""#), r#""content_id":null"#);
    assert!(from_json(none.as_bytes()).unwrap_err().to_string().contains("levels without a content_id"));
    let unknown = body(&m, "").replacen(r#""format":1"#, r#""format":1,"extra":true"#, 1);
    assert!(from_json(unknown.as_bytes()).unwrap_err().to_string().contains("extra"));
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(from_json(deep.as_bytes()).is_err(), "nesting ends in the reader's limit, not the stack");
    assert!(from_json(&vec![b' '; MAX_BODY + 1]).unwrap_err().to_string().contains("at most"));
    let escape = body(&m, "").replacen(r#""name":"skins""#, "\"name\":\"\\u001b[2J\"", 1);
    let e = from_json(escape.as_bytes()).unwrap_err().to_string();
    assert!(!e.chars().any(|c| c.is_control()), "a server's text is escaped: {e:?}");
}

const MAX_FILE_BYTES_FOR_TEST: u64 = crate::limits::MAX_FILE_BYTES;

#[test]
fn a_content_id_needs_one_id_per_level() {
    let a = SimContentId([1; 32]);
    let b = SimContentId([2; 32]);
    let one = content_id(&[("drop:a".into(), a), ("drop:b".into(), b)]);
    assert_eq!(one, content_id(&[("drop:b".into(), b), ("drop:a".into(), a)]), "order does not matter");
    assert_ne!(one, content_id(&[("drop:a".into(), b), ("drop:b".into(), a)]), "the pairing does");
    assert_eq!(content_id(&[("drop:a".into(), a), ("drop:a".into(), a)]), None);
}

fn package(dir: &TestDir, extra: &str) -> (PackageTree, Manifest) {
    let text = format!("[resource]\nname = \"skins\"\nversion = \"1.0.0\"\napi = 1\n{extra}");
    dir.write("resource.toml", &text);
    let tree = PackageTree::open(&dir.0).expect("tree");
    let m = Manifest::parse(&text, std::path::Path::new("resource.toml")).expect("manifest");
    (tree, m)
}

#[test]
fn the_files_a_client_fetches() {
    let d = TestDir::new("delivery-files");
    d.write("textures/a.png", "png");
    d.write("client/hud.luau", "-- hud");
    d.write("server/rules.luau", "-- rules");
    d.write("server/data/secret.txt", "secret");
    d.write("big/movie.bin", "lazy");
    let (tree, m) = package(
        &d,
        "[override]\n\"ssx:texture/a\" = \"textures/a.png\"\n[scripts]\nclient = [\"client/*.luau\"]\n\
         [files]\nlazy = [\"big/**\"]\nserver_only = [\"server/data/**\"]\n",
    );
    let f = files_of(&tree, &m).expect("files");
    let by: Vec<(&str, FileRole, bool)> = f.files.iter().map(|f| (f.path.as_str(), f.role, f.lazy)).collect();
    assert_eq!(
        by,
        [
            ("big/movie.bin", FileRole::Data, true),
            ("client/hud.luau", FileRole::Script, false),
            ("resource.toml", FileRole::Manifest, false),
            ("server/rules.luau", FileRole::Data, false),
            ("textures/a.png", FileRole::Content, false),
        ]
    );
    assert_eq!((f.kept_back, f.server_scripts, f.client_scripts), (1, false, true));
    assert_eq!(f.files[4].hash, ContentHash::of(b"png"));
    let (tree, m) = package(&d, "[scripts]\nserver = [\"server/*.luau\"]\n");
    let f = files_of(&tree, &m).expect("files");
    assert!(f.server_scripts && f.files.iter().all(|f| f.path != "server/rules.luau"));
    let (tree, m) =
        package(&d, "[override]\n\"ssx:texture/a\" = \"textures/a.png\"\n[files]\nlazy = [\"textures/*\"]\n");
    assert!(files_of(&tree, &m).unwrap_err().contains("cannot be lazy"));
    let (tree, m) =
        package(&d, "[override]\n\"ssx:texture/a\" = \"textures/a.png\"\n[files]\nserver_only = [\"**\"]\n");
    assert!(files_of(&tree, &m).unwrap_err().contains("cannot be kept"));
}

#[test]
fn mirrors_are_plain_addresses() {
    let m = Mirror::parse("https://cdn.example.org/ssx/files").expect("valid");
    assert_eq!(m.to_string(), "https://cdn.example.org/ssx/files/");
    assert_eq!(m.file_url(&ContentHash([0xab; 32])), format!("https://cdn.example.org/ssx/files/{}", "ab".repeat(32)));
    assert_eq!(Mirror::parse("http://[::1]:8080").expect("v6").to_string(), "http://[::1]:8080/");
    let pad = |base: &str| format!("{base}{}", "a".repeat(super::mirror::MAX_MIRROR_URL - base.len()));
    assert!(Mirror::parse(&pad("http://h/x")).is_err());
    assert!(Mirror::parse(&format!("{}/", &pad("http://h/x")[..super::mirror::MAX_MIRROR_URL - 1])).is_ok());
    let v6 = "http://[1::2:3:4:5:6:7]/";
    assert_eq!(Mirror::parse(v6).expect("v6").to_string(), "http://[1:0:2:3:4:5:6:7]/");
    assert!(Mirror::parse(&format!("{v6}{}/", "b".repeat(super::mirror::MAX_MIRROR_URL - v6.len() - 1))).is_err());
    for bad in [
        "ftp://h/",
        "HTTPS://h/",
        "https://user@h/",
        "https://h/a/../b",
        "https://h/a//b",
        "https://h/%2e%2e/",
        "https://h/a?b",
        "https://h/a#b",
        "https://0.0.0.0/",
        "https://h:0/",
        "https://h:65536/",
        "https://-h/",
        "https://h/\\x",
        "https://h/ä",
    ] {
        assert!(Mirror::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_store_keeps_only_checked_files() {
    let d = TestDir::new("delivery-store");
    let store = FileStore::open(&d.0).expect("store");
    let (a, b) = (b"aaaa".as_slice(), b"bbbbbbbb".as_slice());
    let (ha, hb) = (ContentHash::of(a), ContentHash::of(b));
    let mut p = store.partial(&ha).expect("partial");
    assert!(matches!(store.partial(&ha), Err(StoreError::Busy(_))), "one fetcher at a time");
    p.append(&a[..2]).expect("append");
    p.append(b"zz").expect("append");
    assert!(matches!(store.confirm(p, 4), Err(StoreError::Mismatch(_))));
    assert!(!store.has(&ha, 4));
    let mut p = store.partial(&ha).expect("again");
    assert_eq!(p.len().expect("len"), 0, "a refused fetch starts over");
    p.append(a).expect("append");
    store.confirm(p, 4).expect("confirm");
    assert!(store.has(&ha, 4) && !store.has(&ha, 5));
    assert!(store.has(&ha, 4), "another size claimed for it does not remove an entry");
    std::fs::write(store.path_of(&ha), b"aa").expect("tear");
    assert!(!store.has(&ha, 4));
    let mut p = store.partial(&ha).expect("partial");
    p.append(a).expect("append");
    store.confirm(p, 4).expect("confirm over the torn entry");
    assert_eq!(store.read(&ha, 4).expect("read"), a);
    store.put(&hb, 8, &mut &b[..]).expect("put");
    assert!(store.put(&ContentHash::of(b"x"), 1, &mut &b"y"[..]).is_err());
    assert!(store.put(&hb, 4, &mut &b[..]).is_err(), "more bytes than the manifest says");
    assert_eq!(store.partial_len(&ContentHash::of(b"x")), None, "a refused put leaves no partial file");
    assert_eq!(store.partial_len(&hb), None);
    store.put(&hb, 8, &mut &b[..]).expect("put");
    assert_eq!(store.read(&hb, 8).expect("read"), b);
    std::fs::write(store.path_of(&hb), b"BBBBBBBB").expect("damage");
    assert!(matches!(store.read(&hb, 8), Err(StoreError::Mismatch(_))));
    assert!(!store.has(&hb, 8), "a changed entry is removed");
    store.put(&hb, 8, &mut &b[..]).expect("put");
    let old = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::options().write(true).open(store.path_of(&ha)).unwrap().set_modified(old).unwrap();
    assert_eq!(store.trim(8, &BTreeSet::new()).expect("trim"), (1, 4), "the older one goes");
    assert!(store.has(&hb, 8));
    assert_eq!(store.trim(0, &BTreeSet::from([hb])).expect("trim"), (0, 0), "kept");
    let (hc, hd) = (ContentHash::of(b"c"), ContentHash::of(b"d"));
    let mut held = store.partial(&hc).expect("partial");
    held.append(b"ccc").expect("append");
    store.partial(&hd).expect("partial").append(b"ddddd").expect("append");
    assert_eq!(store.partial_len(&hd), Some(5));
    assert_eq!(store.trim(8, &BTreeSet::from([hb])).expect("trim"), (1, 5), "the abandoned one goes");
    assert_eq!(store.partial_len(&hd), None);
    drop(held);
    assert_eq!(store.trim(8, &BTreeSet::from([hb])).expect("trim"), (1, 3), "and the other once let go");
    let mount = store.mount().expect("mount");
    let second = store.mount().expect("a second process's slot");
    let dir = mount.lay_out(&store, "skins", &[("textures/b.png", hb, 8)]).expect("lay out");
    assert_eq!(std::fs::read(dir.join("textures/b.png")).unwrap(), b);
    assert!(mount.lay_out(&store, "skins", &[("../x", hb, 8)]).is_err());
    assert!(mount.lay_out(&store, "../x", &[]).is_err());
    assert!(matches!(mount.lay_out(&store, "skins", &[("a", ha, 4)]), Err(StoreError::Missing(_))));
    drop((mount, second));
    assert!(!dir.exists(), "the slot is emptied with its mount");
}

#[test]
fn a_level_name_is_a_drop_or_a_map_in_its_mode() {
    assert_eq!(LevelName::parse("drop:exp_alps_vorassay_drop_3"), Ok(LevelName::Drop("exp_alps_vorassay_drop_3")));
    let map: ContentId = "slope:map/course".parse().unwrap();
    for mode in ["race", "trick", "survive", "death", "freeride", "gm9:gamemode/race_bots"] {
        let name = LevelName::map_name(map.as_str(), mode);
        assert_eq!(LevelName::parse(&name), Ok(LevelName::Map { id: map.clone(), mode }), "{name}");
    }
    assert_eq!(LevelName::drop_name("exp_x"), "drop:exp_x");
    for bad in [
        "map:slope:map/course",
        "map:slope:map/course@",
        "map:slope:gamemode/x@race",
        "map:slope:map/course@slope:map/other",
        "map:slope:map/course@sprint",
        "map:slope:map/course@race@race",
        "drop:",
        "drop:Exp",
        "drop:../x",
        "level:x",
        "map:\u{1b}[2J:map/x@race",
    ] {
        assert!(LevelName::parse(bad).is_err(), "{bad:?}");
    }
    let long = format!("map:s:map/{}@race", "a".repeat(crate::id::MAX_ID_LEN));
    assert!(LevelName::parse(&long).is_err());
    let e = LevelName::parse("map:\u{1b}[2J:map/x@race").unwrap_err();
    assert!(!e.chars().any(|c| c.is_control()), "a server's text is escaped: {e:?}");
}

#[test]
fn the_package_digest_covers_the_server_scripts() {
    let d = TestDir::new("delivery-digest");
    d.write("client/hud.luau", "-- hud");
    d.write("server/rules.luau", "-- rules");
    let manifest = "[scripts]\nclient = [\"client/*.luau\"]\nserver = [\"server/*.luau\"]\n";
    let (tree, m) = package(&d, manifest);
    let first = files_of(&tree, &m).expect("files");
    let (tree, m) = package(&d, manifest);
    assert_eq!(files_of(&tree, &m).expect("files").package, first.package, "the same files, the same digest");
    d.write("server/rules.luau", "-- rules, edited");
    let (tree, m) = package(&d, manifest);
    let edited = files_of(&tree, &m).expect("files");
    assert_eq!(edited.files, first.files, "what a client fetches is unchanged");
    assert_ne!(edited.package, first.package, "the server script's change shows in the digest");
    d.write("client/hud.luau", "-- hud, edited");
    let (tree, m) = package(&d, manifest);
    let client = files_of(&tree, &m).expect("files");
    assert_ne!(client.package, edited.package);
}
