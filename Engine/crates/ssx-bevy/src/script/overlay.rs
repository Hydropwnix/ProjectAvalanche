use bevy::prelude::*;
use bevy::text::FontSize;

use super::ScriptBridge;

const MAX_LINES: usize = 48;
const MAX_LINE: usize = 480;
const WIDTH: usize = 150;

#[derive(Resource)]
pub(super) struct Overlay {
    pub shown: bool,
}

#[derive(Component)]
pub(super) struct OverlayText;

pub(super) fn spawn(mut commands: Commands, overlay: Res<Overlay>) {
    let vis = if overlay.shown { Visibility::Inherited } else { Visibility::Hidden };
    commands.spawn((
        OverlayText,
        Text::default(),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextLayout::no_wrap(),
        TextColor(Color::srgb(0.85, 1.0, 0.85)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.8)),
        Node {
            position_type: PositionType::Absolute,
            top: percent(24),
            left: px(10),
            padding: UiRect::all(px(6)),
            ..default()
        },
        GlobalZIndex(i32::MAX),
        Pickable::IGNORE,
        vis,
    ));
}

fn ascii(s: &str) -> String {
    s.chars()
        .take(MAX_LINE)
        .map(|c| match c {
            'µ' => 'u',
            c if c.is_ascii_graphic() || c == ' ' => c,
            _ => '?',
        })
        .collect()
}

fn shown(line: &str, out: &mut Vec<String>) {
    let line = ascii(line);
    let indent = line.len() - line.trim_start().len() + 4;
    let mut current = String::new();
    for (i, group) in line.split(" | ").enumerate() {
        let piece = if i == 0 { group.to_string() } else { format!("| {group}") };
        if !current.is_empty() && current.len() + 1 + piece.len() > WIDTH {
            out.push(std::mem::take(&mut current));
        }
        if current.is_empty() && i > 0 {
            current = " ".repeat(indent);
        } else if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(&piece);
        while current.len() > WIDTH {
            let cut = current[..WIDTH].rfind(' ').filter(|&c| c > indent).unwrap_or(WIDTH);
            let rest = current[cut..].trim_start().to_string();
            current.truncate(cut);
            out.push(std::mem::take(&mut current));
            current = format!("{}{rest}", " ".repeat(indent));
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
}

pub(super) fn update(
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: ResMut<Overlay>,
    bridge: Res<ScriptBridge>,
    layer: Res<crate::ui::UiOverlay>,
    mut q: Query<(&mut Text, &mut Visibility), With<OverlayText>>,
    mut last: Local<String>,
) {
    if keys.just_pressed(KeyCode::F9) {
        overlay.shown = !overlay.shown;
    }
    let Ok((mut text, mut vis)) = q.single_mut() else { return };
    let want = if overlay.shown { Visibility::Inherited } else { Visibility::Hidden };
    if *vis != want {
        *vis = want;
    }
    if !overlay.shown {
        return;
    }
    let mut lines = vec![format!(
        "SCRIPTS (F9)  handlers left for a later frame by the frame bound: {}; drawn: {} ({} images), {:.3} ms of the \
         UI's CPU",
        bridge.skipped,
        layer.draws.len(),
        layer.images.len(),
        layer.cpu.as_secs_f64() * 1000.0
    )];
    if let Some(why) = &bridge.suspended {
        lines.push(format!("client scripts off for this session: {why}"));
    }
    for line in bridge.monitor.lines() {
        shown(line, &mut lines);
    }
    if lines.len() > MAX_LINES {
        let cut = lines.len() - MAX_LINES;
        lines.drain(1..=cut);
    }
    let joined = lines.join("\n");
    if *last != joined {
        text.0.clone_from(&joined);
        *last = joined;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_broken_not_cut() {
        let line = format!("  pkg {} | heap 1 KiB | el 3 data 0/0 B | err 0 ref 0 over 0", "x".repeat(140));
        let mut out = Vec::new();
        shown(&line, &mut out);
        assert!(out.iter().all(|l| l.len() <= WIDTH), "{out:?}");
        assert!(out.len() >= 2 && out[1].starts_with("      | heap"), "{out:?}");
        let joined: String = out.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(joined.contains("err 0 ref 0 over 0"), "{joined}");
        let mut out = Vec::new();
        shown("budget of 2000 µs, \u{7} and \u{202e}", &mut out);
        assert_eq!(out, ["budget of 2000 us, ? and ?"]);
        let mut out = Vec::new();
        shown(&"y".repeat(400), &mut out);
        assert!(out.len() == 3 && out.iter().all(|l| l.len() <= WIDTH), "{out:?}");
    }
}
