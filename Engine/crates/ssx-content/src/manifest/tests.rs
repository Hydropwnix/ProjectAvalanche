use super::*;
use crate::limits::{MAX_LIST, MAX_MANIFEST_BYTES};
use crate::version::Op;

const EXAMPLE: &str = r#"
[resource]
name = "neon-pack"            # also the namespace of everything it adds: neon-pack:board/neon
version = "1.0.0"
title = "Neon boards"
authors = ["someone"]
description = "Glowing boards and an aurora sky"
api = 1                       # the API_LEVEL it needs
type = "mod"                  # mod | map | gamemode | script | library
priority = 0                  # load order among client mods: higher wins on the same id

[add]                         # new content; ids in this package's namespace (the kind and the name)
"board/neon" = { model = "models/neon_board.glb", data = "data/neon_board.toml" }

[override]                    # replace existing content, any namespace, the whole asset
"ssx:texture/boards/elise_default_d" = "textures/neon_board_d.png"
"ssx:lighting/dawn_lighting" = "data/aurora.toml"

[patch]                       # change fields of data records
"ssx:board/elise_default" = "data/patches/elise_board.toml"

[scripts]                     # Luau; server scripts never leave the server
client = ["client/*.luau"]
server = ["server/*.luau"]
shared = ["shared/*.luau"]

[files]                       # extra files the client scripts read
include = ["ui/*.png"]
lazy = []
server_only = ["server/data/**"]

[dependencies]
require = ["mapmanager >= 1.0, < 2"]   # version ranges are enforced
provide = []

[permissions]
request = []

[settings]
countdown_s = 5
"#;

fn parse(text: &str) -> Result<Manifest, ManifestError> {
    Manifest::parse(text, Path::new("pkg/resource.toml"))
}

fn error(text: &str) -> String {
    let e = parse(text).unwrap_err();
    assert_eq!(e.diagnostics.len(), 1, "{e}");
    let d = &e.diagnostics[0];
    format!("{}:{}: {}", d.line, d.column, d.message)
}

fn minimal(extra: &str) -> String {
    format!("[resource]\nname = \"pkg\"\nversion = \"0.1.0\"\napi = 1\n{extra}")
}

#[test]
fn the_plans_example() {
    let m = parse(EXAMPLE).unwrap();
    assert_eq!(m.resource.name, "neon-pack");
    assert_eq!(m.resource.version, Version::new(1, 0, 0));
    assert_eq!((m.resource.api, m.resource.kind, m.resource.priority), (1, PackageType::Mod, 0));
    let board = ContentId::parse("neon-pack:board/neon").unwrap();
    let Some(Entry::Files(files)) = m.add.get(&board) else { panic!("{:?}", m.add) };
    assert_eq!(files["model"], "models/neon_board.glb");
    assert_eq!(m.overrides.len(), 2);
    assert_eq!(
        m.entry(&"ssx:texture/boards/elise_default_d".parse().unwrap()),
        Some(&Entry::File("textures/neon_board_d.png".into()))
    );
    assert_eq!(m.patches.len(), 1);
    assert_eq!(m.scripts.client, ["client/*.luau"]);
    assert_eq!(m.files.server_only, ["server/data/**"]);
    let dep = &m.dependencies.require[0];
    assert_eq!(dep.name, "mapmanager");
    assert_eq!(dep.req.comparators, [(Op::Ge, Version::new(1, 0, 0)), (Op::Lt, Version::new(2, 0, 0))]);
    assert_eq!(m.settings["countdown_s"].as_integer(), Some(5));
    assert_eq!(m.content_ids().count(), 3);
    assert_eq!(m.ids_of(Kind::Texture).count(), 1);
}

#[test]
fn minimal_defaults() {
    let m = parse(&minimal("")).unwrap();
    assert_eq!((m.resource.kind, m.resource.priority, m.resource.title.as_str()), (PackageType::Mod, 0, ""));
    assert!(m.add.is_empty() && m.dependencies.require.is_empty() && m.settings.is_empty());
}

