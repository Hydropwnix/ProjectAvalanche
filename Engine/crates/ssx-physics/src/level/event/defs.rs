use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventType {
    TrickScore = 0,
    TrickBots = 1,
    TrickGhosts = 2,
    SurviveToBottom = 3,
    SurviveToDeath = 4,
    RaceClock = 5,
    RaceGhosts = 6,
    RaceBots = 7,
    FreeRide = 8,
}

impl EventType {
    pub fn from_raw(v: u32) -> Option<Self> {
        use EventType::*;
        [TrickScore, TrickBots, TrickGhosts, SurviveToBottom, SurviveToDeath, RaceClock, RaceGhosts, RaceBots, FreeRide]
            .get(v as usize)
            .copied()
    }

    pub fn name(self) -> &'static str {
        [
            "EventType_TrickScore",
            "EventType_TrickBots",
            "EventType_TrickGhosts",
            "EventType_SurviveToBottom",
            "EventType_SurviveToDeath",
            "EventType_RaceClock",
            "EventType_RaceGhosts",
            "EventType_RaceBots",
            "EventType_FreeRide",
        ][self as usize]
    }

    pub fn is_race(self) -> bool {
        matches!(self, EventType::RaceClock | EventType::RaceGhosts | EventType::RaceBots)
    }

    pub fn is_trick(self) -> bool {
        matches!(self, EventType::TrickScore | EventType::TrickBots | EventType::TrickGhosts)
    }

    pub fn is_survive(self) -> bool {
        matches!(self, EventType::SurviveToBottom | EventType::SurviveToDeath)
    }

    pub fn ranks_ascending(self) -> bool {
        self.is_race()
    }
}

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

    pub fn ranked_rivals(&self) -> Vec<u32> {
        self.rivals.iter().filter(|r| r.starting_idx == 0).map(Rival::value).collect()
    }
}

pub(super) const CLASS_UI_DROP: u32 = fnv1(b"ui_drop");
pub(super) const FIELD_UI_DROP_GE_DROP: u32 = 0xB9C2_CFC5;
pub(super) const FIELD_UI_DROP_EVENT_TYPE: u32 = 0x146B_5937;
pub(super) const FIELD_UI_DROP_SCRIPTS: u32 = 0x0E24_143E;
pub(super) const FIELD_UI_DROP_NAME: u32 = 0xAF27_A974;
pub(super) const FIELD_UI_DROP_REQ_EQUIPMENT: u32 = 0xD4D5_8427;
pub(super) const FIELD_UI_DROP_AVALANCHE_SCRIPTS: u32 = 0xB1C0_1F64;
pub(super) const FIELD_UI_DROP_FORCED_LOADOUT: u32 = 0x7E5E_ED53;
pub(super) const FIELD_UI_DROP_SLOT_COUNTS: u32 = 0x98E9_EBCC;
pub(super) const FIELD_UI_DROP_HUD_TITLE: u32 = 0xAFD5_EF64;
pub(super) const FIELD_UI_DROP_HUD_BODY: u32 = 0x4375_DFCA;
pub(super) const FIELD_UI_DROP_MESSAGE_PARAMETER: u32 = 0x0B03_FC19;
pub(super) const FIELD_UI_DROP_TYPE: u32 = 0x555E_EBD2;

pub(crate) fn read_pair(vlt: &Path) -> Result<(Vec<u8>, Vec<u8>)> {
    let bin = vlt.with_extension("bin");
    Ok((
        std::fs::read(vlt).with_context(|| format!("reading {}", vlt.display()))?,
        std::fs::read(&bin).with_context(|| format!("reading {}", bin.display()))?,
    ))
}

pub(crate) fn values(d: &FieldData) -> Vec<&Value> {
    match d {
        FieldData::Single(v) => vec![v],
        FieldData::Array(vs) => vs.iter().collect(),
    }
}

