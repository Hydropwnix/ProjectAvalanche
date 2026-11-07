use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::time::Duration;

use ssx_net_proto::dynamic::Dynamic;
use ssx_net_proto::info::ServerInfo;

use crate::common::*;
use crate::operation::{ops_with, ops_with_data};

pub fn icon(side: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, side, side);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Eight);
        let mut w = e.write_header().expect("header");
        let data: Vec<u8> = (0..side * side * 4).map(|i| if i % 4 == 3 { 255 } else { (i / 97 % 256) as u8 }).collect();
        w.write_image_data(&data).expect("data");
    }
    out
}

fn name_ok(n: &str, first_colon: bool) -> bool {
    let mut chars = n.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || (first_colon && c == ':'))
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || (first_colon && c == ':'))
}

fn sample(line: &str) -> Result<(String, BTreeMap<String, String>, f64), String> {
    let (head, value) = line.rsplit_once(' ').ok_or("no value")?;
    let value = match value {
        "+Inf" => f64::INFINITY,
        "-Inf" => f64::NEG_INFINITY,
        v => v.parse::<f64>().map_err(|_| format!("value {v:?}"))?,
    };
    let (name, mut rest) = match head.split_once('{') {
        Some((n, r)) => (n, r.strip_suffix('}').ok_or("labels without }")?),
        None => (head, ""),
    };
    if !name_ok(name, true) {
        return Err(format!("metric name {name:?}"));
    }
    let mut labels = BTreeMap::new();
    while !rest.is_empty() {
        let (label, after) = rest.split_once("=\"").ok_or("label without =\"")?;
        if !name_ok(label, false) || label.starts_with("__") {
            return Err(format!("label name {label:?}"));
        }
        let mut value = String::new();
        let mut chars = after.char_indices();
        let end = loop {
            match chars.next().ok_or("label value without its end")? {
                (_, '\\') => match chars.next().ok_or("an escape at the end")?.1 {
                    '\\' => value.push('\\'),
                    '"' => value.push('"'),
                    'n' => value.push('\n'),
                    c => return Err(format!("escape \\{c}")),
                },
                (i, '"') => break i,
                (_, c) => value.push(c),
            }
        };
        if labels.insert(label.to_string(), value).is_some() {
            return Err(format!("label {label} twice"));
        }
        rest = &after[end + 1..];
        rest = rest.strip_prefix(',').unwrap_or(rest);
    }
    Ok((name.to_string(), labels, value))
}

fn strict(text: &str) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let mut families = BTreeMap::new();
    let (mut current, mut kind) = (String::new(), String::new());
    let mut seen = BTreeSet::new();
    let mut values = Vec::new();
    let mut buckets: BTreeMap<String, Vec<(f64, f64)>> = BTreeMap::new();
    let mut counts: BTreeMap<String, f64> = BTreeMap::new();
    let lines: Vec<&str> = text.strip_suffix('\n').ok_or("no final newline")?.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(help) = line.strip_prefix("# HELP ") {
            let (name, _) = help.split_once(' ').ok_or("HELP without text")?;
            let ty = lines.get(i + 1).and_then(|l| l.strip_prefix(&format!("# TYPE {name} "))).ok_or("no TYPE")?;
            if !["counter", "gauge", "histogram"].contains(&ty)
                || families.insert(name.to_string(), ty.to_string()).is_some()
            {
                return Err(format!("family {name}: type {ty} or twice"));
            }
            (current, kind) = (name.to_string(), ty.to_string());
            i += 2;
            continue;
        }
        let (name, labels, value) = sample(line).map_err(|e| format!("line {}: {e}: {line}", i + 1))?;
        let base = match kind.as_str() {
            "histogram" => ["_bucket", "_sum", "_count"].iter().find_map(|s| name.strip_suffix(s)).unwrap_or(""),
            _ => name.as_str(),
        };
        if base != current {
            return Err(format!("line {}: {name} outside its family ({current})", i + 1));
        }
        if !seen.insert(line.rsplit_once(' ').map(|(h, _)| h.to_string()).unwrap_or_default()) {
            return Err(format!("line {}: a sample twice", i + 1));
        }
        if name.ends_with("_bucket") {
            let le = labels.get("le").ok_or("a bucket without le")?;
            let le = if le == "+Inf" { f64::INFINITY } else { le.parse().map_err(|_| "le")? };
            buckets.entry(current.clone()).or_default().push((le, value));
        } else if name.ends_with("_count") && kind == "histogram" {
            counts.insert(current.clone(), value);
        }
        values.extend(labels.into_values());
        i += 1;
    }
    for (family, b) in &buckets {
        let ordered = b.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 <= w[1].1);
        let last = b.last().ok_or("no bucket")?;
        if !ordered || last.0 != f64::INFINITY || Some(&last.1) != counts.get(family) {
            return Err(format!("{family}: buckets not cumulative to +Inf = count"));
        }
    }
    Ok((families, values))
}

