use super::*;

const F_TITLE_RANGE: u32 = 0x4A54_73B5;
const F_MOVIE: u32 = 0xBC8B_0254;
const F_INDEX: u32 = 0x2AAC_C36E;
const F_SELECTABLE: u32 = fnv1(b"Selectable");
const F_PEAK_RANGE: u32 = 0xAB6B_2547;
const F_DIFFICULTY: u32 = fnv1(b"Difficulty");

pub(crate) fn ranges(v: &Vaults) -> Result<Vec<Range>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_RANGE) {
        let r = v.rec(CLASS_RANGE, key)?;
        out.push(Range {
            key,
            name: v.name("range", key),
            title: r.text(F_TITLE_RANGE).unwrap_or_default(),
            movie: r.text(F_MOVIE).unwrap_or_default(),
            unk_index: r.int(F_INDEX).unwrap_or(0),
            selectable: r.int(F_SELECTABLE) == Some(1),
        });
    }
    Ok(out)
}

pub(crate) fn peaks(v: &Vaults) -> Result<Vec<Peak>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_PEAK) {
        let r = v.rec(CLASS_PEAK, key)?;
        out.push(Peak {
            key,
            name: v.name("peak", key),
            range: r.reference(F_PEAK_RANGE),
            title: r.text(0x4785_FF31).unwrap_or_default(),
            info: r.text(0x31BA_F20E).unwrap_or_default(),
            image: r.text(0xC181_C790).unwrap_or_default(),
            selectable: r.int(F_SELECTABLE) == Some(1),
            campaign_type: r.int(fnv1(b"CampaignPeakType")).unwrap_or(0),
            unk_flag: r.int(0x1B2B_75B8) == Some(1),
            unk_floats: [
                r.float(0xBD23_C427).unwrap_or(0.0),
                r.float(0x970E_8CEB).unwrap_or(0.0),
                r.float(0x9EC4_2B55).unwrap_or(0.0),
            ],
        });
    }
    Ok(out)
}

pub(crate) fn drops(v: &Vaults) -> Result<Vec<Drop>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_GE_DROP) {
        let r = v.rec(CLASS_GE_DROP, key)?;
        let track_key = r.reference(fnv1(b"Track"));
        let track = v
            .rec(CLASS_GE_TRACK, track_key)
            .ok()
            .and_then(|t| t.text(0x1A72_5E76))
            .map(|t| t.trim_end_matches("_track").to_string())
            .unwrap_or_default();
        out.push(Drop {
            key,
            track,
            track_key,
            peak: r.text(0x17E8_41D0).unwrap_or_default(),
            title: r.text(0x1A32_9A87).unwrap_or_default(),
            selectable: r.int(F_SELECTABLE) == Some(1),
            difficulty: r.int(F_DIFFICULTY).unwrap_or(0),
        });
    }
    Ok(out)
}

pub(crate) fn events(v: &Vaults) -> Result<Vec<Event>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_UI_DROP) {
        let r = v.rec(CLASS_UI_DROP, key)?;
        let text = |k: u32| r.text(k).unwrap_or_default();
        out.push(Event {
            key,
            name: r.text(0xAF27_A974).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("{key:08x}")),
            drop: r.reference(0xB9C2_CFC5),
            event_type: r.int(0x146B_5937).unwrap_or(-1),
            drop_type: DropType::from_raw(r.int(0x555E_EBD2).unwrap_or(-1)),
            mode: text(0xE5FA_2B74),
            slot: text(0xC8B4_8ED3),
            peak_short: text(0xF2C4_3D5F),
            hud_title: text(0xAFD5_EF64),
            hud_body: text(0x4375_DFCA),
            goal_body: text(0x10C3_85C4),
            results_title: text(0x8DEC_513D),
            results_body: text(0x76C7_D93D),
            unlock_text: text(0x2BAB_24B9),
            deadly_descent: r.int(0x2A38_BB0C) == Some(1),
            warning: r.int(0x9A4C_11BE).unwrap_or(0),
            time_of_day: r.int(0x2318_0B90).unwrap_or(0),
            cost: r.int(0xB693_9DED).unwrap_or(0),
            difficulty: r.int(F_DIFFICULTY).unwrap_or(0),
            reward: r.int(0xF1FA_C946).unwrap_or(0),
            places: r.int(0xC7B7_41BF),
            targets: r.ints(0x3767_2E94),
            required: r.ints(0xD4D5_8427),
            unk_b683b553: r.ints(0xB683_B553),
            rivals: r.refs(0xC211_FF58),
            forced: r.refs(0x7E5E_ED53),
            unk_63180fcc: r.int(0x6318_0FCC) == Some(1),
            unk_89d02fa4: r.int(0x89D0_2FA4) == Some(1),
            unk_12bccfb5: r.int(0x12BC_CFB5).unwrap_or(0),
            unk_0b03fc19: r.int(0x0B03_FC19),
            unk_98e9ebcc: r.ints(0x98E9_EBCC),
            custom_status: r.int(0xB29D_587A).unwrap_or(0),
            unk_links: [r.reference(0x6CA1_C684), r.reference(0xE8F3_D9AF), r.reference(0xEBFB_2039)],
            peak_ref: r.reference(0x8296_DA5A),
            helicopter: r.reference(0x2101_12EA),
        });
    }
    Ok(out)
}

