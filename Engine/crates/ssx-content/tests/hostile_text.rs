use std::collections::BTreeMap;
use std::path::Path;

use ssx_content::model::FileReader;
use ssx_core::text::{LINE_CHARS, is_hidden};

const H: &str = "\u{1b}]0;PWNED\u{7}\u{1b}[2J\u{1b}[31m\u{9b}31m\u{202e}evil\u{2066}\u{200b}\u{200d}\u{feff}\u{2028}\r\nforged: \
                 ok\u{e0041}";

const H_SOFT: &str = "\u{202e}PWNED\u{2066}\u{200b}\u{200d}\u{feff}\u{2028}\u{2029}\u{61c}\u{e0041}";

fn big_record_text() -> String {
    H.repeat(200 * 1024 / H.len())
}

fn big_soft_text() -> String {
    H_SOFT.repeat(200 * 1024 / H_SOFT.len())
}

fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

thread_local! {
    static WRONG: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn checked(what: &str, messages: impl IntoIterator<Item = String>) -> usize {
    let mut quoting = 0;
    for m in messages {
        let n = m.chars().count();
        if m.chars().any(is_hidden) || n > LINE_CHARS {
            WRONG.with(|w| w.borrow_mut().push(format!("{what}: {n} characters: {:?}", cut(&m))));
        }
        quoting += usize::from(m.contains("PWNED") || m.contains("\\u{1b}") || m.contains("\\u{202e}"));
    }
    quoting
}

fn verdict(quoting: usize, least: usize) {
    let wrong = WRONG.with(|w| std::mem::take(&mut *w.borrow_mut()));
    assert!(wrong.is_empty(), "{} messages break the rule:\n{}", wrong.len(), wrong.join("\n"));
    assert!(quoting >= least, "{quoting} messages quoted the hostile text (at least {least} expected)");
}

fn cut(m: &str) -> String {
    m.chars().take(160).collect()
}

fn problems(p: Vec<(usize, usize, String)>) -> Vec<String> {
    p.into_iter().map(|(l, c, m)| format!("{l}:{c}: {m}")).collect()
}

#[derive(Default)]
struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{}: absent", ssx_core::text::quoted(path)))?;
        if b.len() as u64 > max { Err("too large".into()) } else { Ok(b.clone()) }
    }
    fn size(&self, path: &str) -> Result<u64, String> {
        self.0.get(path).map(|b| b.len() as u64).ok_or_else(|| format!("{}: absent", ssx_core::text::quoted(path)))
    }
    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.read(path, u64::MAX)?;
        let o = usize::try_from(offset).map_err(|e| e.to_string())?;
        let l = usize::try_from(len).map_err(|e| e.to_string())?;
        b.get(o..o.saturating_add(l)).map(<[u8]>::to_vec).ok_or_else(|| "short".into())
    }
}

fn record_cases(fields: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for (h, big) in [(toml_str(H), toml_str(&big_record_text())), (toml_str(H_SOFT), toml_str(&big_soft_text()))] {
        out.extend([format!("{h} = 1\n"), format!("[{h}]\nx = 1\n")]);
        out.extend(fields.iter().map(|f| format!("{f} = {h}\n")));
        out.push(fields.iter().map(|f| format!("{f} = [{h}]\n")).collect());
        out.push(format!("{big} = 1\n"));
        out.push(format!("{} = {big}\n", fields.first().copied().unwrap_or("x")));
    }
    out
}

type RecordReader = fn(&[u8]) -> Vec<String>;