#[test]
fn metrics_are_scraped_strictly_and_only_where_allowed() {
    let o = ops_with(|c| c.operation.metrics_enabled = true, "");
    let mut player = Player::new();
    let granted = player.join(o.addr, "Psymon", "").expect("joined");
    let (mut rider, e) = udp_client(&granted.token, o.addr, LOCAL, None);
    assert!(step_until(&mut [&mut rider], Duration::from_secs(5), |a| connected(a[0], e)));
    let _ = Player::new().join(o.addr, "Psymon", "wrong");
    step_until(&mut [&mut rider], Duration::from_secs(3), |_| false);
    let (status, body) = get(o.addr, LOCAL, "/metrics");
    assert_eq!(status, 200);
    let text = String::from_utf8(body).expect("UTF-8");
    let (families, values) = strict(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    for f in ["ssxa_tick_p99_seconds", "ssxa_instance_players", "ssxa_client_rtt_seconds", "ssxa_join_refusals_total"] {
        assert!(families.contains_key(f), "{f} missing:\n{text}");
    }
    assert!(text.contains("ssxa_players 1\n"), "{text}");
    let id = player.identity.id().to_hex();
    for v in &values {
        assert!(!v.contains("Psymon") && !v.contains("127.0.0.1"), "a player's name or address in a label: {v}");
        assert!(v.len() < 8 || !id.contains(v.as_str()), "a player's id in a label: {v}");
    }
    assert!(values.len() < 400, "labels bounded: {}", values.len());
    let other = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
    assert_eq!(get(o.addr, other, "/metrics").0, 403, "an address the list does not name");
    o.server.stop();
    o.server.join().expect("stops");
    let off = server();
    assert_eq!(get(off.started.addr, LOCAL, "/metrics").0, 404, "off by default");
    off.server_stop();
}

trait Stop {
    fn server_stop(self);
}

impl Stop for avalanche_server::ServerThread {
    fn server_stop(self) {
        self.stop();
        self.join().expect("stops");
    }
}

#[test]
fn dynamic_info_and_the_icon() {
    let png = icon(96);
    let hash = ssx_net_proto::icon::hash_hex(&png);
    let o = ops_with_data(
        |c| {
            c.operation.icon = Some("icon.png".into());
            c.discovery.region = "eu-west".into();
            c.discovery.tags = vec!["race".into(), "friendly".into()];
        },
        "",
        |_, conf: &Path| std::fs::write(conf.join("icon.png"), &png).expect("icon"),
    )
    .expect("starts with its icon");
    let info = ServerInfo::from_json(&get(o.addr, LOCAL, "/info").1).expect("/info 5");
    assert_eq!(
        (info.region.as_str(), info.tags.clone(), info.icon.as_str()),
        ("eu-west", vec!["race".to_string(), "friendly".to_string()], hash.as_str())
    );
    let dynamic = (0..20)
        .find_map(|_| {
            std::thread::sleep(Duration::from_millis(200));
            let (status, body) = get(o.addr, LOCAL, "/dynamic");
            (status == 200).then(|| Dynamic::from_json(&body).expect("strict"))
        })
        .expect("published within 4 s");
    assert!(dynamic.validate().is_ok() && dynamic.instances.len() == 1 && dynamic.players == 0, "{dynamic:?}");
    let (status, body) = get(o.addr, LOCAL, "/icon.png");
    assert_eq!((status, ssx_net_proto::icon::hash_hex(&body)), (200, hash.clone()));
    assert!(ssx_net_proto::icon::read_icon(&body).is_ok(), "the client reads it");
    let tag = format!("GET /icon.png HTTP/1.1\r\nHost: x\r\nIf-None-Match: \"{hash}\"\r\n\r\n");
    assert_eq!(raw(o.addr, LOCAL, tag.as_bytes(), Duration::from_secs(5)).0, 304, "kept by its entity tag");
    o.server.server_stop();
    let o = ops_with_data(
        |c| c.operation.icon = Some("icon.png".into()),
        "",
        |_, conf: &Path| std::fs::write(conf.join("icon.png"), &png).expect("icon"),
    )
    .expect("starts");
    assert!(ssx_net_join::browse::query::ask_icon(o.addr, &[0; 32]).is_err(), "another hash than asked is refused");
    let fetched = ssx_net_join::browse::query::ask_icon(o.addr, &ssx_net_proto::keys::from_hex(&hash).expect("hex"));
    assert!(fetched.is_ok(), "{fetched:?}");
    o.server.server_stop();
}

fn start_error(
    edit: impl FnOnce(&mut avalanche_server::config::Config),
    bans: &str,
    prepare: impl FnOnce(&Path, &Path),
) -> String {
    match ops_with_data(edit, bans, prepare) {
        Ok(o) => {
            o.server.server_stop();
            panic!("started")
        }
        Err(e) => e.to_string(),
    }
}

#[test]
fn hostile_icons_and_operator_files_stop_the_start_with_their_place() {
    let with_icon = |bytes: Vec<u8>| {
        start_error(
            |c| c.operation.icon = Some("icon.png".into()),
            "",
            move |_, conf: &Path| std::fs::write(conf.join("icon.png"), bytes).expect("icon"),
        )
    };
    assert!(with_icon(b"GIF89a not a png".to_vec()).contains("PNG"));
    assert!(with_icon(icon(95)).contains("96"));
    assert!(with_icon(icon(97)).contains("96"));
    let mut big = icon(96);
    big.resize(ssx_net_proto::icon::MAX_ICON_BYTES + 1, 0);
    assert!(with_icon(big).contains("bytes"));
    let mut cut = icon(96);
    cut.truncate(cut.len() - 20);
    assert!(!with_icon(cut).is_empty());
    let e = start_error(|c| c.operation.icon = Some("missing.png".into()), "", |_, _| {});
    assert!(e.contains("missing.png"), "{e}");
    let e = start_error(
        |_| {},
        "[[ban]]\nip = \"300.0.0.1\"\nreason = \"x\"\nby = \"console\"\nat = 1\nuntil = 0\n",
        |_, _| {},
    );
    assert!(e.contains("bans.toml: 2:"), "{e}");
    let e = start_error(
        |_| {},
        "",
        |d: &Path, _: &Path| {
            std::fs::write(d.join("acl.toml"), "[[group]]\nname = \"a b\"\nobjects = []\nallow = []\ndeny = []\n")
                .expect("acl")
        },
    );
    assert!(e.contains("acl.toml: 2:"), "{e}");
    let parse =
        |text: &str| avalanche_server::config::parse(Path::new("server.toml"), text).err().map(|e| e.to_string());
    for (text, want) in [
        ("[metrics]\nenabled = \"yes\"\n", "server.toml:2:"),
        ("[metrics]\nallow = [\"300.1.1.1\"]\n", "server.toml:2:"),
        ("[metrics]\nallow = [\"::1\", \"::1\"]\n", "twice"),
        ("[stop]\nround_end = 601\n", "server.toml:2:"),
        ("[stop]\nround_end = -1\n", "server.toml:2:"),
        ("[stop]\nsoon = 1\n", "server.toml:2:"),
        ("[server]\nicon = \"../icon.png\"\n", "server.toml:2:"),
        ("[server]\nicon = \"/etc/icon.png\"\n", "server.toml:2:"),
        ("[server]\nicon = \"icon.gif\"\n", "server.toml:2:"),
    ] {
        let e = parse(text).unwrap_or_else(|| panic!("accepted: {text}"));
        assert!(e.contains(want), "{text}: {e}");
    }
}