fn stat_pairs(bytes: &[u8], from: usize) -> Vec<(i32, u8)> {
    let (chunks, _) = bytes[from.min(bytes.len())..].as_chunks::<8>();
    chunks.iter().map(|c| (i32::from_be_bytes([c[0], c[1], c[2], c[3]]), c[4])).collect()
}

pub(crate) fn riders(v: &Vaults) -> Result<Vec<Rider>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_CHARACTER) {
        let r = v.rec(CLASS_CHARACTER, key)?;
        let stat_bonuses = r.raws(0xE412_CD30).iter().flat_map(|b| stat_pairs(b, 0)).collect();
        out.push(Rider {
            key,
            name: r.text(0x8097_DCBE).unwrap_or_else(|| v.name("character", key)),
            title: r.text(0xC068_AC7B).unwrap_or_default(),
            display_name: r.text(0xEAA0_A6F7).unwrap_or_default(),
            cost: r.int(fnv1(b"cost")).unwrap_or(0),
            locked: r.int(0x5650_1F0A) == Some(1),
            default: r.int(0x349E_965B) == Some(1),
            unk_940b9b22: r.int(0x940B_9B22).unwrap_or(0),
            stat_bonuses,
            portrait: r.text(0x2714_52A4).unwrap_or_default(),
            description: r.text(0x726F_CE94).unwrap_or_default(),
            round_icon: r.text(0xC0F1_B828).unwrap_or_default(),
            icon: r.text(0x9368_E42D).unwrap_or_default(),
            content: None,
        });
    }
    Ok(out)
}

pub const CREATED_EQUIPMENT: u32 = fnv1(b"created_equipment");
pub const STORE_TEMPLATES: u32 = 0x5013_A9CE;
pub const INFO: u32 = fnv1(b"info");

fn item(v: &Vaults, key: u32, group: u32) -> Result<Item> {
    let r = v.rec(CLASS_EQUIPMENT, key)?;
    let attrs = r.raws(0xFE0C_3C3F).into_iter().next().unwrap_or_default();
    let slot = attrs.get(..4).map_or(-1, |w| i64::from(i32::from_be_bytes([w[0], w[1], w[2], w[3]])));
    Ok(Item {
        key,
        group,
        title: r.raw_text(0x6329_8D35, 0).unwrap_or_default(),
        kind: r.int(fnv1(b"type")).unwrap_or(-1),
        slot,
        rider: r.reference(0x432D_A24B),
        locked: r.int(0x5650_1F0A) == Some(1),
        default: r.int(0x349E_965B) == Some(1),
        level: r.int(fnv1(b"level")).unwrap_or(0),
        quality: r.int(0x85A5_BC64).unwrap_or(0),
        cost: r.int(fnv1(b"cost")).unwrap_or(0),
        attributes: stat_pairs(&attrs, 4),
        unk_b6ef00c6: r.int(0xB6EF_00C6).unwrap_or(0),
        descriptions: r.refs(0x9F3E_A0B0),
        content: None,
    })
}

