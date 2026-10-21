use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::sfx_aems::Aems;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Function,
    Event,
    Global,
}

#[derive(Clone, Debug)]
pub struct Route {
    pub kind: CallKind,
    pub target: Option<u32>,
    pub name: String,
    pub map: Vec<i32>,
    pub constants: Vec<i32>,
    pub delay: i32,
}

#[derive(Default)]
pub struct Tuna {
    pub routes: HashMap<u32, Vec<Route>>,
}

impl Tuna {
    pub fn load(big_root: &Path, aems: &Aems) -> Result<Self> {
        let mut t = Tuna::default();
        for p in crate::music::find_files(big_root, "data/xenon/audio/tuna", "xml") {
            if !p.to_string_lossy().ends_with("_mixmap.xml") {
                continue;
            }
            let text = std::fs::read_to_string(&p)?;
            let root = ssx_formats::xml::parse(text.trim_start_matches('\u{feff}'))
                .with_context(|| format!("parsing {}", p.display()))?;
            t.add(&root, aems);
        }
        Ok(t)
    }

    fn add(&mut self, root: &ssx_formats::xml::Element, aems: &Aems) {
        let num = |e: &ssx_formats::xml::Element, a: &str| e.attr(a).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        for module in root.children_named("Module") {
            for table in module.children_named("TunaMixTable") {
                let mut defs = HashMap::new();
                for d in table.children_named("OutputClassDef") {
                    let kind = match d.attr("csisObjectType") {
                        Some("class") => CallKind::Function,
                        Some("function") => CallKind::Event,
                        _ => CallKind::Global,
                    };
                    let id = (num(d, "systemCrc") as u32) << 16 | num(d, "interfaceCrc") as u32 & 0xFFFF;
                    let map: Vec<i32> = d
                        .attr("parameterMap")
                        .unwrap_or("")
                        .split(',')
                        .filter(|s| !s.trim().is_empty())
                        .map(|s| s.trim().parse().unwrap_or(-1))
                        .collect();
                    let name = d.attr("className").unwrap_or("").to_string();
                    defs.insert(name, (id, kind, map, num(d, "numParameters").max(0) as usize));
                }
                for ev in table.children_named("InputEvent") {
                    let name = ev.attr("name").unwrap_or("");
                    let Some(input) = aems.resolve(num(ev, "interfaceCrc") as u32 & 0xFFFF, name) else { continue };
                    for oc in ev.children_named("OutputClass") {
                        let class = oc.attr("className").unwrap_or("");
                        let Some((id, kind, map, n)) = defs.get(class) else { continue };
                        let mut constants = vec![0; (*n).max(map.len())];
                        for prm in oc.children_named("parameter") {
                            let k = num(prm, "order") as usize;
                            if (1..=constants.len()).contains(&k) {
                                constants[k - 1] = num(prm, "value") as i32;
                            }
                        }
                        let route = Route {
                            kind: *kind,
                            target: aems.resolve(*id, class),
                            name: class.to_string(),
                            map: map.clone(),
                            constants,
                            delay: num(oc, "signalDelay") as i32,
                        };
                        self.routes.entry(input).or_default().push(route);
                    }
                }
            }
        }
    }
}

impl Route {
    pub fn params(&self, input: &[i32]) -> Vec<i32> {
        let mut p = self.constants.clone();
        for (i, &m) in self.map.iter().enumerate() {
            if m >= 0
                && let (Some(v), Some(slot)) = (input.get(m as usize), p.get_mut(i))
            {
                *slot = *v;
            }
        }
        p
    }
}