#[test]
fn records() {
    use ssx_content::audio::sounds::SoundClass;
    let readers: Vec<(&str, &[&str], RecordReader)> = vec![
        ("object", &["model", "lod_thresholds"], |b| {
            ssx_content::object::parse_toml(b, "objects/o.toml").err().map(problems).unwrap_or_default()
        }),
        ("material", &["family", "effect"], |b| {
            ssx_content::material::parse_toml(b, "materials/m.toml").err().map(problems).unwrap_or_default()
        }),
        ("sky", &["type", "faces", "color"], |b| {
            ssx_content::sky::parse_toml(b, "skies/s.toml").err().map(problems).unwrap_or_default()
        }),
        ("sound", &["files", "variation", "voices", "bus", "loop", "others"], |b| {
            ssx_content::audio::sounds::parse_toml(b, SoundClass::Effect).err().map(problems).unwrap_or_default()
        }),
        ("music", &["type", "file", "Artist", "Title"], |b| {
            ssx_content::audio::music::parse_toml(b, None).err().map(problems).unwrap_or_default()
        }),
        ("marker", &["shape", "color", "sound", "model", "facing"], |b| {
            ssx_content::marker::parse_toml(b).err().map(problems).unwrap_or_default()
        }),
        ("character", &["stands_in_for"], |b| {
            ssx_content::rider::parse_character(b, "characters/c.toml").err().map(problems).unwrap_or_default()
        }),
        ("item", &["model", "slot"], |b| {
            ssx_content::rider::parse_item(b, false).err().map(problems).unwrap_or_default()
        }),
        ("lighting", &["preset"], |b| ssx_content::lighting::parse_toml(b).err().map(problems).unwrap_or_default()),
        ("postfx", &["preset"], |b| ssx_content::postfx::parse_toml(b).err().map(problems).unwrap_or_default()),
        ("particles", &["texture", "blend"], |b| {
            ssx_content::particles::parse_system(b).err().map(problems).unwrap_or_default()
        }),
        ("textures.toml", &["kind"], |b| {
            ssx_content::texture::FolderSettings::parse(b, "textures/textures.toml").err().into_iter().collect()
        }),
        ("record", &["x"], |b| ssx_content::package::parse_record(b, "data/r.toml").err().into_iter().collect()),
    ];
    let mut quoting = 0;
    for (what, fields, read) in readers {
        for (i, body) in record_cases(fields).into_iter().enumerate() {
            quoting += checked(&format!("{what} case {i}"), read(body.as_bytes()));
        }
    }
    verdict(quoting, 20);
}

#[test]
fn data_records() {
    use ssx_content::data::{Mode, check_text, kinds};
    let schemas = [
        ("character", kinds::character()),
        ("item", kinds::item(false)),
        ("marker", kinds::marker()),
        ("map", kinds::map()),
        ("anim", kinds::anim()),
    ];
    let h = toml_str(H);
    let mut quoting = 0;
    for (what, schema) in &schemas {
        let bodies = [
            format!("{h} = {h}\n"),
            format!("[{h}]\n{h} = 1\n"),
            format!("[career]\n{h} = [{h}]\n"),
            format!("{} = 1\n", toml_str(&big_record_text())),
        ];
        for (i, body) in bodies.iter().enumerate() {
            for mode in [Mode::Whole, Mode::Patch] {
                let found = check_text(schema, body.as_bytes(), mode, None).err().unwrap_or_default();
                let lines = found.iter().map(|p| format!("{}:{}: {}: {}", p.line, p.column, p.path, p.message));
                quoting += checked(&format!("data {what} case {i}"), lines);
            }
        }
    }
    verdict(quoting, 10);
}

