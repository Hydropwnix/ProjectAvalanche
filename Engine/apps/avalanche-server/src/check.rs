use std::path::Path;

use crate::{config, data, keys};

pub fn run(config_path: &Path, data_dir: &Path, out: &mut dyn FnMut(String)) -> bool {
    let mut ok = true;
    let mut icon = None;
    let loaded = config::load(config_path);
    match &loaded {
        Ok(c) => {
            icon = crate::operation::config::icon_file(c);
            match &c.path {
                Some(p) if p.exists() => out(format!("{}: valid", p.display())),
                _ => out(format!("{}: missing, the defaults are used", config_path.display())),
            }
            out(format!("  name {:?}", c.name));
            out(format!("  description {:?}", c.description));
            out(format!("  game port {} (UDP and TCP){}", c.addr(), crate::bind::stack_note(c.bind)));
            out(format!("  max players {}", c.max_players));
            out(format!("  password {}", if c.password.is_empty() { "none (an open server)" } else { "set" }));
            out(format!("  chat {}", if c.chat { "on" } else { "off" }));
            for line in c.discovery.lines() {
                out(line);
            }
            for notice in &c.notices {
                out(format!("  note: {notice}"));
            }
            out(format!("  port mapping: {}", c.port_mapping.describe()));
            let m = &c.operation;
            let allow: Vec<String> = m.metrics_allow.iter().map(ToString::to_string).collect();
            out(format!(
                "  /metrics {}; the stop's round end at most {} s",
                if m.metrics_enabled { format!("on for {}", allow.join(", ")) } else { "off".into() },
                m.round_end_secs
            ));
            for (k, i) in c.instances.iter().enumerate() {
                let g = &i.spec;
                out(format!(
                    "  instance {}: {} on {}, rewinds {:?}, the event's explore ghosts {}, riders drawn as {}, at most {} \
                     riders",
                    k + 1,
                    g.mode.name(),
                    g.level,
                    g.rewinds,
                    if g.ghosts { "on" } else { "off" },
                    g.ghost_style.name(),
                    i.capacity.unwrap_or(c.max_players).min(c.max_players),
                ));
                if let Some((r, rotation)) = &i.rounds {
                    let s = |ticks: u32| ticks / ssx_net_proto::TICK_HZ;
                    out(format!(
                        "    rounds: at least {} riders, the lobby {} s, loading at most {} s, the countdown {} s, a \
                         round at most {} s, the results {} s; the rotation {}",
                        r.min_players,
                        s(r.lobby_wait),
                        s(r.load_timeout),
                        s(r.countdown),
                        s(r.round_limit),
                        s(r.results),
                        rotation.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
                    ));
                }
            }
        }
        Err(e) => {
            out(e.to_string());
            ok = false;
        }
    }
    let missing =
        [keys::NETCODE_FILE, keys::IDENTITY_FILE].iter().all(|f| std::fs::symlink_metadata(data_dir.join(f)).is_err());
    if missing {
        out(format!("{}: no keys yet; the first start makes them", data_dir.display()));
    } else {
        match keys::load(data_dir) {
            Ok(k) => out(format!("{}: keys valid; identity key {}", data_dir.display(), k.identity_key().to_hex())),
            Err(e) => {
                out(e.to_string());
                ok = false;
            }
        }
    }
    ok &= check_operation(data_dir, icon.as_deref(), out);
    let default_resources = std::path::PathBuf::from(crate::resources::config::DEFAULT_FOLDER);
    let configured = match &loaded {
        Ok(c) => data::guard::Configured::of(c),
        Err(_) => data::guard::Configured::resources(&default_resources),
    };
    let subset_read = match data::check_subset_configured(data_dir, true, &configured) {
        Ok((subset, findings)) => {
            let classes: Vec<&str> = subset.classes.iter().map(|c| c.name()).collect();
            out(format!(
                "{}: the rules subset ({}): {} members, {:.1} MB{}",
                data_dir.display(),
                classes.join(", "),
                subset.members,
                subset.bytes as f64 / 1e6,
                if subset.known_pressing { ", of the known pressing" } else { ", of another pressing" }
            ));
            for f in &findings {
                out(format!("{}: {}", f.place, f.what));
            }
            ok &= findings.is_empty();
            true
        }
        Err(findings) => {
            for f in &findings {
                out(format!("{}: {}", f.place, f.what));
            }
            ok = false;
            false
        }
    };
    if let (Ok(c), true) = (&loaded, subset_read) {
        match crate::rules::check_configured(c, data_dir) {
            Ok(findings) if findings.is_empty() => out("  the configured drops: each a drop of the rules".into()),
            Ok(findings) => {
                for f in &findings {
                    out(format!("{}: {}", f.place, f.what));
                    ok &= !f.problem;
                }
            }
            Err(e) => {
                out(format!(
                    "the configured drops: not checked: {}",
                    ssx_core::text::shown_max(&e, ssx_core::text::MESSAGE_CHARS)
                ));
                ok = false;
            }
        }
    }
    ok
}

fn check_operation(data_dir: &Path, icon: Option<&Path>, out: &mut dyn FnMut(String)) -> bool {
    let mut ok = true;
    match crate::operation::Operation::load(data_dir) {
        Ok(op) => {
            let (bans, acl) = (op.bans(), op.acl());
            out(format!(
                "{}: {} bans, {} mutes; {}: {} groups, {} resources' requests, the remote console {}",
                op.bans_path().display(),
                bans.bans().len(),
                bans.mutes().len(),
                op.acl_path().display(),
                acl.groups().len(),
                acl.requests().len(),
                if acl.has_operators() { "on (an operator key is in a group)" } else { "off (no operator key)" }
            ));
        }
        Err(e) => {
            out(e);
            ok = false;
        }
    }
    if let Some(path) = icon {
        match crate::operation::icon::load(path) {
            Ok(i) => out(format!("{}: the icon, 96 x 96, SHA-256 {}", path.display(), i.hash)),
            Err(e) => {
                out(e);
                ok = false;
            }
        }
    }
    ok
}
