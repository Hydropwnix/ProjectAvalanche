use super::{MapRow, ModeInfo};
use crate::game::fe::{FeEnv, FrontEnd, MapLaunch};

pub fn ride(fe: &mut FrontEnd, env: &mut FeEnv, id: &str, mode: &str, server_event_type: u8) -> Result<(), String> {
    let launch = map_launch(fe, id, mode, server_event_type)?;
    fe.note(format!("maps: the server's map {id} ({mode}: event type {server_event_type})"));
    crate::game::fe::flow::launch_map(fe, env, launch, i64::from(server_event_type));
    Ok(())
}

fn map_launch(fe: &FrontEnd, id: &str, mode: &str, server_event_type: u8) -> Result<MapLaunch, String> {
    let rows: &[MapRow] = &fe.ava.maps.rows;
    let row = rows.iter().find(|r| r.id == id).ok_or_else(|| format!("the server's map {id} is not installed"))?;
    let info = row.entry.as_ref().map_err(|e| format!("the server's map {id}: {e}"))?;
    let shown = ssx_core::text::quoted(mode);
    let m: &ModeInfo = info
        .modes
        .iter()
        .find(|m| m.choice == mode)
        .ok_or_else(|| format!("the server's map {id} runs the mode {shown}, which it does not offer here"))?;
    if let Some(why) = &m.refused {
        return Err(format!("the server's map {id} in the mode {shown}: {why}"));
    }
    if m.event_type != i64::from(server_event_type) {
        return Err(format!(
            "the server runs the map {id} as the event type {server_event_type}, its mode {shown} is {}",
            m.event_type
        ));
    }
    let track = match &info.base {
        None => String::new(),
        Some(drop) => {
            let e = fe
                .career
                .event_named(drop)
                .ok_or_else(|| format!("the server's map {id}: its base drop {drop} is not on this game's disc"))?;
            fe.career.drop(e.drop).map(|d| d.track.clone()).ok_or_else(|| format!("{id}: {drop} has no track"))?
        }
    };
    Ok(MapLaunch { id: id.to_string(), title: info.title.clone(), track, mode: m.choice.clone() })
}