#[test]
fn manifests() {
    let h = toml_str(H);
    let head = |extra: &str| {
        format!(
            "[resource]\nname = \"m\"\nversion = \"0.1.0\"\ntitle = \"t\"\nauthors = []\ndescription = \"\"\napi = 1\n\
             {extra}\n"
        )
    };
    let cases = [
        head(&format!("{h} = 1")),
        format!("{}{h} = 1\n", head("")),
        format!(
            "[resource]\nname = {h}\nversion = {h}\ntitle = {h}\nauthors = [{h}]\ndescription = {h}\napi = 1\ntype = {h}\n"
        ),
        format!("{}[add]\n{h} = {h}\n\"texture/a\" = {h}\n\"object/b\" = {{ {h} = \"x\" }}\n", head("")),
        format!("{}[override]\n{} = \"t.png\"\n\"ssx:texture/x\" = {h}\n", head(""), toml_str(&format!("texture/{H}"))),
        format!("{}[patch]\n{h} = {h}\n", head("")),
        format!("{}[dependencies]\nrequire = [{h}, {}]\nprovide = [{h}]\n", head(""), toml_str(&format!("x {H}"))),
        format!("{}[scripts]\nclient = [{h}]\n[files]\ninclude = [{h}]\n[shaders]\nfiles = [{h}]\n", head("")),
        format!("{}[settings]\n{h} = {h}\n[actionscript]\n{h} = 1\n[permissions]\n{h} = 1\n", head("")),
        head(&format!("{} = 1", toml_str(&big_record_text()))),
        format!("[resource]\nname = {}\n", toml_str(&big_record_text())),
    ];
    let mut quoting = 0;
    for (i, text) in cases.iter().enumerate() {
        let Err(e) = ssx_content::Manifest::parse(text, Path::new(&format!("pkg/{H}/resource.toml"))) else {
            continue;
        };
        let lines: Vec<String> = e.to_string().lines().map(str::to_string).collect();
        assert!(lines.len() <= 1 + e.diagnostics.len(), "manifest case {i}: one line a diagnostic: {lines:?}");
        quoting += checked(&format!("manifest case {i}"), lines);
        let diagnostics = e.diagnostics.iter().map(|d| format!("{}:{}: {}", d.line, d.column, d.message));
        quoting += checked(&format!("manifest case {i} diagnostics"), diagnostics);
    }
    verdict(quoting, 10);
}

#[test]
fn strings_tables() {
    use ssx_content::strings::{Lang, parse_table};
    let h = toml_str(H);
    let cases = [
        format!("{h} = \"x\"\n"),
        format!("TXT_REMAINING = {h}\n"),
        format!("{} = \"y\"\n", toml_str(&format!("ok_{H}"))),
        format!("TXT_REMAINING = {}\n", toml_str("<font color='\u{1b}[31m'>x</font>")),
        format!("{} = \"z\"\n", toml_str(&big_record_text())),
        format!("TXT_REMAINING = {}\n", toml_str(&big_record_text())),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<font color='{H_SOFT}'>x</font>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<font kerning='{H_SOFT}'>x</font>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<p align='{H_SOFT}'>x</p>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<font {H_SOFT}='1'>x</font>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<font {H_SOFT}>x</font>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<b{H_SOFT}>x</b>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("<b>x</b{H_SOFT}>"))),
        format!("TXT_REMAINING = {}\n", toml_str(&format!("x^{H_SOFT}^y"))),
        format!("{} = \"z\"\n", toml_str(&big_soft_text())),
        format!("{} = \"z\"\n", toml_str(&format!("txt_ava_{H_SOFT}"))),
    ];
    let mut quoting = 0;
    for (i, body) in cases.iter().enumerate() {
        for ns in ["ssx", "pkg"] {
            let found = parse_table(body.as_bytes(), "strings/en.toml", Lang::En, ns).err().unwrap_or_default();
            quoting += checked(&format!("strings case {i} ({ns})"), problems(found));
        }
    }
    let spec = |s: &str| format!("%1[{}]s", s.replace(']', ""));
    for (text, base) in [
        (format!("x^{H_SOFT}^y"), "x^A^y"),
        (format!("%1s {H_SOFT}"), "%1d"),
        (H.to_string(), "^A^"),
        (spec(H), "%1s"),
        (spec(&big_record_text()), "%1d"),
    ] {
        quoting += checked("strings against", ssx_content::strings::check_against(&text, Some(base)).err());
    }
    verdict(quoting, 5);
}

