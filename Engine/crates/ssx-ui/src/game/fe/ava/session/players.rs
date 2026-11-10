use super::{PlayerStatus, SessionPlayer};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::ava::browser::rows::Window;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;
use ssx_net_proto::{Mode, StandingState};

pub const DP: &str = "dp_ava_players";
pub const DP_ROWS: &str = "dp_ava_players_rows";
pub const DP_ROWS_INDEX: &str = "dp_ava_players_rows_index";
pub const DP_WINDOW: &str = "dp_ava_players_window";
pub const OPEN: &str = "go_to_ava_players";
pub const BACK: &str = "go_to_pause";
pub const RESULTS_OPEN: &str = "ava_results_players";
pub const RESULTS_CLOSE: &str = "ava_results_players_close";

#[derive(Default)]
pub struct Players {
    pub active: bool,
    pub window: Window,
    pub dirty: bool,
    pub over_results: bool,
}

pub fn open_over_results(fe: &mut FrontEnd, env: &mut FeEnv) {
    if fe.ava.session.list.over_results {
        return;
    }
    fe.note("results: a round's A opens the Players screen over them");
    fe.ava.session.list.over_results = true;
    env.nav_events.push(RESULTS_OPEN.into());
}

pub fn close_over_results(fe: &mut FrontEnd, env: &mut FeEnv) {
    if std::mem::take(&mut fe.ava.session.list.over_results) {
        fe.note("results: the Players screen over them closed");
        env.nav_events.push(RESULTS_CLOSE.into());
    }
}

fn status_key(s: Option<PlayerStatus>) -> &'static str {
    match s {
        None | Some(PlayerStatus::Joining) => "TXT_AVA_PLAYERS_JOINING",
        Some(PlayerStatus::Loading) => "TXT_AVA_PLAYERS_LOADING",
        Some(PlayerStatus::Intro) => "TXT_AVA_PLAYERS_INTRO",
        Some(PlayerStatus::Riding) => "TXT_AVA_PLAYERS_RIDING",
        Some(PlayerStatus::Away) => "TXT_AVA_PLAYERS_AWAY",
        Some(PlayerStatus::Spectating) => "TXT_AVA_PLAYERS_SPECTATING",
        Some(PlayerStatus::Finished) => "TXT_AVA_PLAYERS_FINISHED",
    }
}

fn result_text(loc: &mut Localization, mode: Option<Mode>, p: &SessionPlayer) -> String {
    let (Some(mode), Some(row)) = (mode, p.standing.as_ref()) else { return "-".into() };
    match row.state {
        StandingState::Failed => return loc.text("TXT_AVA_RESULTS_FAILED"),
        StandingState::Left => return loc.text("TXT_AVA_RESULTS_LEFT"),
        StandingState::Invalidated => {
            let why = row.invalid.map_or_else(String::new, |c| loc.text(super::results::check_key(c)));
            return loc.format("TXT_AVA_RESULTS_INVALID", &[&why]);
        }
        StandingState::Finished | StandingState::Racing => {}
    }
    match mode {
        Mode::Race | Mode::Freeride => match (row.state, row.split) {
            (StandingState::Finished, _) => loc.race_time(row.clock_ms),
            (_, Some(k)) => {
                loc.format("TXT_AVA_PLAYERS_SPLIT", &[&(u16::from(k) + 1).to_string(), &loc.race_time(row.clock_ms)])
            }
            (_, None) => "-".into(),
        },
        Mode::Trick => loc.int(i64::from(row.points)),
        Mode::Survive => loc.distance_with_unit(f64::from(row.descent_mm) / 1000.0),
    }
}

fn note_key(mode: Option<Mode>) -> &'static str {
    match mode {
        None | Some(Mode::Freeride) => "TXT_AVA_PLAYERS_FREERIDE",
        Some(Mode::Race) => "TXT_AVA_PLAYERS_RACE",
        Some(Mode::Trick) => "TXT_AVA_PLAYERS_TRICK",
        Some(Mode::Survive) => "TXT_AVA_PLAYERS_SURVIVE",
    }
}

fn selected(fe: &FrontEnd) -> Option<&SessionPlayer> {
    fe.ava.session.players.get(fe.ava.session.list.window.selected())
}

pub fn activate(fe: &mut FrontEnd) {
    let list = &mut fe.ava.session.list;
    list.active = true;
    list.window = Window { first: 0, focus: 0, ..list.window };
    list.dirty = true;
    fe.note(format!("players: open ({} players)", fe.ava.session.players.len()));
}

pub fn deactivate(fe: &mut FrontEnd) {
    fe.ava.session.list.active = false;
    fe.ava.session.list.over_results = false;
}

pub fn update(fe: &mut FrontEnd) {
    let s = &mut fe.ava.session;
    if !s.list.active || !std::mem::take(&mut s.list.dirty) {
        return;
    }
    s.list.window.clamp(s.players.len());
    fe.publish.extend([DP.into(), DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_WINDOW.into()]);
}

