use ssx_content::{ContentId, Kind};

use super::markers::MarkerView;
use super::{HudOrder, SoundOrder};
use crate::api::WORLD;
use crate::host::ScriptHost;
use crate::host::elements::Creator;
use crate::host::outbox::MarkerSpec;
use crate::value::ElementId;

const SERVER: &str = "server";

fn finite3(p: [f32; 3]) -> bool {
    p.iter().all(|v| v.is_finite() && f64::from(v.abs()) <= WORLD)
}

fn content_id(s: &str, kind: Kind) -> Result<ContentId, String> {
    let id: ContentId = s.parse().map_err(|e| format!("{s:?}: {e}"))?;
    if id.kind() != kind {
        return Err(format!("{id} is not a {kind:?} id"));
    }
    Ok(id)
}

impl ScriptHost {
    pub fn local_player(&self) -> Option<ElementId> {
        self.shared.lock().local_player
    }

    pub fn leave_session(&self) {
        let mut w = self.shared.lock();
        let tops: Vec<ElementId> = w
            .tree
            .iter()
            .filter(|(_, e)| {
                e.creator == Creator::Server
                    && !e.parent.is_some_and(|p| w.tree.get(p).is_some_and(|pe| pe.creator == Creator::Server))
            })
            .map(|(id, _)| id)
            .collect();
        for id in tops {
            crate::host::ops::destroy(&mut w, crate::host::Side::Client, id);
        }
        w.local_player = None;
        w.game.session.clear();
        w.latent_in.drop_peer(crate::host::outbox::Peer::Server);
    }

    pub(crate) fn local_instance(w: &crate::host::world::World) -> Option<ElementId> {
        let player = w.local_player?;
        let rider = w.game.session.values().find(|(_, p)| *p == player).map(|(r, _)| *r)?;
        w.tree.get(rider).and_then(|e| e.parent)
    }

    pub(crate) fn mirror_marker(
        &self,
        package: Option<&str>,
        element: ElementId,
        spec: MarkerSpec,
    ) -> Result<(), String> {
        let id = content_id(&spec.id, Kind::Marker)?;
        if !finite3(spec.position)
            || !(spec.heading.is_finite() && spec.heading.abs() <= 360.0)
            || !(spec.size.is_finite() && (0.1..=100.0).contains(&spec.size))
        {
            return Err("a marker's place, heading or size beyond its range".into());
        }
        let caps = self.shared.limits.game;
        let mut w = self.shared.lock();
        match w.tree.get(element) {
            Some(e) if e.creator == Creator::Server && (e.kind == "marker" || e.kind == "checkpoint") => {}
            _ => return Err(format!("{element} is not a marker of the server's")),
        }
        if let Some(i) = spec.instance
            && !w.tree.get(i).is_some_and(|e| e.creator == Creator::Server && e.kind == "instance")
        {
            return Err(format!("{i} is not an instance of the server's"));
        }
        let meta = w.game.catalog.markers.get(&id).cloned().ok_or_else(|| format!("{id}: no source provides it"))?;
        let package = package.unwrap_or(SERVER).to_string();
        let inside = match w.game.markers.by_element.get(&element) {
            Some(m) => m.inside,
            None => {
                let server = w
                    .game
                    .markers
                    .by_element
                    .keys()
                    .filter(|e| w.tree.get(**e).is_some_and(|el| el.creator == Creator::Server))
                    .count();
                if server >= caps.server_markers {
                    return Err(format!("the server's markers: {server} drawn (at most {})", caps.server_markers));
                }
                let mine = w.game.markers.count_of(&package);
                if mine >= caps.markers_package {
                    return Err(format!("{package} has {mine} markers (at most {})", caps.markers_package));
                }
                if let Some(level) = &w.game.frame.level {
                    let (m, b) = (&w.game.markers, level.budget);
                    if m.scripts_count() >= b.markers
                        || m.vertices + meta.vertices > b.vertices
                        || m.triangles + meta.triangles > b.triangles
                    {
                        return Err(format!("{id} would pass the level's marker budget"));
                    }
                }
                false
            }
        };
        w.game.markers.insert(MarkerView {
            element,
            package,
            id,
            position: spec.position,
            heading: spec.heading,
            size: spec.size,
            colour: spec.colour,
            visible: spec.visible,
            hit: meta.hit,
            hit_scale: 1.0,
            sound: meta.sound,
            vertices: meta.vertices,
            triangles: meta.triangles,
            inside,
            instance: spec.instance,
            elsewhere: spec.instance.is_some(),
        });
        Ok(())
    }

    pub(crate) fn server_notice(
        &self,
        package: Option<&str>,
        title: String,
        text: String,
        seconds: f32,
    ) -> Result<(), String> {
        if title.len() > 256 || text.len() > 1024 || !(seconds.is_finite() && (0.5..=30.0).contains(&seconds)) {
            return Err("a notice beyond its sizes or time".into());
        }
        let package = package.unwrap_or(SERVER).to_string();
        let every = self.shared.limits.game.hud_every;
        let mut w = self.shared.lock();
        if w.game.frame.hud_busy {
            return Err("the HUD's message panel shows the game's message".into());
        }
        let now = w.now;
        if let Some(at) = w.game.hud_at.get(&package)
            && now.saturating_sub(*at) < every
        {
            return Err(format!("one HUD message every {} ms per package", every.as_millis()));
        }
        w.game.hud_at.insert(package.clone(), now);
        w.game.orders.hud.push(HudOrder { package, title, text, seconds });
        Ok(())
    }

    pub(crate) fn server_sound(
        &self,
        package: Option<&str>,
        id: &str,
        position: Option<[f32; 3]>,
    ) -> Result<(), String> {
        let id = content_id(id, Kind::Sound)?;
        if position.is_some_and(|p| !finite3(p)) {
            return Err("a sound's place beyond the world".into());
        }
        let package = package.unwrap_or(SERVER).to_string();
        let caps = self.shared.limits.game;
        let mut w = self.shared.lock();
        if !w.game.catalog.ids.contains_key(&id) {
            return Err(format!("{id}: no source provides it"));
        }
        let sounds = &w.game.orders.sounds;
        let mine = sounds.iter().filter(|s| s.package == package).count();
        if mine >= caps.sounds_frame_package || sounds.len() >= caps.sounds_frame {
            return Err(format!("{} sounds this frame (at most {})", sounds.len(), caps.sounds_frame));
        }
        w.game.orders.sounds.push(SoundOrder { package, id, position });
        Ok(())
    }
}