fn hostile_gltf(variant: usize, text: &str) -> serde_json::Value {
    use serde_json::json;
    let pos: Vec<u8> = [0f32, 0., 0., 1., 0., 0., 0., 1., 0.].iter().flat_map(|f| f.to_le_bytes()).collect();
    let b64 = |b: &[u8]| {
        const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
        for c in b.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for k in 0..4 {
                s.push(if k <= c.len() { A[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
            }
        }
        s
    };
    let uri = format!("data:application/octet-stream;base64,{}", b64(&pos));
    let mut g = json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "name": "n"}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "buffers": [{"byteLength": pos.len(), "uri": uri}],
        "bufferViews": [{"buffer": 0, "byteLength": pos.len()}],
        "accessors": [{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                       "min": [0, 0, 0], "max": [1, 1, 0]}],
    });
    match variant {
        0 => g[text] = json!(1),
        1 => g["extensionsRequired"] = json!([text]),
        2 => g["extensionsUsed"] = json!([text, format!("KHR_{text}")]),
        3 => g["buffers"][0]["uri"] = json!(format!("{text}.bin")),
        4 => g["buffers"][0]["uri"] = json!(format!("data:{text};base64,AAAA")),
        5 => g["nodes"][0]["extras"] = json!({"ava_surface": text, "ava_collision": true}),
        6 => g["nodes"][0]["name"] = json!(format!("{text}_lod1")),
        7 => g["nodes"][0][text] = json!(1),
        8 => g["nodes"][0]["extras"] = json!({ text: 1 }),
        9 => g["meshes"][0]["primitives"][0]["attributes"] = json!({ text: 0 }),
        10 => g["asset"]["version"] = json!(text),
        _ => g["accessors"][0]["type"] = json!(text),
    }
    g
}

#[test]
fn gltf_models() {
    use ssx_content::model::{Limits, decode_file, probe_file};
    let big = H.repeat((1 << 20) / H.len());
    let limits = Limits::default();
    let mut quoting = 0;
    for variant in 0..12 {
        for text in [H, big.as_str()] {
            let json = serde_json::to_vec(&hostile_gltf(variant, text)).expect("JSON");
            let rel = format!("models/{}.gltf", if text.len() > 100 { "big" } else { "m" });
            let mut files = Files::default();
            files.0.insert(rel.clone(), json.clone());
            let mut out = Vec::new();
            if let Err(e) = decode_file(&rel, &json, &files, &limits) {
                out.push(e.message.clone());
                out.push(e.to_string());
            }
            if let Err(e) = probe_file(&rel, &files, &limits) {
                out.push(e.to_string());
            }
            quoting += checked(&format!("glTF variant {variant} ({} bytes)", text.len()), out);
        }
    }
    for bad in [format!("{{\"asset\": {{\"version\": \"2.0\"}}, \"{H}\" "), format!("{{\"asset\": \"{H}\"}}")] {
        let mut files = Files::default();
        files.0.insert("models/j.gltf".into(), bad.clone().into_bytes());
        let e = decode_file("models/j.gltf", bad.as_bytes(), &files, &limits).err().map(|e| e.to_string());
        quoting += checked("glTF JSON", e);
    }
    verdict(quoting, 8);
}

fn map_scene(node: usize, key: &str, extra: bool, value: serde_json::Value) -> Vec<u8> {
    let mut g: serde_json::Value = serde_json::from_str(&ssx_content::map::template::gltf()).expect("template");
    if extra {
        g["nodes"][node]["extras"][key] = value;
    } else {
        g["nodes"][node][key] = value;
    }
    serde_json::to_vec(&g).expect("JSON")
}

