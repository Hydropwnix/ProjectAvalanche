use super::*;

#[derive(Clone, Debug)]
pub struct CorePlugin {
    pub fourcc: [u8; 4],
    pub channels: usize,
    pub ctor: Vec<f64>,
}

pub type PluginInfo = ([u8; 4], usize, Vec<f64>);

const CORE_NAMESPACE: u8 = 200;

impl Graph {
    pub fn add_core_voice(&mut self, name: &str, level: u8, plugins: &[CorePlugin]) -> (usize, Option<usize>) {
        let mut input = None;
        let mut list = Vec::with_capacity(plugins.len());
        for (k, p) in plugins.iter().enumerate() {
            let outputs = p.channels.clamp(1, 8);
            let (n_ctor, ranges) = descriptor(&p.fourcc);
            let ctor: Vec<f64> = (0..p.ctor.len().max(n_ctor))
                .map(|i| p.ctor.get(i).copied().unwrap_or_else(|| ctor_default(&p.fourcc, i)))
                .collect();
            let mut stage = make_stage(&p.fourcc, outputs, &ctor);
            if let Stage::SubMix { input: inp } = &mut stage {
                let acc = self.accumulator(&format!("{name}#{k}"), CORE_NAMESPACE, outputs);
                *inp = Some(acc);
                input.get_or_insert(acc);
            }
            if let Stage::Send { key, .. } = &mut stage {
                *key = (String::new(), CORE_NAMESPACE);
            }
            list.push(Plugin {
                fourcc: p.fourcc,
                outputs,
                attrs: ranges.iter().map(|r| r.0).collect(),
                ranges,
                ctor,
                stage,
                bypass: false,
            });
        }
        let v = MixVoice {
            name: name.to_string(),
            level,
            vars: 0,
            plugins: list,
            mute: None,
            gain: VarRef { patch: 0, offset: 0, kind: 0 },
            mute_output: VarRef { patch: 0, offset: 0, kind: 0 },
            params: Vec::new(),
            source: None,
            core: true,
            first: true,
            meter: 0.0,
            rest: 0,
        };
        let slot = self.voices.iter().position(Option::is_none).unwrap_or(self.voices.len());
        if slot == self.voices.len() {
            self.voices.push(None);
        }
        self.voices[slot] = Some(v);
        let at = self
            .order
            .iter()
            .position(|&i| self.voices[i].as_ref().is_some_and(|o| o.level >= level))
            .unwrap_or(self.order.len());
        self.order.insert(at, slot);
        (slot, input)
    }

    fn plugin_mut(&mut self, slot: usize, plugin: usize) -> Option<&mut Plugin> {
        self.voices.get_mut(slot).and_then(Option::as_mut).and_then(|v| v.plugins.get_mut(plugin))
    }

    pub fn set_plugin_attr(&mut self, slot: usize, plugin: usize, attr: usize, value: f64) {
        if let Some(a) = self.plugin_mut(slot, plugin).and_then(|p| p.attrs.get_mut(attr)) {
            *a = value;
        }
    }

    pub fn attrs_by_name(&self, voice: &str, fourcc: &[u8; 4]) -> Option<Vec<f64>> {
        let v = self.voices.iter().flatten().find(|v| v.name.eq_ignore_ascii_case(voice))?;
        v.plugins.iter().find(|p| &p.fourcc == fourcc).map(|p| p.attrs.clone())
    }

    pub fn plugin_attr(&self, slot: usize, plugin: usize, attr: usize) -> Option<f64> {
        self.voices.get(slot)?.as_ref()?.plugins.get(plugin)?.attrs.get(attr).copied()
    }

    pub fn set_plugin_bypass(&mut self, slot: usize, plugin: usize, on: bool) {
        if let Some(p) = self.plugin_mut(slot, plugin) {
            p.bypass = on;
        }
    }

    pub fn set_send_target(&mut self, slot: usize, plugin: usize, target: Option<usize>) {
        if let Some(Stage::Send { target: t, key, .. }) = self.plugin_mut(slot, plugin).map(|p| &mut p.stage) {
            *t = target;
            *key = (String::new(), CORE_NAMESPACE);
        }
    }

    pub fn set_voice_source(&mut self, slot: usize, source: Option<u32>) {
        if let Some(v) = self.voices.get_mut(slot).and_then(Option::as_mut) {
            v.source = source;
        }
    }

    pub fn voice_plugins(&self, voice: &str) -> Option<Vec<PluginInfo>> {
        let v = self.voices.iter().flatten().find(|v| v.name == voice)?;
        Some(v.plugins.iter().map(|p| (p.fourcc, p.outputs, p.attrs.clone())).collect())
    }

    pub fn submix_input(&self, voice: &str) -> Option<usize> {
        let v = self.voices.iter().flatten().find(|v| v.name == voice)?;
        v.plugins.iter().find_map(|p| if let Stage::SubMix { input } = &p.stage { *input } else { None })
    }

    pub fn senders_into(&self, input: usize) -> usize {
        self.voices
            .iter()
            .flatten()
            .filter(|v| {
                v.plugins.iter().any(|p| match &p.stage {
                    Stage::Send { target, .. } | Stage::Route { target, .. } => *target == Some(input),
                    _ => false,
                })
            })
            .count()
    }

    pub fn add_bus_copies(&mut self, template: &str, names: &[&str]) -> usize {
        let def = self.set.buses.modules.iter().find_map(|m| match &m.kind {
            ModuleV1Kind::Voice(v) if v.name == template => Some(v.clone()),
            _ => None,
        });
        let vars = self.voices.iter().flatten().find(|v| v.name == template).map(|v| v.vars);
        let (Some(def), Some(vars)) = (def, vars) else { return 0 };
        let has = |f: &[u8; 4]| def.plugins.iter().any(|p| &p.fourcc == f);
        if !has(b"Sub0") || !has(b"Pn30") {
            return 0;
        }
        for name in names {
            if self.voices.iter().flatten().any(|v| v.name == *name) {
                continue;
            }
            let mut copy = def.clone();
            copy.name = (*name).to_string();
            for l in &mut copy.links {
                if def.plugins.get(usize::from(l.plugin)).is_some_and(|p| &p.fourcc == b"Sub0") {
                    l.target = (*name).to_string();
                }
            }
            self.add_voice(copy, vars, None);
        }
        names.iter().filter(|n| self.submix_input(n).is_some()).count()
    }

    pub fn redirect_voice_send(&mut self, slot: usize, target: Option<usize>) -> bool {
        let Some(v) = self.voices.get_mut(slot).and_then(Option::as_mut) else { return false };
        for p in &mut v.plugins {
            match &mut p.stage {
                Stage::Send { target: t, key, .. } => {
                    *t = target;
                    *key = (String::new(), CORE_NAMESPACE);
                    return true;
                }
                Stage::Route { last, .. } => {
                    p.stage = Stage::Send { last: *last, target, key: (String::new(), CORE_NAMESPACE) };
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    pub fn redirect_send(&mut self, voice: &str, target: Option<usize>) -> bool {
        let Some(v) = self.voices.iter_mut().flatten().find(|v| v.name == voice) else { return false };
        for p in &mut v.plugins {
            if let Stage::Send { target: t, key, .. } = &mut p.stage {
                *t = target;
                *key = (String::new(), CORE_NAMESPACE);
                return true;
            }
        }
        false
    }
}
