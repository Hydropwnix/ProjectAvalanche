use super::*;

pub use ssx_rules::events::{
    CLASS_UI_DROP, EventType, FIELD_UI_DROP_AVALANCHE_SCRIPTS, FIELD_UI_DROP_EVENT_TYPE, FIELD_UI_DROP_FORCED_LOADOUT,
    FIELD_UI_DROP_GE_DROP, FIELD_UI_DROP_HUD_BODY, FIELD_UI_DROP_HUD_TITLE, FIELD_UI_DROP_MESSAGE_PARAMETER,
    FIELD_UI_DROP_NAME, FIELD_UI_DROP_REQ_EQUIPMENT, FIELD_UI_DROP_SCRIPTS, FIELD_UI_DROP_SLOT_COUNTS,
    FIELD_UI_DROP_TYPE, read_pair, shows_explore_ghosts, values,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rival {
    pub starting_idx: i32,
    pub ghost_id: i32,
    pub character: u32,
    pub catchup: u32,
    pub score: u32,
    pub tuned_score: u32,
    pub lap: u32,
    pub end_bits: [u32; 3],
}

impl Rival {
    pub fn value(&self) -> u32 {
        if self.tuned_score == u32::MAX { self.score } else { self.tuned_score }
    }

    pub fn end_position(&self) -> [f32; 3] {
        self.end_bits.map(f32::from_bits)
    }
}

pub fn parse_ghost_db(text: &str) -> Vec<(u32, Rival)> {
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        let num = |i: usize| f.get(i).and_then(|s| s.parse::<i64>().ok());
        let (Some(id), Some(start), Some(ghost), Some(score), Some(tuned), Some(ch), Some(cu)) =
            (num(0), num(1), num(2), num(3), num(4), num(5), num(6))
        else {
            continue;
        };
        let rival = Rival {
            starting_idx: start as i32,
            ghost_id: ghost as i32,
            character: ch as u32,
            catchup: cu as u32,
            score: score as u32,
            tuned_score: tuned as u32,
            lap: num(7).map_or(0, |v| v as u32),
            end_bits: [8, 9, 10].map(|i| num(i).map_or(0, |v| v as u32)),
        };
        out.push((id as u32, rival));
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct EventDef {
    pub name: String,
    pub key: u32,
    pub drop_key: u32,
    pub event_type: Option<EventType>,
    pub scripts: Vec<String>,
    pub rivals: Vec<Rival>,
    pub required_equipment: Vec<u32>,
    pub avalanche_scripts: Vec<String>,
    pub forced_rider: u32,
    pub forced_items: crate::gear::ForcedItems,
    pub loadout: Option<crate::character::GearStats>,
    pub objectives: Option<std::sync::Arc<crate::objectives::EventObjectives>>,
    pub message: crate::objectives::DropMessage,
    pub places: Option<i64>,
    pub goal: Option<i64>,
}

impl EventDef {
    pub(super) fn probe(&self) -> Option<&crate::objectives::Probe> {
        self.objectives.as_ref().map(|o| &o.probe)
    }

    pub fn delayed_finish(&self) -> bool {
        self.probe().is_some_and(|p| p.flag("DelayedFinish"))
    }

    pub fn death_permanent(&self) -> bool {
        self.probe().is_some_and(|p| p.set_true("Player1", "BLN_IsDeathPermanent"))
    }

    pub fn forces_wingsuit(&self) -> bool {
        self.probe().is_some_and(|p| p.set_true("Player1", "BLN_ForceDescentInAir"))
    }

    pub fn ghost_board(&self) -> Vec<ssx_rules::results::GhostEntry> {
        ssx_rules::results::ghost_board(self.rivals.iter().map(|r| (r.value(), r.character)), self.event_type)
    }

    pub fn ranked_rivals(&self) -> Vec<u32> {
        self.ghost_board().iter().map(|g| g.value).collect()
    }
}

pub fn rider_display_names(big_root: &Path) -> Result<HashMap<String, String>> {
    const FIELD_SHORT_NAME: u32 = 0x8097_DCBE;
    const FIELD_DISPLAY_NAME: u32 = 0xEAA0_A6F7;
    ssx_assets::vault_db::with_all_db(big_root, |db| {
        let mut out = HashMap::new();
        for (key, _) in db.collections_of_all(crate::gear::CLASS_CHARACTER) {
            let fields = db.fields(crate::gear::CLASS_CHARACTER, key)?;
            let text =
                |k: u32| fields.iter().find(|f| f.key == k).and_then(|f| ssx_assets::vault_db::single_text(&f.data));
            if let (Some(short), Some(display)) = (text(FIELD_SHORT_NAME), text(FIELD_DISPLAY_NAME)) {
                out.insert(short, display);
            }
        }
        Ok(out)
    })
}

pub fn load_gear_db(big_root: &Path) -> Result<GearDb> {
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let (vlt, bin) = read_pair(&dir.join("attribdb.vlt"))?;
    let (svlt, sbin) = read_pair(&dir.join("attribdb_schema.vlt"))?;
    let schema_vault = Vault::parse(&svlt, &sbin)?;
    let schema = Schema::from_vault(&schema_vault).context("reading the schema")?;
    let vault = Vault::parse(&vlt, &bin).context("parsing attribdb.vlt")?;
    let cols = collection::collections(&vault)?;
    let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
    let mut gear = GearDb::from_vault(&cols, &decoded, &collection::index(&cols))?;
    if let Ok((gv, gb)) = read_pair(&dir.join("ge_attribdb.vlt")) {
        let gvault = Vault::parse(&gv, &gb).context("parsing ge_attribdb.vlt")?;
        let gcols = collection::collections(&gvault)?;
        let gdec = gcols.iter().map(|c| c.decode(&gvault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        gear.add_riders(&gcols, &gdec, &collection::index(&gcols))?;
    }
    Ok(gear)
}

pub fn load_events(big_root: &Path) -> Result<Vec<EventDef>> {
    let gear = load_gear_db(big_root)?;
    let ghosts =
        ssx_assets::content::data::events::ghost_db_text(big_root).map(|t| parse_ghost_db(&t)).unwrap_or_default();
    Ok(ssx_assets::career::event_records(big_root)?.into_iter().map(|r| event_def(r, &gear, &ghosts)).collect())
}

fn event_def(r: ssx_rules::events::UiDropRecord, gear: &GearDb, ghosts: &[(u32, Rival)]) -> EventDef {
    let rivals = ghosts.iter().filter(|(id, _)| *id == r.key).map(|(_, rival)| *rival).collect();
    let forced_rider = r.forced[5];
    let forced_items: crate::gear::ForcedItems = std::array::from_fn(|k| r.forced[k]);
    let loadout = match (r.key == crate::gear::FINAL_DROP, r.slot_counts) {
        (true, Some(slots)) => gear.equipped_loadout(forced_rider, &crate::gear::FINAL_DROP_EQUIPS, slots),
        _ => (forced_rider != 0)
            .then(|| gear.fresh_profile_loadout(forced_rider, &forced_items, &r.required_equipment))
            .flatten(),
    };
    let message = crate::objectives::DropMessage { title: r.title, body: r.body, parameter: r.parameter, kind: r.kind };
    EventDef {
        name: r.name,
        key: r.key,
        drop_key: r.drop_key,
        event_type: r.event_type,
        scripts: r.scripts,
        rivals,
        required_equipment: r.required_equipment,
        avalanche_scripts: r.avalanche_scripts,
        forced_rider,
        forced_items,
        loadout,
        objectives: None,
        message,
        places: r.places,
        goal: r.goal,
    }
}

pub fn pick_event<'a>(events: &'a [EventDef], drop_key: u32, want: &str) -> Option<&'a EventDef> {
    choose_event(events.iter().filter(|e| e.drop_key == drop_key), want)
}

pub fn level_event(events: &[EventDef], launched: Option<u32>, want: impl FnOnce() -> String) -> Option<&EventDef> {
    match launched {
        Some(key) => events.iter().find(|e| e.key == key),
        None => match want().as_str() {
            "none" => None,
            want => choose_event(events.iter(), want),
        },
    }
}

pub fn choose_event<'a>(events: impl Iterator<Item = &'a EventDef>, want: &str) -> Option<&'a EventDef> {
    // PORT(choice): which of a drop's events plays without a front end (by type, one with `ghostdb.csv` rows of
    // starting index 0 first: the Explore events'), the port hosts' choice; the game's front end has the player pick a
    // `ui_drop`.
    let fits = |e: &EventDef| match (want, e.event_type) {
        (_, None) => false,
        ("race", Some(t)) => t.is_race(),
        ("trick", Some(t)) => t.is_trick(),
        ("survive", Some(t)) => t.is_survive(),
        _ => e.name.contains(want),
    };
    let list: Vec<&EventDef> = events.filter(|e| fits(e)).collect();
    if let Some(e) = list.iter().find(|e| e.name == want) {
        return Some(e);
    }
    crate::guess_hit!();
    list.iter().find(|e| e.rivals.iter().any(|r| r.starting_idx == 0)).or(list.first()).copied()
}
