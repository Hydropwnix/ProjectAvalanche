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
const PEAK_SHOTS: [u32; 6] = [0xBD46_56E0, 0xBAD1_AEA6, 0x9746_39E4, 0x8ECD_EEEA, 0x9EF1_4AB2, 0x630F_3BF4];

const F_GE_DROP: u32 = 0xB9C2_CFC5;
const F_NAME: u32 = 0xAF27_A974;
const F_PEAK_SHORT: u32 = 0xF2C4_3D5F;
const F_POSITION: u32 = 0x45B5_F74A;
const F_RIVALS: u32 = 0xC211_FF58;
const F_PEAK_REF: u32 = 0x8296_DA5A;

/// The peak's drops: the `ge_drop` (in `ge_attribdb`, on `test_peak`), the event's name and the title's string key.
pub const DROPS: [(u32, &str, &str); 20] = [
    (0x7B8C_2A8E, "dev_test_plane_1", "TXT_NM_DRP_TESTPLANE_1_1"),
    (0xCC3D_0DC3, "dev_test_plane_2", "TXT_NM_DRP_TESTPLANE_1_2"),
    (0x5544_B724, "dev_test_plane_3", "TXT_NM_DRP_TESTPLANE_1_3"),
    (0xBC35_46B9, "dev_test_plane_4", "TXT_NM_DRP_TESTPLANE_1_4"),
    (0x8CD4_52B2, "dev_test_plane_5", "TXT_NM_DRP_TESTPLANE_1_5"),
    (0x9489_0117, "dev_test_plane_6", "TXT_NM_DRP_TESTPLANE_1_6"),
    (0x112B_1EB8, "dev_test_plane_7", "TXT_NM_DRP_TESTPLANE_1_7"),
    (0x18CA_22FD, "dev_test_plane_8", "TXT_NM_DRP_TESTPLANE_1_8"),
    (0xB06A_6676, "dev_test_plane_9", "TXT_NM_DRP_TESTPLANE_1_9"),
    (0xBB1A_9CB2, "dev_test_plane_10", "TXT_NM_DRP_TESTPLANE_1_10"),
    (0xEA7B_90B9, "dev_test_plane_11", "TXT_NM_DRP_TESTPLANE_1_11"),
    (0x3F71_68B8, "dev_test_plane_12", "TXT_NM_DRP_TESTPLANE_1_12"),
    (0xC627_0A13, "dev_test_slope", "Test Slope"),
    (0x23A1_AB80, "dev_test_rail", "Test Rail"),
    (0xE325_ADE7, "dev_test_surface", "Test Surface"),
    (0x8572_9FDD, "dev_test_spray", "Test Spray"),
    (0x321B_99E8, "dev_test_scene", "Test Scene"),
    (0x3A14_67B6, "dev_test_audio", "Test Audio"),
    (0x6C85_B3E7, "dev_test_online", "Test Online"),
    (0x8E20_696D, "dev_test_profile", "TXT_NM_DRP_TEST_1_1"),
];

/// The strings the peak and its drops show (none of them is in the game's text).
pub fn strings() -> Vec<(String, String)> {
    let mut out = vec![
        ("TXT_NM_PEA_TEST".to_string(), "Test Peak".to_string()),
        ("TXT_PEAK_INFO_TEST".to_string(), "The developers' test tracks, left unused in the game's files.".to_string()),
        ("TXT_NM_DRP_TEST_1_1".to_string(), "Test Profile".to_string()),
    ];
    for n in 1..=12 {
        out.push((format!("TXT_NM_DRP_TESTPLANE_1_{n}"), format!("Test Plane {n}")));
    }
    out
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

    // Its events, on a ring below the peak.
    let n = DROPS.len() as f32;
    for (i, (ge_drop, name, _)) in DROPS.iter().enumerate() {
        let key = fnv1(name.as_bytes());
        let mut c = et.0.clone();
        c.key = key;
        c.export_key = fnv1(format!("ui_drop/{name}").as_bytes());
        let a = i as f32 / n * std::f32::consts::TAU;
        let at = [position[0] + 3_000.0 * a.cos(), position[1] - 2_000.0, position[2] + 3_000.0 * a.sin()];
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