#[test]
fn unknown_keys_are_errors_with_position() {
    let e = error(&minimal("tittle = \"x\"\n"));
    assert!(e.starts_with("5:1: unknown field `tittle`"), "{e}");
    let e = error(&minimal("[scripts]\nclinet = []\n"));
    assert!(e.starts_with("6:1: unknown field `clinet`"), "{e}");
    let e = error(&minimal("[sound]\nx = 1\n"));
    assert!(e.starts_with("5:2: unknown field `sound`"), "{e}");
    let shown = parse(&minimal("tittle = 1\n")).unwrap_err().to_string();
    assert!(shown.starts_with("pkg/resource.toml:5:1: unknown field `tittle`"), "{shown}");
}

#[test]
fn syntax_and_type_errors_with_position() {
    assert!(error("[resource\nname = 1").starts_with("1:10:"));
    let e = error("[resource]\nname = \"pkg\"\nversion = \"0.1.0\"\napi = \"one\"\n");
    assert!(e.starts_with("4:7:"), "{e}");
    let e = error("[resource]\nname = \"pkg\"\napi = 1\n");
    assert!(e.contains("missing field `version`"), "{e}");
    let e = error(&minimal("type = \"plugin\"\n"));
    assert!(e.starts_with("5:8:") && e.contains("unknown variant `plugin`"), "{e}");
}

#[test]
fn resource_checks() {
    let e = error("[resource]\nname = \"Neon Pack\"\nversion = \"1.0.0\"\napi = 1\n");
    assert!(e.starts_with("2:8: bad package name"), "{e}");
    assert!(error("[resource]\nname = \"ssx\"\nversion = \"1.0.0\"\napi = 1\n").contains("reserved"));
    let e = error("[resource]\nname = \"p\"\nversion = \"1.0\"\napi = 1\n");
    assert!(e.starts_with("3:11: bad version"), "{e}");
    assert!(error("[resource]\nname = \"p\"\nversion = \"1.0.0\"\napi = 0\n").starts_with("4:7: api levels"));
}