pub(crate) fn items(v: &Vaults) -> Result<Vec<Item>> {
    let keys = v.keys_of(CLASS_EQUIPMENT);
    let groups: Vec<u32> =
        keys.iter().copied().filter(|&k| v.parent(CLASS_EQUIPMENT, k) == CREATED_EQUIPMENT).collect();
    let mut out = Vec::new();
    for &k in &keys {
        let p = v.parent(CLASS_EQUIPMENT, k);
        if groups.contains(&p) {
            out.push(item(v, k, p)?);
        }
    }
    Ok(out)
}

pub(crate) fn templates(v: &Vaults) -> Result<Vec<StoreTemplate>> {
    let mut out = Vec::new();
    for k in v.keys_of(CLASS_EQUIPMENT).into_iter().filter(|&k| v.parent(CLASS_EQUIPMENT, k) == STORE_TEMPLATES) {
        let r = v.rec(CLASS_EQUIPMENT, k)?;
        let int = |f: u32| r.int(f).unwrap_or(0);
        out.push(StoreTemplate {
            key: k,
            title: r.raw_text(0x6329_8D35, 0).unwrap_or_default(),
            slot: r.int(0xCE9C_0603).unwrap_or(-1),
            kind: r.int(0x972C_8CB2).unwrap_or(-1),
            stats: r.ints(0x0A78_9FA7),
            price_scale: r.float(0xB862_10C3).unwrap_or(0.0),
            charlock_chance: int(0x2E98_BC3F),
            bonus_chance: int(0xE120_8475),
            min_level: int(0x5CAD_E5A0),
            max_level: int(0xDEF5_B19A),
            min_rarity: int(0xEC0D_8C05),
            max_rarity: int(0x57CE_9BD3),
            fixed_points: int(0xD867_E313),
            bonus_stats: r.ints(0x20BF_EB89),
            descriptions: r.refs(0xB251_4A1A),
            members: r.refs(0x95DA_EEAD),
        });
    }
    Ok(out)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoreTables {
    pub groups: Vec<u32>,
    pub rarity_weights: Vec<i64>,
    pub rarity_bands: Vec<i64>,
    pub rarity_price: Vec<f32>,
    pub stat_points: Vec<(i64, i64)>,
    pub bonus_points: Vec<(i64, i64)>,
    pub prices: Vec<(i64, i64)>,
    pub price_spread: Vec<i64>,
    pub durations: Vec<i64>,
    pub special_interval: (i64, i64),
    pub special_min_rarity: i64,
    pub mystery_chance: i64,
    pub stat_cap: i64,
    pub bonus_min_rarity: i64,
    pub bonus_price: f32,
    pub name_variants: [i64; 4],
    pub suit_match_chance: i64,
    pub sell_percent: i64,
    pub suit_score: i64,
}

fn min_max(r: &Rec<'_>, key: u32) -> Vec<(i64, i64)> {
    r.raws(key)
        .iter()
        .filter(|b| b.len() == 8)
        .map(|b| {
            let w = |i: usize| i64::from(i32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]));
            (w(0), w(4))
        })
        .collect()
}

pub(crate) fn store_tables(v: &Vaults) -> Result<StoreTables> {
    let r = v.rec(CLASS_EQUIPMENT, INFO)?;
    let int = |k: u32| r.int(k).unwrap_or(0);
    let interval = min_max(&r, 0x7B55_AC0D).first().copied().unwrap_or((0, 0));
    Ok(StoreTables {
        groups: r.refs(0x1A3F_D5C3),
        rarity_weights: r.ints(0x98AF_33F3),
        rarity_bands: r.ints(0xCBD9_D504),
        rarity_price: r.floats(0x01F0_1ABE),
        stat_points: min_max(&r, 0x1FEF_6756),
        bonus_points: min_max(&r, 0xE27A_B77B),
        prices: min_max(&r, 0xA1A6_A9D3),
        price_spread: r.ints(0x59E3_CBF1),
        durations: r.ints(0x6E97_A28C),
        special_interval: interval,
        special_min_rarity: int(0x48D8_3C1E),
        mystery_chance: int(0x8129_F431),
        stat_cap: int(0x1E82_2913),
        bonus_min_rarity: int(0x0985_4FC3),
        bonus_price: r.float(0xD059_1403).unwrap_or(1.0),
        name_variants: [int(0xAAC3_9713), int(0xF9C9_A6B2), int(0x1182_F749), int(0xFC57_3105)],
        suit_match_chance: int(0x4464_A727),
        sell_percent: int(0x0667_1AD9),
        suit_score: int(0x2CDB_E895),
    })
}