#[test]
fn maps() {
    use serde_json::json;
    use ssx_content::map::{parse_toml, read_map, template::MAP_TOML};
    let h = toml_str(H);
    let big = H.repeat((1 << 20) / H.len());
    let mut quoting = 0;
    for body in [
        format!("{h} = 1\n{MAP_TOML}"),
        MAP_TOML.replace("title = \"Template slope\"", &format!("title = {h}")),
        MAP_TOML.replace("intro = \"spawn\"", &format!("intro = {h}\n{h} = 1")),
        MAP_TOML.replace("modes = [\"race\", \"freeride\"]", &format!("modes = [{h}]\nbase = {h}\nmidground = {h}")),
        MAP_TOML.replace("scene = \"maps/course.gltf\"", &format!("scene = {h}")),
        format!("{} = 1\n{MAP_TOML}", toml_str(&big_record_text())),
    ] {
        quoting += checked("map.toml", parse_toml(body.as_bytes()).err().map(problems).unwrap_or_default());
        let e = read_map(body.as_bytes(), "map.toml", &Files::default()).err().map(|e| e.to_string());
        quoting += checked("map.toml read", e);
    }
    let cases: Vec<(usize, &str, bool, serde_json::Value)> = vec![
        (1, "name", false, json!(H)),
        (1, "ava_type", true, json!(H)),
        (1, "ava_surface", true, json!(H)),
        (2, "ava_surface", true, json!(big)),
        (3, "ava_slot", true, json!(H)),
        (4, "ava_order", true, json!(H)),
        (7, "ava_marker", true, json!(H)),
        (7, "ava_marker", true, json!(format!("ava:marker/{H}"))),
        (8, H, true, json!(1)),
        (0, "name", false, json!(big)),
        (5, "extras", false, json!(H)),
    ];
    for (node, key, extra, value) in cases {
        let mut files = Files::default();
        files.0.insert("maps/course.gltf".into(), map_scene(node, key, extra, value));
        let e = read_map(MAP_TOML.as_bytes(), "map.toml", &files).err();
        let lines = e.iter().flat_map(|e| [e.at.clone(), e.message.clone(), e.to_string()]);
        quoting += checked(&format!("map scene node {node} {}", cut(key)), lines);
    }
    verdict(quoting, 8);
}