pub fn rider_display_names(big_root: &Path) -> Result<HashMap<String, String>> {
    const FIELD_SHORT_NAME: u32 = 0x8097_DCBE;
    const FIELD_DISPLAY_NAME: u32 = 0xEAA0_A6F7;
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let (svlt, sbin) = read_pair(&dir.join("attribdb_schema.vlt"))?;
    let schema = Schema::from_vault(&Vault::parse(&svlt, &sbin)?).context("reading the schema")?;
    let mut out = HashMap::new();
    for file in ["attribdb.vlt", "ge_attribdb.vlt"] {
        let (v, b) = read_pair(&dir.join(file))?;
        let vault = Vault::parse(&v, &b).with_context(|| format!("parsing {file}"))?;
        let cols = collection::collections(&vault)?;
        let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        let index = collection::index(&cols);
        for (i, _) in cols.iter().enumerate().filter(|(_, c)| c.class == crate::gear::CLASS_CHARACTER) {
            let fields = collection::with_inherited(&cols, &decoded, &index, i)?;
            let text = |k: u32| {
                fields.iter().find(|(_, f)| f.key == k).and_then(|(_, f)| {
                    values(&f.data)
                        .into_iter()
                        .find_map(|v| if let Value::Text(Some(t)) = v { Some(t.clone()) } else { None })
                })
            };
            if let (Some(short), Some(display)) = (text(FIELD_SHORT_NAME), text(FIELD_DISPLAY_NAME)) {
                out.insert(short, display);
            }
        }
    }
    Ok(out)
}