pub const BOOST_MAX_COLLECTION: u32 = 0x3291_8AA5;

pub(crate) fn boost_max(v: &Vaults) -> Result<Vec<u8>> {
    let r = v.rec(CLASS_EQUIPMENT, BOOST_MAX_COLLECTION)?;
    Ok(r.raws(0x26D5_959E).into_iter().next().unwrap_or_default())
}

pub const DEFAULT_EQUIPMENT: u32 = fnv1(b"default_equipment");

pub(crate) fn default_equipment(v: &Vaults) -> Result<Vec<u32>> {
    Ok(v.rec(CLASS_EQUIPMENT, DEFAULT_EQUIPMENT)?.refs(0xFB00_E151))
}

pub const CLASS_EQUIPMENT_DESCRIPTION: u32 = fnv1(b"equipment_description");

#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub key: u32,
    pub rider: u32,
    pub id: i64,
    pub slot: i64,
    pub excluded: bool,
    pub icon: String,
}

pub(crate) fn descriptions(v: &Vaults) -> Result<Vec<Description>> {
    let mut out = Vec::new();
    for key in v.keys_of(CLASS_EQUIPMENT_DESCRIPTION) {
        let r = v.rec(CLASS_EQUIPMENT_DESCRIPTION, key)?;
        out.push(Description {
            key,
            rider: r.reference(fnv1(b"character")),
            id: r.int(fnv1(b"id")).unwrap_or(0),
            slot: r.int(0x1779_0AC8).unwrap_or(-1),
            excluded: r.int(0xA37D_C8F7) == Some(1),
            icon: r.text(0x6EAA_937A).unwrap_or_default(),
        });
    }
    Ok(out)
}

pub(crate) fn progression(v: &Vaults) -> Result<Progression> {
    let r = v.rec(CLASS_EQUIPMENT, INFO)?;
    let named = [0x69BB_AD40, 0x665F_A517, 0x446D_D69D];
    let named: Vec<u32> = named.into_iter().chain(STORE_INFO_KEYS).chain([0x2CDB_E895]).collect();
    let mut unk = Vec::new();
    for k in r.keys().into_iter().filter(|k| !named.contains(k)) {
        let mut vals = r.ints(k);
        if vals.is_empty() {
            vals = r.floats(k).into_iter().map(|f| f as i64).collect();
        }
        if vals.is_empty() {
            vals = r
                .raws(k)
                .iter()
                .filter(|b| b.len() == 8)
                .flat_map(|b| [b[..4].to_vec(), b[4..].to_vec()])
                .map(|w| i64::from(i32::from_be_bytes([w[0], w[1], w[2], w[3]])))
                .collect();
        }
        if !vals.is_empty() {
            unk.push((k, vals));
        }
    }
    Ok(Progression {
        level_thresholds: r.ints(0x69BB_AD40),
        level_prices: r.ints(0x665F_A517),
        level_slots: r.raws(0x446D_D69D).iter().filter_map(|b| b.get(..5)?.try_into().ok()).collect(),
        unk,
    })
}

pub const REWARD_TIERS: [&str; 5] = ["easy", "medium", "hard", "extreme", "superextreme"];

pub(crate) fn explore_rewards(v: &Vaults) -> Result<Vec<ExploreRewards>> {
    let mut out = Vec::new();
    for tier in REWARD_TIERS {
        let r = v.rec(CLASS_EXPLORE_REWARDS, fnv1(tier.as_bytes()))?;
        let unk = [0x659D_C832u32, 0xE810_3A31, 0x7CF2_365E, 0x9F63_50CD, 0x163C_CFEB, 0xEAA2_81A2, 0x2F78_CEC3]
            .into_iter()
            .filter_map(|k| r.int(k).map(|x| x as f64).or(r.float(k).map(f64::from)).map(|x| (k, x)))
            .collect();
        out.push(ExploreRewards {
            tier: tier.to_string(),
            bronze: r.int(0xDFFF_8BBC).unwrap_or(0),
            silver: r.int(0x9A64_51B7).unwrap_or(0),
            gold: r.int(0x6377_06B2).unwrap_or(0),
            per_place_passed: r.int(0x9B3F_6C97).unwrap_or(0),
            unk,
        });
    }
    Ok(out)
}