#[test]
fn content_section_checks() {
    let e = error(&minimal("[add]\n\"Board/neon\" = \"m.glb\"\n"));
    assert!(e.starts_with("6:1: [add] \"Board/neon\": unknown kind"), "{e}");
    let e = error(&minimal("[add]\n\"texture/neon\" = \"textures/neon.glb\"\n"));
    assert!(e.starts_with("6:18: pkg:texture/neon: \"textures/neon.glb\" is not a texture file"), "{e}");
    let e = error(&minimal("[add]\n\"texture/neon\" = \"../neon.png\"\n"));
    assert!(e.contains("\"..\" segment"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = \"/etc/a.png\"\n"));
    assert!(e.contains("is absolute"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = \"C:\\\\a.png\"\n"));
    assert!(e.contains("contains ':'"), "{e}");
    let e = error(&minimal("[override]\n\"pkg:texture/a\" = \"a.png\"\n"));
    assert!(e.contains("goes in [add]"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:movie/intromovie\" = \"a.ogv\"\n"));
    assert!(e.starts_with("6:26: ssx:movie/intromovie: \"a.ogv\" is not a movie file (.webm)"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = 3\n"));
    assert!(e.contains("expected a file path"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = \"t/*.png\"\n"));
    assert!(e.contains("names one file"), "{e}");
}

#[test]
fn patch_checks() {
    let e = error(&minimal("[patch]\n\"ssx:texture/rock\" = \"p.toml\"\n"));
    assert!(e.starts_with("6:1: [patch] ssx:texture/rock: `texture` records cannot be patched"), "{e}");
    let e = error(&minimal("[patch]\n\"ssx:board/a\" = \"p.json\"\n"));
    assert!(e.starts_with("6:17: [patch] ssx:board/a: a patch is a .toml file"), "{e}");
    assert!(parse(&minimal("[patch]\n\"other-mod:map/x\" = \"patches/x.toml\"\n")).is_ok());
}

#[test]
fn scripts_files_and_dependencies() {
    assert!(error(&minimal("[scripts]\nclient = [\"client/x.lua\"]\n")).contains("must name `.luau` files"));
    assert!(error(&minimal("[files]\ninclude = [\"../x\"]\n")).contains("\"..\" segment"));
    let e = error(&minimal("[dependencies]\nrequire = [\"lib\", \"other ~1\"]\n"));
    assert!(e.starts_with("6:19: bad version requirement"), "{e}");
    assert!(error(&minimal("[dependencies]\nrequire = [\"pkg >= 1\"]\n")).contains("cannot require itself"));
    assert!(error(&minimal("[dependencies]\nrequire = [\"a\", \"a >= 2\"]\n")).contains("required twice"));
    assert!(error(&minimal("[dependencies]\nprovide = [\"Lib\"]\n")).contains("in provide"));
}

#[test]
fn several_errors_are_reported_together() {
    let e = parse(&minimal("[add]\n\"texture/a\" = \"a.wav\"\n\"sound/b\" = \"b.png\"\n")).unwrap_err();
    assert_eq!(e.diagnostics.iter().map(|d| d.line).collect::<Vec<_>>(), [6, 7]);
}

#[test]
fn positions_count_characters() {
    assert_eq!(line_col("ab\ncdé\nf", 0), (1, 1));
    assert_eq!(line_col("ab\ncdé\nf", 3), (2, 1));
    assert_eq!(line_col("ab\ncdé\nf", 8), (3, 1));
    assert_eq!(line_col("éé", 2), (1, 2));
    assert_eq!(line_col("x", 99), (1, 2));
}

#[test]
fn hostile_manifests() {
    let big = format!("{}\n{}", minimal(""), "#".repeat(MAX_MANIFEST_BYTES));
    assert!(parse(&big).unwrap_err().to_string().contains("bytes"));
    let e = error(&minimal("[add]\n\"texture/a\" = \"T/a.png\"\n\"texture/b\" = \"t/A.png\"\n"));
    assert!(e.starts_with("7:15: folder \"t\" differs only in case from \"T\" at line 6, column 15"), "{e}");
    let e = error(&minimal("[add]\n\"texture/a\" = \"Textures/a.png\"\n\"texture/b\" = \"textures/b.png\"\n"));
    assert!(
        e.starts_with("7:15: folder \"textures\" differs only in case from \"Textures\" at line 6, column 15"),
        "{e}"
    );
    let e = error(&minimal("[add]\n\"texture/a\" = \"t/a.png\"\n\"texture/b\" = \"t/A.png\"\n"));
    assert!(e.starts_with("7:15: path \"t/A.png\" differs only in case from \"t/a.png\" at line 6"), "{e}");
    let e = error(&minimal("[add]\n\"texture/a\" = \"x.png\"\n[patch]\n\"ssx:board/a\" = \"X.png/p.toml\"\n"));
    assert!(e.contains("folder \"X.png\" differs only in case from \"x.png\""), "{e}");
    assert!(parse(&minimal("[add]\n\"texture/a\" = \"t/a.png\"\n\"texture/b\" = \"t/b.png\"\n")).is_ok());
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = \"con.png\"\n"));
    assert!(e.contains("reserved device name"), "{e}");
    let e = error(&minimal("[override]\n\"ssx:texture/a\" = \"a b.png\"\n"));
    assert!(e.contains("contains ' '"), "{e}");
    let e = error(&format!("{}title = \"{}\"\n", minimal(""), "t".repeat(300)));
    assert!(e.starts_with("1:1: title"), "{e}");
    assert!(error(&format!("{}title = \"a\\u0007b\"\n", minimal(""))).contains("title"));
    assert!(error(&format!("{}description = \"a\\u001bb\"\n", minimal(""))).contains("description"));
    assert!(parse(&format!("{}description = \"line\\nline\\ttab\"\n", minimal(""))).is_ok());
    assert!(error(&format!("{}authors = [\"\"]\n", minimal(""))).contains("authors"));
    let deep = format!("[settings]\nx = {}1{}\n", "[".repeat(40), "]".repeat(40));
    assert!(error(&minimal(&deep)).contains("nested deeper than 32"));
    assert!(error(&minimal("[permissions]\nrequest = [\"a b\"]\n")).contains("[permissions]"));
    assert!(parse(&minimal("[permissions]\nrequest = [\"function.fetchRemote\"]\n")).is_ok());
    let e = error(&minimal("[add]\n\"board/x\" = { Model = \"m.glb\" }\n"));
    assert!(e.contains("bad role name"), "{e}");
    let e = error(&minimal("[add]\n\"board/x\" = { model = 3 }\n"));
    assert!(e.contains("must be a file path"), "{e}");
    let many = format!("[scripts]\nclient = [{}]\n", vec!["\"a.luau\""; MAX_LIST + 1].join(","));
    assert!(error(&minimal(&many)).contains("more than 1024"));
    let e = error(&minimal("[resource.extra]\n"));
    assert!(e.contains("duplicate") || e.contains("unknown field"), "{e}");
}
