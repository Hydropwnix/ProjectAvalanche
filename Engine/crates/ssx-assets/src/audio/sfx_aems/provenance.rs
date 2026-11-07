use super::*;

pub(super) fn player_events(program: &[Node], handles: &HashMap<u32, u32>) -> (Vec<u32>, HashMap<u32, Vec<u32>>) {
    let owner = |off: i64| -> Option<usize> {
        program.iter().position(|n| off >= i64::from(n.state) && off < i64::from(n.state) + i64::from(n.state_size))
    };
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); program.len()];
    for (a, n) in program.iter().enumerate() {
        for w in &n.wires {
            let Some(b) = owner(i64::from(n.state) + i64::from(w.dst)) else { continue };
            let from = match w.src {
                None => Some(a),
                Some(s) => owner(i64::from(n.state) + i64::from(s)),
            };
            if let Some(f) = from
                && !edges[b].contains(&f)
            {
                edges[b].push(f);
            }
        }
    }
    let players: Vec<u32> = program.iter().filter(|n| n.op == Op::Player).map(|n| n.state).collect();
    let mut out = HashMap::new();
    for (p, n) in program.iter().enumerate().filter(|(_, n)| n.op == Op::Player) {
        let (mut seen, mut todo) = (vec![false; program.len()], vec![p]);
        let mut events = Vec::new();
        while let Some(k) = todo.pop() {
            if std::mem::replace(&mut seen[k], true) {
                continue;
            }
            if program[k].op == Op::EventReceived
                && let Some(&key) = handles.get(&program[k].state)
                && !events.contains(&key)
            {
                events.push(key);
            }
            todo.extend(edges[k].iter().copied());
        }
        out.insert(n.state, events);
    }
    (players, out)
}

impl Aems {
    pub(super) fn choose(&mut self, inst: &mut Instance, st: u32, params: &[(u8, i32)]) -> Option<Choice> {
        if self.overrides.is_empty() && self.script_routes.is_empty() {
            return None;
        }
        let links = &self.banks[inst.bank].links[inst.module];
        let since = inst.started.insert(st, self.ticks).unwrap_or(0);
        let upstream = links.player_events.get(&st);
        let mut keys: Vec<u32> = Vec::new();
        for &(k, origin, t) in inst.received.iter().rev() {
            if t >= since && upstream.is_some_and(|u| u.contains(&k)) {
                keys.extend(origin);
                keys.push(k);
            }
        }
        let csis = self.csis.get(inst.csis).and_then(Option::as_ref);
        keys.extend(csis.and_then(|c| c.origin));
        keys.extend(csis.map(|c| c.function).or(links.function));
        keys.extend(inst.script);
        if let Some((origin, bus)) = keys.iter().find_map(|k| self.script_routes.get(k).map(|b| (*k, b.clone()))) {
            let player = links.players.iter().position(|&o| o == st).unwrap_or(0);
            if self.override_seen.insert((origin, inst.bank, inst.module, player)) || self.verbose {
                let bank = self.banks[inst.bank].name.clone();
                self.logf(format!("script sound: {bank} m{} p{player} → {bus}", inst.module));
            }
            return Some(Choice::Route(bus));
        }
        let (key, sound) = keys.iter().find_map(|k| self.overrides.get(k).map(|s| (*k, s.clone())))?;
        let player = links.players.iter().position(|&o| o == st).unwrap_or(0);
        let bank = self.banks[inst.bank].name.clone();
        let entry = self.name(key);
        let id9 = params.iter().find(|p| p.0 == 9).map(|p| p.1);
        let voice = id9.and_then(|id| self.voice_names.get(&id)).or_else(|| self.voice_names.get(&-1));
        let first = self.override_seen.insert((key, inst.bank, inst.module, player));
        let log = self.verbose || first;
        if !sound.takes(&bank, inst.module, player, voice.map(|(p, v)| (p.as_str(), v.as_str()))) {
            return match sound.others {
                Others::Keep => None,
                Others::Mute => {
                    if log {
                        let text = format!(
                            "override {} ({}): {bank} m{} p{player} muted",
                            sound.id, sound.package, inst.module
                        );
                        self.logf(text);
                    }
                    Some(Choice::Mute)
                }
            };
        }
        let draw = sound.draw()?;
        if !log {
            return Some(Choice::Play { sound, draw });
        }
        self.logf(format!(
            "override {} ({}) for {entry}: {bank} m{} p{player} plays {} × {:.3}",
            sound.id, sound.package, inst.module, draw.sample.file, draw.pitch
        ));
        Some(Choice::Play { sound, draw })
    }

    pub(super) fn remember(&self, inst: &mut Instance, key: u32, origin: Option<u32>) {
        let scripted = origin.is_some_and(|o| self.script_routes.contains_key(&o));
        if self.overrides.is_empty() && !scripted {
            return;
        }
        if inst.received.len() == RECEIVED {
            inst.received.remove(0);
        }
        inst.received.push((key, origin, self.ticks));
    }
}

use crate::sfx_override::Others;

#[derive(Clone, Debug)]
pub struct PlayerInfo {
    pub bank: usize,
    pub module: usize,
    pub player: usize,
    pub voice_id: Option<i32>,
    pub entries: Vec<SampleEntry>,
}

impl Aems {
    pub fn players_of(&self, key: u32) -> Vec<PlayerInfo> {
        let mut out = Vec::new();
        for (b, bank) in self.banks.iter().enumerate() {
            for (m, links) in bank.links.iter().enumerate() {
                let module = &bank.abk.modules[m];
                let by_function = links.function == Some(key);
                for (k, &st) in links.players.iter().enumerate() {
                    let by_event = links.player_events.get(&st).is_some_and(|e| e.contains(&key));
                    if !by_function && !by_event {
                        continue;
                    }
                    let base = (module.data_offset + INSTANCE_HEADER + st) as usize;
                    let data = &bank.data;
                    let np = rb(data, base + 0xE) as usize;
                    let voice_id =
                        (0..np).find(|&j| rb(data, base + 0x1C + 12 * j) == 9).map(|j| rd(data, base + 0x24 + 12 * j));
                    let table = rd(data, base + 4) as u32;
                    let entries = bank
                        .abk
                        .sample_tables
                        .iter()
                        .find(|t| t.offset == table)
                        .map(|t| t.entries.clone())
                        .unwrap_or_default();
                    out.push(PlayerInfo { bank: b, module: m, player: k, voice_id, entries });
                }
            }
        }
        out
    }
}
