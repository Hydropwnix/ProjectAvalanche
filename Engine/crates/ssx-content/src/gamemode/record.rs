use super::EventType;
use crate::id::ContentId;
use crate::profile_state::{MAX_FIELDS, ModState, StateValue};
use ssx_rules::gamemode::{MAX_COUNT, Record, RunOutcome};

const SLOT_FIELDS: [&str; 6] = ["", "best", "attempts", "deaths", "rewinds", "sim"];
pub const MAX_SLOTS: usize = MAX_FIELDS / SLOT_FIELDS.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    Full,
    Refused,
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordError::Full => write!(f, "the map's records hold {MAX_SLOTS} other modes already"),
            RecordError::Refused => write!(f, "the profile's state of mod content is at its caps"),
        }
    }
}

fn field(k: usize, what: &str) -> String {
    if what.is_empty() { format!("m{k}") } else { format!("m{k}_{what}") }
}

fn slot_of(st: &ModState, map: &ContentId, mode: &str) -> Option<usize> {
    (1..=MAX_SLOTS).find(|&k| st.text(map, &field(k, "")) == Some(mode))
}

pub fn read(st: &ModState, map: &ContentId, mode: &str) -> Option<Record> {
    let k = slot_of(st, map, mode)?;
    let value = |w: &str| st.int(map, &field(k, w)).and_then(|v| u32::try_from(v).ok());
    let count = |w: &str| value(w).map_or(0, |v| v.min(MAX_COUNT));
    Some(Record {
        best: value("best"),
        attempts: count("attempts"),
        deaths: count("deaths"),
        fewest_rewinds: value("rewinds"),
    })
}

pub fn best_sim(st: &ModState, map: &ContentId, mode: &str) -> Option<String> {
    let k = slot_of(st, map, mode)?;
    st.text(map, &field(k, "sim")).map(str::to_string)
}

pub fn add_run(
    st: &mut ModState,
    map: &ContentId,
    mode: &str,
    t: EventType,
    run: RunOutcome,
    sim: Option<&str>,
) -> Result<bool, RecordError> {
    let k = match slot_of(st, map, mode) {
        Some(k) => k,
        None => (1..=MAX_SLOTS).find(|&k| st.get(map, &field(k, "")).is_none()).ok_or(RecordError::Full)?,
    };
    let mut r = read(st, map, mode).unwrap_or_default();
    let moved = r.add(t, run);
    let mut next = ModState::default();
    if let Some(e) = st.entries.get(map) {
        next.entries.insert(map.clone(), e.clone());
    } else if st.entries.len() >= crate::profile_state::MAX_ENTRIES {
        return Err(RecordError::Refused);
    }
    let int = |v: u32| StateValue::Int(i64::from(v));
    let mut ok = next.set(map, &field(k, ""), StateValue::Text(mode.to_string()));
    ok &= next.set(map, &field(k, "attempts"), int(r.attempts));
    ok &= next.set(map, &field(k, "deaths"), int(r.deaths));
    if let Some(b) = r.best {
        ok &= next.set(map, &field(k, "best"), int(b));
    }
    if let Some(w) = r.fewest_rewinds {
        ok &= next.set(map, &field(k, "rewinds"), int(w));
    }
    if moved {
        match sim {
            Some(s) => ok &= next.set(map, &field(k, "sim"), StateValue::Text(s.to_string())),
            None => {
                if let Some(e) = next.entries.get_mut(map) {
                    e.remove(&field(k, "sim"));
                }
            }
        }
    }
    if !ok {
        return Err(RecordError::Refused);
    }
    let entry = next.entries.remove(map).ok_or(RecordError::Refused)?;
    st.entries.insert(map.clone(), entry);
    Ok(moved)
}
