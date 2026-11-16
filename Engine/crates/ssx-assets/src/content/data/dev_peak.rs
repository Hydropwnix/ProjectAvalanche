//! A port mod: the disc's unused developer peak, `test_peak`, on the globe.
//!
//! `ge_attribdb` keeps twenty drops on the peak `test_peak` (the ten `test_*` tracks of `gameexplorer`, whose level
//! files are all on the disc), but the disc has no `peak` record of that name and no `ui_drop` (event) on any of
//! them, so nothing in the menus reaches them. This adds both to the `attribdb` collections as they are read (next to
//! the content packages' routing of `ui_drop`s, so every reader of the events sees the same ones): a peak "Test Peak"
//! in the Rockies, between Whitehorn and the range's third peak, and one Explore Trick It event per drop on it, built
//! from an Explore event of the disc (its scripts, targets and loadout), with no rivals (no ghost rows have them).

use ssx_formats::attrib::collection::{Collection, DecodedCollection, Field, FieldData};
use ssx_formats::attrib::{RefSpec, Value, fnv1};

pub const PEAK_NAME: &str = "test_peak";
pub const PEAK: u32 = fnv1(PEAK_NAME.as_bytes());

const CLASS_PEAK: u32 = fnv1(b"peak");
const CLASS_UI_DROP: u32 = 0xB11E_A551;
const CLASS_GE_DROP: u32 = fnv1(b"ge_drop");

/// The disc's records the new ones are copied from: the Rockies' second peak (its range, scene link and look) and an
/// Explore Trick It event (`exp_newzealand_wakefield_drop_4_trick`).
const PEAK_TEMPLATE: u32 = 0x89E6_E130;
const PEAK_A: u32 = 0x89E6_E133;
const PEAK_B: u32 = 0x89E6_E131;
const EVENT_TEMPLATE: u32 = 0xBA22_B246;

const F_PEAK_TITLE: u32 = 0x4785_FF31;
const F_PEAK_INFO: u32 = 0x31BA_F20E;
const F_PEAK_POSITION: u32 = 0x5E19_D274;
const F_PEAK_SELECTABLE: u32 = 0x0E54_07B9;
// (the peak's own shots, between its neighbours'; the drop shot is the template peak's own, its drops are by those)
const PEAK_SHOTS: [u32; 4] = [0xBD46_56E0, 0xBAD1_AEA6, 0x9746_39E4, 0x8ECD_EEEA];

const F_GE_DROP: u32 = 0xB9C2_CFC5;
const F_NAME: u32 = 0xAF27_A974;
const F_PEAK_SHORT: u32 = 0xF2C4_3D5F;
const F_POSITION: u32 = 0x45B5_F74A;
const F_RIVALS: u32 = 0xC211_FF58;
const F_PEAK_REF: u32 = 0x8296_DA5A;

/// The peak's drops, one on each of the test tracks, from the top of the mountain down: the `ge_drop` (in
/// `ge_attribdb`, on `test_peak`), the event's name and the title's string key. (The disc has twelve drops on the flat
/// `test_plane`, the same track in different light and weather; the first of them stands for it. After them, the drops
/// of the unused `classic_peak`.)
pub const DROPS: [(u32, &str, &str); 14] = [
    (0x7B8C_2A8E, "dev_test_plane", "TXT_NM_DRP_TESTPLANE_1_1"),
    (0xC627_0A13, "dev_test_slope", "Test Slope"),
    (0x23A1_AB80, "dev_test_rail", "Test Rail"),
    (0xE325_ADE7, "dev_test_surface", "Test Surface"),
    (0x8572_9FDD, "dev_test_spray", "Test Spray"),
    (0x321B_99E8, "dev_test_scene", "Test Scene"),
    (0x3A14_67B6, "dev_test_audio", "Test Audio"),
    (0x6C85_B3E7, "dev_test_online", "Test Online"),
    (0x8E20_696D, "dev_test_profile", "TXT_NM_DRP_TEST_1_1"),
    // (the four drops of the disc's `classic_peak`, on their own tracks: no peak record and no event either)
    (0xCE47_CAA1, "dev_test_stylemile", "TXT_NM_DRP_STYLEMILE_1_1"),
    (0x9F3C_2FF0, "dev_test_snowjam", "TXT_NM_DRP_SNOWJAM_1_1"),
    (0x2596_D667, "dev_test_intimidator", "TXT_NM_DRP_INTIMIDATOR_1_1"),
    (0x7F11_465C, "dev_test_happiness", "TXT_NM_DRP_HAPPINESS_1_1"),
    // (the `ge_drop` named `default`: the record the others are made from, on no track; its marker can't be chosen)
    (0x9F7A_6694, DEFAULT_DROP, "default"),
];