#[test]
fn clips() {
    use ssx_content::anim::{read_anim, template, write};
    let clip = template::clip();
    let mut quoting = 0;
    for text in [H, H_SOFT] {
        let mut joints = template::joints();
        joints[0].name = text.into();
        for (name, joints) in [(text, template::joints()), (template::ANIMATION, joints)] {
            let Ok(glb) = write::glb(&joints, name, &clip) else { continue };
            let mut files = Files::default();
            files.0.insert(template::GLTF_PATH.into(), glb);
            for animation in [template::ANIMATION, text] {
                let toml = format!("[anim]\ngltf = {:?}\nanimation = {}\n", template::GLTF_PATH, toml_str(animation));
                let e = read_anim(toml.as_bytes(), "anims/a.toml", &files).err().unwrap_or_default();
                quoting += checked("clip glTF", e.iter().flat_map(|e| [e.message.clone(), e.to_string()]));
            }
        }
        let (h, a) = (toml_str(text), "[anim]\ngltf = \"anims/a.glb\"\nanimation = \"x\"\n");
        for body in [
            format!("{h} = 1\n{a}"),
            format!("[anim]\ngltf = {h}\nanimation = {h}\n{h} = 1\n"),
            format!("{a}[[anim.float]]\ndof = {h}\n"),
            format!("{a}[[anim.joint]]\njoint = {h}\n{h} = 1\n"),
            format!("{a}[[anim.joint]]\nname = {h}\n"),
            format!(
                "[anim]\ngltf = \"anims/a.glb\"\nanimation = {}\n",
                toml_str(&text.repeat(200 * 1024 / text.len()))
            ),
        ] {
            let e = read_anim(body.as_bytes(), "anims/a.toml", &Files::default()).err().unwrap_or_default();
            quoting += checked("clip record", e.iter().map(ToString::to_string));
        }
    }
    verdict(quoting, 3);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

fn zip_of(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let u16le = |v: usize| u16::try_from(v).expect("u16").to_le_bytes();
    let u32le = |v: usize| u32::try_from(v).expect("u32").to_le_bytes();
    for (name, data) in entries {
        let (offset, crc) = (out.len(), crc32(data).to_le_bytes());
        out.extend(b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x21\x00");
        out.extend(crc);
        out.extend(u32le(data.len()));
        out.extend(u32le(data.len()));
        out.extend(u16le(name.len()));
        out.extend([0, 0]);
        out.extend(*name);
        out.extend(*data);
        central.extend(b"PK\x01\x02\x14\x00\x14\x00\x00\x00\x00\x00\x00\x00\x21\x00");
        central.extend(crc);
        central.extend(u32le(data.len()));
        central.extend(u32le(data.len()));
        central.extend(u16le(name.len()));
        central.extend([0; 12]);
        central.extend(u32le(offset));
        central.extend(*name);
    }
    let at = out.len();
    out.extend(&central);
    out.extend(b"PK\x05\x06\x00\x00\x00\x00");
    out.extend(u16le(entries.len()));
    out.extend(u16le(entries.len()));
    out.extend(u32le(central.len()));
    out.extend(u32le(at));
    out.extend([0, 0]);
    out
}

#[test]
fn zip_archives() {
    use ssx_content::zip::{ZipArchive, ZipLimits};
    let long = H.repeat(60_000 / H.len());
    let names: Vec<Vec<u8>> = vec![
        H.as_bytes().to_vec(),
        format!("textures/{H}.png").into_bytes(),
        format!("a/../{H}").into_bytes(),
        long.into_bytes(),
        b"bad\xff\xfe\x9b2J\x1b]0;x\x07.png".to_vec(),
    ];
    let mut quoting = 0;
    for (i, name) in names.iter().enumerate() {
        let zip = zip_of(&[(b"resource.toml", b"x"), (name, b"x")]);
        let mut out = Vec::new();
        match ZipArchive::open(std::io::Cursor::new(zip), &ZipLimits::default()) {
            Err(e) => out.push(e.to_string()),
            Ok(mut z) => {
                for k in 0..z.entries().len() {
                    if let Err(e) = z.read(k, 1 << 20) {
                        out.push(e.to_string());
                    }
                }
            }
        }
        quoting += checked(&format!("zip name {i}"), out);
    }
    verdict(quoting, 3);
}

#[test]
fn a_package_checked_whole() {
    let root = std::env::temp_dir().join(format!("ssx-content-hostile-text-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let h = toml_str(H);
    let write = |rel: &str, text: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("folder");
        std::fs::write(p, text).expect("write");
    };
    write(
        "resource.toml",
        "[resource]\nname = \"hx\"\nversion = \"0.1.0\"\ntitle = \"t\"\nauthors = []\ndescription = \"\"\napi = 1\n\n\
         [add]\n\"object/o\" = \"objects/o.toml\"\n\"material/m\" = \"materials/m.toml\"\n\"map/p\" = \"map.toml\"\n",
    );
    write("objects/o.toml", &format!("{h} = 1\nmodel = {h}\n"));
    write("materials/m.toml", &format!("family = {h}\n"));
    write("map.toml", &format!("[map]\ntitle = {h}\nscene = {h}\n{h} = 1\n"));
    write("textures/textures.toml", &format!("{h} = {{ kind = {h} }}\n"));
    #[cfg(unix)]
    write(&format!("textures/{}.png", H.replace(['/', '\n'], "_")), "x");
    let checked_package = ssx_content::check::check_package(&root, &ssx_content::texture::Limits::default());
    let lines: Vec<String> = checked_package.findings.iter().map(ToString::to_string).collect();
    let _ = std::fs::remove_dir_all(&root);
    let quoting = checked("package check", lines);
    verdict(quoting, 3);
}