#[allow(clippy::type_complexity)]
pub(super) fn read_attribdb(
    big_root: &Path,
) -> Result<(Vec<collection::Collection>, Vec<collection::DecodedCollection>, HashMap<(u32, u32), usize>, GearDb)> {
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let (vlt, bin) = read_pair(&dir.join("attribdb.vlt"))?;
    let (svlt, sbin) = read_pair(&dir.join("attribdb_schema.vlt"))?;
    let schema_vault = Vault::parse(&svlt, &sbin)?;
    let schema = Schema::from_vault(&schema_vault).context("reading the schema")?;
    let vault = Vault::parse(&vlt, &bin).context("parsing attribdb.vlt")?;
    let cols = collection::collections(&vault)?;
    let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
    let index = collection::index(&cols);
    let mut gear = GearDb::from_vault(&cols, &decoded, &index)?;
    if let Ok((gv, gb)) = read_pair(&dir.join("ge_attribdb.vlt")) {
        let gvault = Vault::parse(&gv, &gb).context("parsing ge_attribdb.vlt")?;
        let gcols = collection::collections(&gvault)?;
        let gdec = gcols.iter().map(|c| c.decode(&gvault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
        gear.add_riders(&gcols, &gdec, &collection::index(&gcols))?;
    }
    Ok((cols, decoded, index, gear))
}

pub fn load_gear_db(big_root: &Path) -> Result<GearDb> {
    Ok(read_attribdb(big_root)?.3)
}

pub fn load_events(big_root: &Path) -> Result<Vec<EventDef>> {
    let (cols, decoded, index, gear) = read_attribdb(big_root)?;
    let ghosts = std::fs::read_to_string(big_root.join("bootcache/data/common/ghosts/ghostdb.csv"))
        .map(|t| parse_ghost_db(&t))
        .unwrap_or_default();
    let mut out = Vec::new();
    for (i, c) in cols.iter().enumerate().filter(|(_, c)| c.class == CLASS_UI_DROP) {
        let fields = collection::with_inherited(&cols, &decoded, &index, i)?;
        let field = |k: u32| fields.iter().find(|(_, f)| f.key == k).map(|(_, f)| &f.data);
        let drop_key = field(FIELD_UI_DROP_GE_DROP)
            .and_then(|d| {
                values(d).into_iter().find_map(|v| if let Value::Ref(r) = v { Some(r.collection) } else { None })
            })
            .unwrap_or(0);
        let event_type = field(FIELD_UI_DROP_EVENT_TYPE).and_then(|d| match values(d).first() {
            Some(Value::Raw { bytes, .. }) if bytes.len() >= 4 => {
                EventType::from_raw(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            }
            _ => None,
        });
        let texts = |k: u32| -> Vec<String> {
            field(k).map_or_else(Vec::new, |d| {
                values(d)
                    .into_iter()
                    .filter_map(|v| if let Value::Text(Some(t)) = v { Some(t.clone()) } else { None })
                    .collect()
            })
        };
        let name = texts(FIELD_UI_DROP_NAME).into_iter().next().unwrap_or_else(|| format!("{:08x}", c.key));
        let rivals = ghosts.iter().filter(|(id, _)| *id == c.key).map(|(_, r)| *r).collect();
        let required_equipment = field(FIELD_UI_DROP_REQ_EQUIPMENT).map_or_else(Vec::new, |d| {
            values(d)
                .into_iter()
                .filter_map(|v| match v {
                    Value::Raw { bytes, .. } if bytes.len() >= 4 => {
                        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                    }
                    _ => None,
                })
                .collect()
        });
        let forced = |k: usize| {
            field(FIELD_UI_DROP_FORCED_LOADOUT).map_or(0, |d| match values(d).get(k).copied() {
                Some(Value::Ref(r)) => r.collection,
                _ => 0,
            })
        };
        let forced_rider = forced(5);
        let forced_items: crate::gear::ForcedItems = std::array::from_fn(forced);
        let slot_counts = field(FIELD_UI_DROP_SLOT_COUNTS).map(|d| {
            let mut out = [0u32; 5];
            for (o, v) in out.iter_mut().zip(values(d)) {
                *o = match v {
                    Value::UInt8(b) => u32::from(*b).min(4),
                    Value::UInt32(w) => (*w & 0xFF).min(4),
                    _ => 0,
                };
            }
            out
        });
        let loadout = match (c.key == crate::gear::FINAL_DROP, slot_counts) {
            (true, Some(slots)) => gear.equipped_loadout(forced_rider, &crate::gear::FINAL_DROP_EQUIPS, slots),
            _ => (forced_rider != 0)
                .then(|| gear.fresh_profile_loadout(forced_rider, &forced_items, &required_equipment))
                .flatten(),
        };
        let scripts = texts(FIELD_UI_DROP_SCRIPTS);
        let avalanche_scripts = texts(FIELD_UI_DROP_AVALANCHE_SCRIPTS);
        let word = |k: u32| {
            field(k).and_then(|d| match values(d).first().copied() {
                Some(Value::UInt32(v)) => Some(*v as i32),
                Some(Value::Int32(v)) => Some(*v),
                Some(Value::Raw { bytes, .. }) if bytes.len() >= 4 => {
                    Some(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                }
                _ => None,
            })
        };
        let text = |k: u32| texts(k).into_iter().next().filter(|t| !t.is_empty());
        let message = crate::objectives::DropMessage {
            title: text(FIELD_UI_DROP_HUD_TITLE),
            body: text(FIELD_UI_DROP_HUD_BODY),
            parameter: word(FIELD_UI_DROP_MESSAGE_PARAMETER).unwrap_or(0),
            kind: word(FIELD_UI_DROP_TYPE).unwrap_or(0),
        };
        out.push(EventDef {
            name,
            key: c.key,
            drop_key,
            event_type,
            scripts,
            rivals,
            required_equipment,
            avalanche_scripts,
            forced_rider,
            forced_items,
            loadout,
            objectives: None,
            message,
        });
    }
    Ok(out)
}

pub fn pick_event<'a>(events: &'a [EventDef], drop_key: u32, want: &str) -> Option<&'a EventDef> {
    choose_event(events.iter().filter(|e| e.drop_key == drop_key), want)
}

pub fn choose_event<'a>(events: impl Iterator<Item = &'a EventDef>, want: &str) -> Option<&'a EventDef> {
    // TODO(guess): which of a drop's events plays without a front end (by type, ranked rivals first) is a port choice; settle: the front end's selection once ported (the player picks a `ui_drop`).
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
    list.iter().find(|e| !e.ranked_rivals().is_empty()).or(list.first()).copied()
}
