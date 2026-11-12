use super::markers::{MarkerView, hit};
use super::{DrawList, GameFrame, Orders, RideEvent, SoundOrder};
use crate::host::call::{Dispatch, Errors, call_function, dispatch, enter};
use crate::host::elements::{Creator, ROOT};
use crate::host::{ScriptHost, Side, ops};
use crate::value::{ElementId, NetValue};

#[derive(Clone, Debug, Default)]
pub struct FrameReport {
    pub orders: Orders,
    pub markers: Option<Vec<MarkerView>>,
    pub skipped: u64,
}

fn s(v: &str) -> NetValue {
    NetValue::String(v.as_bytes().to_vec())
}

fn level_marker_id(node: &str) -> &str {
    let mut end = node.len().min(super::MAX_LEVEL_MARKER_ID);
    while !node.is_char_boundary(end) {
        end -= 1;
    }
    &node[..end]
}

fn is_session(key: u64) -> bool {
    key == 0 || (super::SESSION_KEYS..super::SESSION_KEYS + 0x1_0000).contains(&key)
}

fn session_rider(w: &crate::host::world::World, key: u64) -> Option<ElementId> {
    if key == 0 {
        let me = w.local_player?;
        return w.game.session.values().find(|(_, p)| *p == me).map(|(r, _)| *r);
    }
    let n = u16::try_from(key - super::SESSION_KEYS).ok()?;
    w.game.session.get(&n).map(|(r, _)| *r)
}