const DEFAULT_DROP: &str = "dev_test_default";

/// Whether a drop of the test peak has a level to load (all but the `default` record's).
pub fn playable(key: u32) -> bool {
    key != fnv1(DEFAULT_DROP.as_bytes())
}

/// The strings the peak and its drops show (none of them is in the game's text).
pub fn strings() -> Vec<(String, String)> {
    vec![
        ("TXT_NM_PEA_TEST".to_string(), "Test Peak".to_string()),
        ("TXT_PEAK_INFO_TEST".to_string(), "The developers' test tracks, left unused in the game's files.".to_string()),
        ("TXT_NM_DRP_TEST_1_1".to_string(), "Test Profile".to_string()),
        ("TXT_NM_DRP_TESTPLANE_1_1".to_string(), "Test Plane".to_string()),
        ("TXT_NM_DRP_STYLEMILE_1_1".to_string(), "Style Mile".to_string()),
        ("TXT_NM_DRP_SNOWJAM_1_1".to_string(), "Snow Jam".to_string()),
        ("TXT_NM_DRP_INTIMIDATOR_1_1".to_string(), "Intimidator".to_string()),
        ("TXT_NM_DRP_HAPPINESS_1_1".to_string(), "Happiness".to_string()),
        ("default".to_string(), "Default (no track)".to_string()),
    ]
}

