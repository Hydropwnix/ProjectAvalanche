use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use ssx_assets::port_options::DiscordPresence;
use ssx_discord::{Activity, ActivityType, AssetKey, Button, Client, Config, Text, Timing};
use ssx_ui::game::GameUi;
use ssx_ui::game::presence::{self as words, Doing, Lines, Menu, Part, Ride, RideNames, Round, Timer};

use super::{GameUiRes, Shell, Stage};
use crate::shell::game_level::LevelGame;
use crate::shell::live::LiveSession;

// PORT(choice): the presence names the Discord application "Project Avalanche" by its id, shows its art asset `logo`
// (the project's own logo, `Engine/assets/branding/project-avalanche-logo-1024.png`, uploaded in the Developer
// Portal) and two buttons, the releases and the community server (the user's decisions, 2026-11-15). The console had
// no Discord.
pub const APPLICATION_ID: &str = "1571572936433995906";
pub const LARGE_IMAGE: &str = "logo";
pub const LARGE_TEXT: &str = concat!("Project Avalanche ", env!("CARGO_PKG_VERSION"));
pub const BUTTONS: [(&str, &str); 2] = [
    ("Get Project Avalanche", "https://github.com/MnkyArts/ProjectAvalanche/releases"),
    ("Join the Discord", "https://discord.com/invite/XrFGQEfwdJ"),
];

#[derive(Resource)]
struct Presence {
    client: Client,
    doing: Option<Doing>,
    game_start: Option<u64>,
    run: Option<((u32, u32), Option<u64>)>,
}

pub(crate) fn client(choice: DiscordPresence, config: Config) -> Option<std::io::Result<Client>> {
    match choice {
        DiscordPresence::Off => None,
        DiscordPresence::On => Some(Client::start(config)),
    }
}

pub(super) fn register(app: &mut App, choice: DiscordPresence) {
    let config = Config {
        client_id: APPLICATION_ID.into(),
        endpoints: ssx_discord::transport::endpoints(),
        timing: Timing::default(),
    };
    match client(choice, config) {
        None => info!("presence: Discord Rich Presence is off (no connection to Discord)"),
        Some(Err(e)) => warn!("presence: Discord Rich Presence could not start its thread: {e}"),
        Some(Ok(client)) => {
            info!("presence: Discord Rich Presence is on (the Discord app on this computer, when it runs)");
            app.insert_resource(Presence { client, doing: None, game_start: unix_now(), run: None });
            app.add_systems(Update, update.after(super::frame::shell_step));
        }
    }
}

fn unix_now() -> Option<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

#[cfg(feature = "net")]
fn round(live: Option<&LiveSession>, rounds: Option<&crate::net::Rounds>) -> Option<Round> {
    use ssx_net_proto::InstanceState;
    live?;
    let mode = rounds.and_then(|r| r.mode).map(ssx_net_proto::Mode::name);
    let state = rounds.and_then(|r| r.state);
    Some(Round { mode, competing: matches!(state, Some(InstanceState::Countdown | InstanceState::Running)) })
}

#[cfg(not(feature = "net"))]
fn round(live: Option<&LiveSession>) -> Option<Round> {
    live.map(|_| Round { mode: None, competing: false })
}

fn observe(shell: &Shell, ui: &GameUi, level: Option<&LevelGame>, online: Option<Round>) -> Doing {
    if shell.stage != Stage::Menus {
        let up = shell.stage == Stage::Level && level.is_some_and(LevelGame::is_up);
        let part = match level {
            _ if !up => Part::Loading,
            Some(l) if l.results_shown() => Part::Results,
            _ => Part::Riding,
        };
        let run = level.map_or(0, LevelGame::restarts);
        return Doing::Ride(Ride { launch: shell.rides, run, part, online });
    }
    let Some(fe) = ui.fe.as_ref().filter(|fe| !words::starting(fe.screen)) else { return Doing::Starting };
    let menu = words::menu(fe.screen, fe.game_mode, &|n| ui.nav.is_active(n));
    match (online, menu) {
        (None, m) => Doing::Menu(m),
        (Some(_), Menu::Equipment(_)) => Doing::Menu(Menu::OnlineEquipment),
        (Some(_), _) => Doing::Menu(Menu::Joining),
    }
}

fn activity(lines: &Lines, start: Option<u64>) -> Activity {
    let mut a = Activity::new(if lines.competing { ActivityType::Competing } else { ActivityType::Playing });
    a.details = Text::new(&lines.details);
    a.state = lines.state.as_deref().and_then(Text::new);
    a.start = start;
    a.large_image = AssetKey::new(LARGE_IMAGE);
    a.large_text = Text::new(LARGE_TEXT);
    for b in BUTTONS.into_iter().filter_map(|(label, url)| Button::new(label, url)) {
        a.push_button(b);
    }
    a
}

fn update(
    shell: Res<Shell>,
    mut ui: ResMut<GameUiRes>,
    level: Option<Res<LevelGame>>,
    live: Option<Res<LiveSession>>,
    #[cfg(feature = "net")] rounds: Option<Res<crate::net::Rounds>>,
    mut presence: ResMut<Presence>,
) {
    #[cfg(feature = "net")]
    let online = round(live.as_deref(), rounds.as_deref());
    #[cfg(not(feature = "net"))]
    let online = round(live.as_deref());
    let doing = observe(&shell, &ui.0, level.as_deref(), online);
    if presence.doing == Some(doing) {
        return;
    }
    presence.doing = Some(doing);
    let ui = &mut ui.0;
    let (names, outcome) = match (doing, shell.launch.as_ref()) {
        (Doing::Ride(r), Some(launch)) => {
            let names = words::ride_names(ui.fe.as_ref(), &mut ui.loc, launch);
            let result = level.as_deref().and_then(LevelGame::result).filter(|_| r.part == Part::Results);
            (names, result.map(|res| words::outcome(&mut ui.loc, launch.event_type, res)))
        }
        _ => (RideNames::default(), None),
    };
    let lines = words::lines(&doing, &names, outcome.as_deref(), &mut ui.loc);
    // PORT(choice): the timer counts a run from the frame its level is drawn (a restart starts it over, a pause does
    // not stop it: Discord's timer cannot pause), and the game's time everywhere else. The console had no Discord.
    let start = match (lines.timer, doing) {
        (Timer::Run, Doing::Ride(r)) => {
            let key = (r.launch, r.run);
            if presence.run.is_none_or(|(k, _)| k != key) {
                presence.run = Some((key, unix_now()));
            }
            presence.run.and_then(|(_, at)| at)
        }
        _ => presence.game_start,
    };
    debug!("presence: {doing:?}: {lines:?}");
    presence.client.set(activity(&lines, start));
}

#[cfg(test)]
mod tests;
