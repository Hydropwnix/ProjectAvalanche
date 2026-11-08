use std::io::Read;
use std::ops::Range;
use std::path::Path;

use ssx_net_proto::Mode;
use ssx_net_proto::instance::Level;
use ssx_rules::subset::{EventKind, RulesData, TUTORIAL_DROPS};

use crate::config::Config;

const MAX_TEXT: u64 = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct DropFinding {
    pub place: String,
    pub what: String,
    pub problem: bool,
}

pub fn check_configured(config: &Config, data_dir: &Path) -> Result<Vec<DropFinding>, String> {
    let folder = super::folder(config, data_dir);
    let events = ssx_assets::content::data::events::ui_drop_records(&folder).map_err(|e| format!("{e:#}"))?;
    let rules = RulesData::with_events(&folder, events).map_err(|e| format!("{e:#}"))?;
    let (file, text) = match &config.path {
        Some(p) => (p.display().to_string(), read_bounded(p)),
        None => ("(the defaults)".to_string(), None),
    };
    let places = text.as_deref().and_then(crate::game_config::drop_places);
    let resources = !config.resources.start.is_empty();
    let mut out = Vec::new();
    for (k, g) in config.instances.iter().enumerate() {
        let kind = super::kind_of(g.spec.mode);
        let levels: Vec<&Level> = match &g.rounds {
            Some((_, rotation)) => rotation.iter().collect(),
            None => vec![&g.spec.level],
        };
        let written = places.as_ref().and_then(|p| p.get(k));
        for (n, d) in levels.into_iter().filter_map(Level::as_drop).enumerate() {
            let span = written.and_then(|w| w.get(n)).filter(|(name, _)| name == d).and_then(|(_, s)| s.clone());
            let place = match (&text, span) {
                (Some(t), Some(s)) => position(&file, t, &s),
                _ => file.clone(),
            };
            let say = |what: String, problem: bool| DropFinding { place: place.clone(), what, problem };
            let event = rules.event(d);
            let usable = event.is_some_and(|e| !TUTORIAL_DROPS.contains(&e.key));
            if let Err(e) = rules.check(d, kind) {
                let what = format!("instance {}: {e:#}", k + 1);
                match usable {
                    true if resources => out.push(say(
                        format!("{what} in the disc's records; a resource the server starts may change it, which the start checks"),
                        false,
                    )),
                    _ => out.push(say(what, true)),
                }
            }
            if usable
                && g.spec.mode != Mode::Freeride
                && let Err(e) = rules.check_start(d, EventKind::Any)
            {
                out.push(say(format!("instance {}: {e:#}", k + 1), true));
            }
        }
    }
    Ok(out)
}

fn read_bounded(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_TEXT).read_to_string(&mut text).ok()?;
    Some(text)
}

fn position(file: &str, text: &str, span: &Range<usize>) -> String {
    let (line, column) = ssx_content::manifest::line_col(text, span.start);
    format!("{file}:{line}:{column}")
}