pub fn action(fe: &mut FrontEnd, env: &mut FeEnv, action: &str, param: &ActionParam) {
    if action == "A_AvaPlayersBack" {
        if fe.ava.session.list.over_results {
            close_over_results(fe, env);
        } else {
            env.nav_events.push(BACK.into());
        }
        return;
    }
    let len = fe.ava.session.players.len();
    let w = &mut fe.ava.session.list.window;
    let moved = match action {
        "A_AvaPlayersSetup" => {
            let (rows, band) = (param.field("maxData"), param.field("unselectableData"));
            if rows.is_finite() && (1.0..=32.0).contains(&rows) && band.is_finite() && band >= 0.0 {
                w.rows = rows as usize;
                w.band = (band as usize).min(w.rows.saturating_sub(1));
            }
            w.clamp(len);
            true
        }
        "A_AvaPlayersRequestData" if param.number.is_finite() => w.step(param.number.signum() as i64, len),
        "A_AvaPlayersPage" if param.number.is_finite() => w.page(param.number.signum() as i64, len),
        "A_AvaPlayersMute" => {
            if let Some(p) = selected(fe).cloned() {
                if fe.ava.session.toggle_mute(p.key_id) {
                    let now = if fe.ava.session.muted(&p.key_id) { "muted" } else { "unmuted" };
                    fe.note(format!("players: {} {now}", p.name));
                } else {
                    fe.note("players: this player's own row is not muted");
                }
            }
            false
        }
        _ => false,
    };
    if moved {
        fe.publish.extend([DP_ROWS.into(), DP_ROWS_INDEX.into(), DP_WINDOW.into()]);
    }
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx, loc: &mut Localization, dp: &str) -> Option<Value> {
    let s = &fe.ava.session;
    match dp {
        DP => {
            let o = Value::Object(cx.new_object());
            let count = match s.max_players {
                Some(max) => loc.format("TXT_AVA_PLAYERS_COUNT_OF", &[&s.players.len().to_string(), &max.to_string()]),
                None => loc.format("TXT_AVA_PLAYERS_COUNT", &[&s.players.len().to_string()]),
            };
            let server = crate::game::fe::ava::shown(&s.server, fe.ava.glyphs.as_ref());
            for (field, text) in [
                ("title", loc.text("TXT_AVA_PLAYERS_TITLE")),
                ("server", server),
                ("count", count),
                ("colPlace", loc.text("TXT_AVA_PLAYERS_COL_PLACE")),
                ("colName", loc.text("TXT_AVA_PLAYERS_COL_NAME")),
                ("colRider", loc.text("TXT_AVA_PLAYERS_COL_RIDER")),
                ("colStatus", loc.text("TXT_AVA_PLAYERS_COL_STATUS")),
                (
                    "colResult",
                    loc.text(if s.rounds == Some(Mode::Race) {
                        "TXT_AVA_PLAYERS_COL_TIME"
                    } else {
                        "TXT_AVA_PLAYERS_COL_SCORE"
                    }),
                ),
                ("colPing", loc.text("TXT_AVA_PLAYERS_COL_PING")),
                ("colChat", loc.text("TXT_AVA_PLAYERS_COL_CHAT")),
                ("note", loc.text(note_key(s.rounds))),
                ("pingHelp", loc.text("TXT_AVA_PLAYERS_PING_HELP")),
            ] {
                let _ = cx.set_member(&o, field, Value::str(&text));
            }
            Some(o)
        }
        DP_ROWS => {
            let w = &s.list.window;
            let mut rows = Vec::new();
            for p in s.players.iter().skip(w.first).take(w.rows) {
                let rider = p.character.and_then(|k| fe.career.riders.iter().find(|r| r.key == k));
                let row = Value::Object(cx.new_object());
                let name = crate::game::fe::ava::bounded(
                    &p.name,
                    ssx_net_proto::name::MAX_PLAYER_NAME_CHARS,
                    fe.ava.glyphs.as_ref(),
                );
                let ping = p.ping_ms.map_or_else(|| "-".to_string(), |ms| format!("{ms} ms"));
                let chat = if p.own {
                    loc.text("TXT_AVA_PLAYERS_YOU")
                } else if s.muted(&p.key_id) {
                    loc.text("TXT_AVA_PLAYERS_MUTED")
                } else {
                    String::new()
                };
                let place = p.place.map_or_else(String::new, |k| k.to_string());
                let result = result_text(loc, s.rounds, p);
                for (field, text) in [
                    ("strPlace", place),
                    ("strResult", result),
                    ("strName", name),
                    ("strRider", rider.map(|r| loc.text(&r.title)).unwrap_or_default()),
                    ("strStatus", loc.text(status_key(p.status))),
                    ("strPing", ping),
                    ("strChat", chat),
                    ("strIcon", rider.map(|r| r.icon.clone()).unwrap_or_default()),
                ] {
                    let _ = cx.set_member(&row, field, Value::str(&text));
                }
                let _ = cx.set_member(&row, "bEnabled", Value::Bool(true));
                rows.push(row);
            }
            Some(Value::Object(cx.new_array(rows)))
        }
        DP_ROWS_INDEX => Some(Value::Number(s.list.window.focus as f64)),
        DP_WINDOW => Some(crate::game::fe::ava::window_object(cx, s.players.len(), s.list.window.first)),
        _ => None,
    }
}