/// The drops' places on the drop screen (`droppos.rnf`, where the game keeps each drop's marker by name): the test
/// peak's snake down the mountain from its top in a widening wave (up and down move along them).
pub fn add_drop_points(entries: &mut Vec<(String, f32, f32)>) {
    if entries.is_empty() || entries.iter().any(|(n, _, _)| n.starts_with("dev_test_")) {
        return;
    }
    let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for &(_, x, y) in entries.iter() {
        (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
    }
    let (w, h) = (x1 - x0, y1 - y0);
    // (in the area's own share: the summit of the drop shot's mountain is near (0.62, 0.08))
    for (i, (_, name, _)) in DROPS.iter().enumerate() {
        // (a wave whose swing grows on the way down: a snaking line)
        let t = i as f32;
        let (u, v) = (0.62 + (0.05 + 0.012 * t) * (0.9 * t).sin(), 0.08 + 0.061 * t);
        entries.push(((*name).to_string(), x0 + w * u, y0 + h * v));
    }
}

pub fn is_dev_drop(key: u32) -> bool {
    DROPS.iter().any(|(_, name, _)| fnv1(name.as_bytes()) == key)
}

/// Adds the peak and its events to `attribdb`'s collections (once; nothing when the templates are not there, as in a
/// vault that is not the disc's `attribdb`).
pub fn inject(cols: &mut Vec<Collection>, decoded: &mut Vec<DecodedCollection>) {
    inject_from(cols, decoded, &[]);
}

/// As `inject`, the templates looked for in these vaults too (read before `attribdb`, such as the tuner pack and the
/// DLC's, whose copies of a record take its place).
pub fn inject_from(
    cols: &mut Vec<Collection>,
    decoded: &mut Vec<DecodedCollection>,
    others: &[(&[Collection], &[DecodedCollection])],
) {
    if cols.iter().any(|c| c.class == CLASS_PEAK && c.key == PEAK) {
        return;
    }
    let lookup = |class: u32, key: u32| -> Option<(Collection, DecodedCollection)> {
        let mine = std::iter::once((&cols[..], &decoded[..]));
        others.iter().copied().rev().chain(mine).find_map(|(c, d)| {
            let i = c.iter().position(|c| c.class == class && c.key == key)?;
            Some((c[i].clone(), d.get(i)?.clone()))
        })
    };
    let (Some(pt), Some(et)) = (lookup(CLASS_PEAK, PEAK_TEMPLATE), lookup(CLASS_UI_DROP, EVENT_TEMPLATE)) else {
        note(format!("test peak: not added (no template peak or event among {} collections)", cols.len()));
        return;
    };
    let vec3 = |d: &DecodedCollection, k: u32| match d.field(k).map(|f| &f.data) {
        Some(FieldData::Single(Value::Vector3(v))) => Some(*v),
        _ => None,
    };
    let mid = |a: [f32; 3], b: [f32; 3]| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0, (a[2] + b[2]) / 2.0];
    let (peak_a, peak_b) = (lookup(CLASS_PEAK, PEAK_A), lookup(CLASS_PEAK, PEAK_B));
    let between = |k: u32| {
        let a = peak_a.as_ref().and_then(|(_, d)| vec3(d, k));
        let b = peak_b.as_ref().and_then(|(_, d)| vec3(d, k));
        a.zip(b).map(|(a, b)| mid(a, b))
    };
    // (the fields a copy lacks come from any record that has them)
    let mut all: Vec<&[DecodedCollection]> = vec![&decoded[..]];
    all.extend(others.iter().map(|(_, d)| *d));
    let position = between(F_PEAK_POSITION).unwrap_or([-26_220.0, 8_444.0, -22_096.0]);

    // The peak.
    let mut peak = pt.0.clone();
    peak.key = PEAK;
    peak.export_key = fnv1(format!("peak/{PEAK_NAME}").as_bytes());
    let mut fields = pt.1.clone();
    set(&mut fields, &all, F_PEAK_TITLE, FieldData::Single(Value::Text(Some("TXT_NM_PEA_TEST".into()))));
    set(&mut fields, &all, F_PEAK_INFO, FieldData::Single(Value::Text(Some("TXT_PEAK_INFO_TEST".into()))));
    set(&mut fields, &all, F_PEAK_POSITION, FieldData::Single(Value::Vector3(position)));
    set(&mut fields, &all, F_PEAK_SELECTABLE, FieldData::Single(Value::Bool(true)));
    for k in PEAK_SHOTS {
        if let Some(v) = between(k) {
            set(&mut fields, &all, k, FieldData::Single(Value::Vector3(v)));
        }
    }
    let mut added = vec![(peak, fields)];

    // Its events, where the template peak's drop shot looks (its own drops' spread; the drop screen places the
    // markers by `add_drop_points`).
    let anchor = vec3(&pt.1, F_PEAK_POSITION).unwrap_or([16_843.65, 14_824.46, 26_394.14]);
    for (i, (ge_drop, name, _)) in DROPS.iter().enumerate() {
        let key = fnv1(name.as_bytes());
        let mut c = et.0.clone();
        c.key = key;
        c.export_key = fnv1(format!("ui_drop/{name}").as_bytes());
        let (col, row) = ((i % 10) as f32, (i / 10) as f32);
        let at = [anchor[0] - 1_850.0 + 420.0 * col, anchor[1] - 1_500.0 + 1_000.0 * row, anchor[2] + 250.0];
        let mut fields = et.1.clone();
        let ge = RefSpec { class: CLASS_GE_DROP, collection: *ge_drop };
        set(&mut fields, &all, F_GE_DROP, FieldData::Single(Value::Ref(ge)));
        set(&mut fields, &all, F_NAME, FieldData::Single(Value::Text(Some((*name).into()))));
        set(&mut fields, &all, F_PEAK_SHORT, FieldData::Single(Value::Text(Some(PEAK_NAME.into()))));
        set(&mut fields, &all, F_POSITION, FieldData::Single(Value::Vector3(at)));
        set(&mut fields, &all, F_RIVALS, FieldData::Array(Vec::new()));
        let peak = RefSpec { class: CLASS_PEAK, collection: PEAK };
        set(&mut fields, &all, F_PEAK_REF, FieldData::Single(Value::Ref(peak)));
        added.push((c, fields));
    }
    drop(all);
    let n = added.len();
    for (c, d) in added {
        cols.push(c);
        decoded.push(d);
    }
    note(format!("test peak: added (the peak and {} events)", n - 1));
}

/// One line in the game's log for each different note (the collections are read by several parts of the game).
fn note(line: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.contains(&line) {
        eprintln!("{line}");
        seen.push(line);
    }
}

/// Sets a field of a copied record: its own, or else one of the same key copied from any record that has it (the
/// field's type comes with it); nothing when no record has the field.
fn set(fields: &mut DecodedCollection, all: &[&[DecodedCollection]], key: u32, data: FieldData) {
    if let Some(f) = fields.fields.iter_mut().find(|f| f.key == key) {
        f.data = data;
        return;
    }
    let found: Option<Field> = all.iter().flat_map(|v| v.iter()).find_map(|d| d.field(key)).cloned();
    if let Some(mut f) = found {
        f.data = data;
        fields.fields.push(f);
    }
}