impl ScriptHost {
    pub fn frame(&self, mut f: GameFrame) -> Result<FrameReport, String> {
        if self.shared.side != Side::Client {
            return Err("the game's frame is a client's".into());
        }
        let events = std::mem::take(&mut f.events);
        let keys = std::mem::take(&mut f.keys);
        let typing = f.typing;
        let now = f.now;
        let generation_before;
        let mut hits: Vec<(ElementId, bool)> = Vec::new();
        let local_rider;
        {
            let mut w = self.shared.lock();
            generation_before = w.game.markers.generation;
            let level = f.level.as_ref().map(|l| l.id);
            if w.game.markers.level != level {
                let old: Vec<ElementId> = w
                    .game
                    .markers
                    .by_element
                    .keys()
                    .copied()
                    .filter(|e| w.tree.get(*e).is_none_or(|el| el.creator != Creator::Server))
                    .collect();
                for e in old {
                    ops::destroy(&mut w, Side::Client, e);
                }
                w.game.markers.level = level;
                w.game.markers.touch();
                w.game.last_position = None;
                if level.is_some() {
                    let own = f.level_markers.as_deref().map_or(&[][..], Vec::as_slice);
                    for lm in own.iter().take(super::MAX_LEVEL_MARKERS) {
                        let e =
                            w.tree.create("marker", ROOT, Creator::Host, None, true, self.shared.limits.tree_depth)?;
                        w.tree.set_name(e, level_marker_id(&lm.node));
                        w.game.markers.insert(MarkerView::of_level(e, lm));
                    }
                }
            }
            let session = w.local_player.is_some();
            let keys_now: Vec<u64> = f.riders.iter().map(|r| r.key).filter(|k| !(session && is_session(*k))).collect();
            let gone: Vec<(u64, ElementId)> =
                w.game.made.iter().filter(|(k, _)| !keys_now.contains(k)).map(|(k, e)| (*k, *e)).collect();
            for (k, e) in gone {
                w.game.made.remove(&k);
                ops::destroy(&mut w, Side::Client, e);
            }
            let depth = self.shared.limits.tree_depth;
            let mut resolved = std::collections::BTreeMap::new();
            for r in &f.riders {
                if session && is_session(r.key) {
                    if let Some(e) = session_rider(&w, r.key) {
                        resolved.insert(r.key, e);
                    }
                    continue;
                }
                let e = match w.game.made.get(&r.key) {
                    Some(e) => *e,
                    None => {
                        let e = w.tree.create("rider", ROOT, Creator::Host, None, true, depth)?;
                        w.game.made.insert(r.key, e);
                        e
                    }
                };
                if w.tree.get(e).is_some_and(|el| el.name != r.name) {
                    w.tree.set_name(e, &r.name);
                }
                resolved.insert(r.key, e);
            }
            w.game.riders = resolved;
            let here = ScriptHost::local_instance(&w);
            let mut moved = false;
            for m in w.game.markers.by_element.values_mut() {
                let elsewhere = m.instance.is_some_and(|i| Some(i) != here);
                moved |= m.elsewhere != elsewhere;
                m.elsewhere = elsewhere;
            }
            if moved {
                w.game.markers.touch();
            }
            let local = f.riders.iter().find(|r| r.local.is_some()).map(|r| (r.key, r.position));
            local_rider = local.and_then(|(k, _)| w.game.riders.get(&k).copied());
            if let Some((_, p1)) = local {
                if let Some(p0) = w.game.last_position {
                    let mut sounds = Vec::new();
                    for m in w.game.markers.by_element.values_mut().filter(|m| m.visible && !m.elsewhere) {
                        if let Some(forward) = hit(m, p0, p1) {
                            hits.push((m.element, forward));
                            if let Some(id) = &m.sound {
                                let order = SoundOrder {
                                    package: m.package.clone(),
                                    id: id.clone(),
                                    position: Some(m.position),
                                };
                                sounds.push(order);
                            }
                        }
                    }
                    w.game.orders.sounds.extend(sounds);
                }
                w.game.last_position = Some(p1);
            } else {
                w.game.last_position = None;
            }
            w.game.frame = f;
            w.game.bound = Some(self.shared.limits.game.frame_bound);
        }
        if !typing {
            for k in &keys {
                let state = if k.down { "down" } else { "up" };
                let binds: Vec<_> = {
                    let w = self.shared.lock();
                    w.game
                        .binds
                        .iter()
                        .filter(|b| b.key == k.key && (b.state == "both" || b.state == state))
                        .map(|b| (b.package.clone(), b.callback.clone()))
                        .collect()
                };
                for (package, cb) in binds {
                    if self.over_frame_bound() {
                        break;
                    }
                    let during = format!("the bind of {} ({})", k.key, cb.at);
                    let args = [s(k.key), s(state)];
                    let _ = enter(&self.shared, &package, &during, Errors::Note, |vm| {
                        call_function(vm, &cb.key, None, &args)
                    });
                }
                self.raise_engine("onClientKey", ROOT, &[s(k.key), NetValue::Bool(k.down)]);
            }
        }
        if let Some(rider) = local_rider {
            for e in &events {
                match e {
                    RideEvent::Land { air_time, bail } => self.raise_engine(
                        "onClientRiderLand",
                        rider,
                        &[NetValue::Number(f64::from(*air_time)), NetValue::Bool(*bail)],
                    ),
                    RideEvent::Trick { name, points } => {
                        self.raise_engine("onClientTrick", rider, &[s(name), NetValue::Number(*points)])
                    }
                }
            }
            for (marker, forward) in &hits {
                self.raise_engine("onClientMarkerHit", *marker, &[NetValue::Element(rider), NetValue::Bool(*forward)]);
            }
        }
        self.shared.lock().game.drawing = Some(DrawList::default());
        self.raise_engine("onClientRender", ROOT, &[]);
        let drawn = self
            .shared
            .lock()
            .game
            .drawing
            .take()
            .map(|d| {
                let (top, mut rest): (Vec<_>, Vec<_>) = d.items.into_iter().partition(|(p, _)| d.on_top.contains(p));
                rest.extend(top);
                rest
            })
            .unwrap_or_default();
        self.shared.lock().game.bound = None;
        self.tick(now);
        let mut w = self.shared.lock();
        let mut orders = std::mem::take(&mut w.game.orders);
        orders.draws = Some(drawn);
        let held: Vec<String> = w.game.capture.iter().filter(|p| w.running(p).is_none()).cloned().collect();
        for p in held {
            w.game.capture.remove(&p);
        }
        orders.capture = !w.game.capture.is_empty();
        let markers = (w.game.markers.generation != generation_before).then(|| w.game.markers.list());
        Ok(FrameReport { orders, markers, skipped: w.game.skipped })
    }

    fn over_frame_bound(&self) -> bool {
        let w = self.shared.lock();
        w.game.bound.is_some_and(|b| w.calls.is_empty() && w.side_tick_time >= b)
    }

    pub(crate) fn raise_engine(&self, name: &'static str, source: ElementId, args: &[NetValue]) {
        let d = Dispatch { name, source, args, only: None, client: None };
        let _ = dispatch(&self.shared, &d);
        self.settle();
    }
}
