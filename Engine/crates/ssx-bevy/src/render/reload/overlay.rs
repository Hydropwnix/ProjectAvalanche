use std::sync::Arc;
use std::time::SystemTime;

use bevy::log::{info, warn};
use bevy::prelude::{
    BackgroundColor, Color, Commands, Component, Local, Node, PositionType, Query, Res, Resource, Text, TextColor,
    TextFont, UiRect, With, default, px,
};
use bevy::text::FontSize;
use ssx_assets::content::dev::DevWatch;
use ssx_content::Layer;

use super::ReloadState;

#[derive(Resource)]
pub struct DevOverlay(pub Arc<DevWatch>);

#[derive(Component)]
pub(super) struct DevOverlayText;

type OverlayKey = (u64, u64, usize, Option<u64>, u64);

const MAX_CONFLICTS: usize = 6;

pub(super) fn spawn(mut commands: Commands, overlay: Res<DevOverlay>) {
    let s = overlay.0.status();
    match (&s.name, s.broken.is_empty()) {
        (Some(name), true) => info!("mod-dev: {name} mounted from {}, watched", s.root.display()),
        _ => warn!("mod-dev: {}: not mounted yet, watched: {}", s.root.display(), s.broken.join("; ")),
    }
    commands.spawn((
        DevOverlayText,
        Text::default(),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(10),
            right: px(10),
            max_width: px(720),
            padding: UiRect::all(px(8)),
            ..default()
        },
    ));
}

fn lines(overlay: &DevOverlay, state: &ReloadState) -> String {
    let s = overlay.0.status();
    let mut out = Vec::new();
    let name = match (&s.name, &s.version) {
        (Some(n), Some(v)) => format!("{n} {v}"),
        _ => "(not mounted yet)".to_string(),
    };
    out.push(format!("MOD-DEV  {name}  ({})", s.root.display()));
    let ago =
        s.at.and_then(|t| SystemTime::now().duration_since(t).ok())
            .map_or(String::new(), |d| format!(" {} s ago", d.as_secs()));
    if s.broken.is_empty() {
        let ids = if s.changed == 1 { "id" } else { "ids" };
        out.push(format!("reload {}:{ago} {} {ids} changed in {:.2} s", s.reloads, s.changed, s.seconds));
    } else {
        out.push(format!("EDIT REFUSED{ago}, the last good state stays:"));
        out.extend(s.broken.iter().take(4).map(|l| format!("  {}", l.chars().take(110).collect::<String>())));
    }
    state.content.with_registry(|reg| {
        let mods: Vec<String> = reg
            .mounts()
            .filter(|m| m.layer == Layer::ClientMods)
            .map(|m| if s.name.as_deref() == Some(m.name) { format!("{}*", m.name) } else { m.name.to_string() })
            .collect();
        out.push(format!("mods (weakest first, * in development): {}", mods.join(", ")));
        let report = reg.report();
        let mut per: std::collections::BTreeMap<&str, usize> = Default::default();
        for o in report.overrides.iter().filter(|o| o.winner.0 == Layer::ClientMods) {
            *per.entry(o.winner.1.as_str()).or_default() += 1;
        }
        let list: Vec<String> = per.iter().map(|(n, c)| format!("{n} {c}")).collect();
        out.push(format!("overrides of the game: {}", if list.is_empty() { "none".into() } else { list.join(", ") }));
        out.push(format!("conflicts: {}", report.conflicts.len()));
        for c in report.conflicts.iter().take(MAX_CONFLICTS) {
            out.push(format!("  {}: {} wins over {}", c.id, c.winner, c.losers.join(", ")));
        }
    });
    let hooks = crate::render::hooks::runtime::summary();
    if !hooks.is_empty() {
        out.push(hooks);
    }
    for d in state.content.decisions() {
        let refused = d.outcomes.iter().filter(|o| o.refused.is_some()).count();
        if refused > 0 {
            out.push(format!(
                "on {}: {refused} of {} package objects on the disc's (see the log)",
                d.track,
                d.outcomes.len()
            ));
        }
    }
    out.extend(state.content.audio_overlay());
    out.extend(state.content.rider_overlay());
    out.join("\n")
}

pub(super) fn update(
    overlay: Option<Res<DevOverlay>>,
    state: Res<ReloadState>,
    mut last: Local<Option<OverlayKey>>,
    mut texts: Query<&mut Text, With<DevOverlayText>>,
) {
    let Some(overlay) = overlay else { return };
    let s = overlay.0.status();
    let ago = s.at.and_then(|t| SystemTime::now().duration_since(t).ok()).map(|d| d.as_secs());
    let key = (
        state.content.generation() ^ crate::render::hooks::runtime::settled() << 32,
        s.reloads,
        s.broken.len(),
        ago,
        state.content.rider_decisions(),
    );
    if last.as_ref() == Some(&key) {
        return;
    }
    if !s.broken.is_empty() && last.as_ref().is_none_or(|l| (l.1, l.2) != (key.1, key.2) || l.3 > key.3) {
        warn!("mod-dev: an edit was refused, the last good state stays: {}", s.broken.join("; "));
    }
    if s.broken.is_empty() && last.as_ref().is_some_and(|l| l.1 != key.1) {
        let ids = if s.changed == 1 { "id" } else { "ids" };
        info!("mod-dev: reload {}: {} {ids} changed in {:.2} s", s.reloads, s.changed, s.seconds);
    }
    *last = Some(key);
    let text = lines(&overlay, &state);
    for mut t in &mut texts {
        if t.0 != text {
            t.0.clone_from(&text);
        }
    }
}
