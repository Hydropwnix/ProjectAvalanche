//! A port mod: the disc's unused developer peak, `test_peak`, on the globe.
//!
//! `ge_attribdb` keeps twenty drops on the peak `test_peak` (the ten `test_*` tracks of `gameexplorer`, whose level
//! files are all on the disc), but the disc has no `peak` record of that name and no `ui_drop` (event) on any of
//! them, so nothing in the menus reaches them. This adds both to the `attribdb` collections as they are read (next to
//! the content packages' routing of `ui_drop`s, so every reader of the events sees the same ones): a peak "Test Peak"
//! in the Rockies, between Mount Robson and Extinguisher Tower, and one Explore Trick It event per drop on it, built
//! from an Explore event of the disc (its scripts, targets and loadout), with no rivals (no ghost rows have them).

use ssx_formats::attrib::collection::{Collection, DecodedCollection, Field, FieldData};
use ssx_formats::attrib::{RefSpec, Value, fnv1};

pub const PEAK_NAME: &str = "test_peak";
pub const PEAK: u32 = fnv1(PEAK_NAME.as_bytes());
/// The peak's picture ("{ }"): no file, the menus draw it (`ssx-ui`'s file source).
pub const IMAGE: &str = "assets/peak/dev_test_peak.dds";

const CLASS_PEAK: u32 = fnv1(b"peak");
const CLASS_UI_DROP: u32 = 0xB11E_A551;
const CLASS_GE_DROP: u32 = fnv1(b"ge_drop");

/// The disc's records the new ones are copied from: the Rockies' second peak (its range, scene link and look) and an
/// Explore Trick It event (`exp_newzealand_wakefield_drop_4_trick`).
pub const PEAK_TEMPLATE: u32 = 0x89E6_E130;
// (it stands between Mount Robson, the template, and Extinguisher Tower)
const PEAK_A: u32 = PEAK_TEMPLATE;
const PEAK_B: u32 = 0x89E6_E131;
/// The peak whose drop shot (its direction and distance) the test peak's drop shot takes: Mount Robson's.
pub const CAMERA_TEMPLATE: u32 = PEAK_TEMPLATE;
const EVENT_TEMPLATE: u32 = 0xBA22_B246;
/// The Race It versions' template: `exp_newzealand_wakefield_drop_3_race`.
const RACE_TEMPLATE: u32 = 0x6433_61E7;
/// The Race It versions' names: the Trick It event's with this after it.
const RACE: &str = "_race";

const F_PEAK_TITLE: u32 = 0x4785_FF31;
const F_PEAK_INFO: u32 = 0x31BA_F20E;
const F_PEAK_POSITION: u32 = 0x5E19_D274;
const F_PEAK_SELECTABLE: u32 = 0x0E54_07B9;
const F_PEAK_IMAGE: u32 = 0xC181_C790;
const F_PEAK_TYPE: u32 = fnv1(b"CampaignPeakType");
// (the peak's own shots, between its neighbours'; the drop shot is the template peak's own, its drops are by those)
const PEAK_SHOTS: [u32; 4] = [0xBD46_56E0, 0xBAD1_AEA6, 0x9746_39E4, 0x8ECD_EEEA];

const F_GE_DROP: u32 = 0xB9C2_CFC5;
const F_NAME: u32 = 0xAF27_A974;
const F_PEAK_SHORT: u32 = 0xF2C4_3D5F;
const F_POSITION: u32 = 0x45B5_F74A;
const F_RIVALS: u32 = 0xC211_FF58;
const F_PEAK_REF: u32 = 0x8296_DA5A;

/// The peak's drops, one on each of the test tracks (and the `classic_peak`'s), from the top of the mountain down: the `ge_drop` (in
/// `ge_attribdb`, on `test_peak`), the event's name and the title's string key. (The disc has twelve drops on the flat
/// `test_plane`, the same track in different light and weather; the first of them stands for it. After them, the drops
/// of the unused `classic_peak`.)
pub const DROPS: [(u32, &str, &str); 11] = [
    (0x8E20_696D, "dev_test_profile", "TXT_NM_DRP_TEST_1_1"),
    (0x7F11_465C, "dev_test_happiness", "TXT_NM_DRP_HAPPINESS_1_1"),
    (0x9F3C_2FF0, "dev_test_snowjam", "TXT_NM_DRP_SNOWJAM_1_1"),
    (0xCE47_CAA1, "dev_test_stylemile", "TXT_NM_DRP_STYLEMILE_1_1"),
    (0xE325_ADE7, "dev_test_surface", "Test Surface"),
    (0x6C85_B3E7, "dev_test_online", "Test Online"),
    (0x23A1_AB80, "dev_test_rail", "Test Rail"),
    (0xC627_0A13, "dev_test_slope", "Test Slope"),
    (0x8572_9FDD, "dev_test_spray", "Test Spray"),
    (0x7B8C_2A8E, "dev_test_plane", "TXT_NM_DRP_TESTPLANE_1_1"),
    (0x3A14_67B6, "dev_test_audio", "Test Audio"),
];

static RACE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the test peak shows its Race It events (Y on its drop screen) rather than its Trick It ones.
pub fn race_mode() -> bool {
    RACE_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn toggle_race_mode() -> bool {
    !RACE_MODE.fetch_xor(true, std::sync::atomic::Ordering::Relaxed)
}

static DROP_SCREEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the test peak's drop screen is up (set by the menus each frame; its button bar shows Y "Switch Event").
pub fn drop_screen() -> bool {
    DROP_SCREEN.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn set_drop_screen(up: bool) {
    DROP_SCREEN.store(up, std::sync::atomic::Ordering::Relaxed);
}

fn is_race(key: u32) -> bool {
    DROPS.iter().any(|(_, name, _)| fnv1(format!("{name}{RACE}").as_bytes()) == key)
}

/// Whether a drop is on the drop screen now: every drop but the test peak's events of the other kind.
pub fn shown(key: u32) -> bool {
    !is_dev_drop(key) || is_race(key) == race_mode()
}

/// A test peak event's other kind (Trick It ↔ Race It).
pub fn counterpart(key: u32) -> Option<u32> {
    DROPS.iter().find_map(|(_, name, _)| {
        let (trick, race) = (fnv1(name.as_bytes()), fnv1(format!("{name}{RACE}").as_bytes()));
        if key == trick {
            Some(race)
        } else if key == race {
            Some(trick)
        } else {
            None
        }
    })
}

/// The strings the peak and its drops show (none of them is in the game's text).
pub fn strings() -> Vec<(String, String)> {
    vec![
        ("TXT_NM_PEA_TEST".to_string(), "Test Peak".to_string()),
        ("TXT_PEAK_TYPE_TEST".to_string(), "Hidden".to_string()),
        ("TXT_PEAK_INFO_TEST".to_string(), "The developers' test tracks, left unused in the game's files.".to_string()),
        ("TXT_NM_DRP_TEST_1_1".to_string(), "Test Profile".to_string()),
        ("TXT_NM_DRP_TESTPLANE_1_1".to_string(), "Test Plane".to_string()),
        ("TXT_NM_DRP_STYLEMILE_1_1".to_string(), "Style Mile".to_string()),
        ("TXT_NM_DRP_SNOWJAM_1_1".to_string(), "Snow Jam".to_string()),
        ("TXT_NM_DRP_HAPPINESS_1_1".to_string(), "Happiness".to_string()),
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
    // (in the area's own share, worked out from a screenshot of the drop screen (ultrawide, 2576 pixels across):
    // `u` = 0.62 + (x - 1595) / 542, `v` = 0.08 + (y - 350) / 580. The markers: "narrowing snake, left first", from
    // the summit, swinging left first, the swings narrowing on the way down)
    let at = |x: f32, y: f32| (x0 + w * (0.62 + (x - 1595.0) / 542.0), y0 + h * (0.08 + (y - 350.0) / 580.0));
    for (i, (_, name, _)) in DROPS.iter().enumerate() {
        let t = i as f32;
        let (x, y) = at(1600.0 - (70.0 - 5.0 * t) * t.sin(), 365.0 + 36.0 * t);
        entries.push(((*name).to_string(), x, y));
        entries.push((format!("{name}{RACE}"), x, y));
    }
    // (the path's tail, past the last marker down to where the snow ends)
    for (i, (x, y)) in [(1640.0, 765.0), (1600.0, 805.0), (1630.0, 845.0)].into_iter().enumerate() {
        let (x, y) = at(x, y);
        entries.push((format!("{PATH_TAIL}{i}"), x, y));
    }
}

/// The names of the path's tail points among the drop screen's places (`add_drop_points`).
pub const PATH_TAIL: &str = "dev_test_path_tail_";

/// The test peak's path on the drop screen, top to bottom: its markers' places, then its tail's.
pub fn path_points(entries: &[(String, f32, f32)]) -> Vec<(f32, f32)> {
    let find = |n: &str| entries.iter().find(|(e, _, _)| e == n).map(|&(_, x, y)| (x, y));
    let markers = DROPS.iter().filter_map(|(_, name, _)| find(*name));
    let tail = (0..3).filter_map(|i| find(&format!("{PATH_TAIL}{i}")));
    markers.chain(tail).collect()
}

pub fn is_dev_drop(key: u32) -> bool {
    DROPS.iter().any(|(_, name, _)| fnv1(name.as_bytes()) == key) || is_race(key)
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
    set(&mut fields, &all, F_PEAK_IMAGE, FieldData::Single(Value::Text(Some(IMAGE.into()))));
    let plain = Value::Raw { bytes: vec![0; 4], pointers: Vec::new() };
    set(&mut fields, &all, F_PEAK_TYPE, FieldData::Single(plain));
    for k in PEAK_SHOTS {
        if let Some(v) = between(k) {
            set(&mut fields, &all, k, FieldData::Single(Value::Vector3(v)));
        }
    }
    let mut added = vec![(peak, fields)];

    // Its events, where the template peak's drop shot looks (its own drops' spread; the drop screen places the
    // markers by `add_drop_points`).
    let anchor = vec3(&pt.1, F_PEAK_POSITION).unwrap_or([16_843.65, 14_824.46, 26_394.14]);
    let race = lookup(CLASS_UI_DROP, RACE_TEMPLATE);
    let kinds = std::iter::once((&et, String::new())).chain(race.as_ref().map(|r| (r, RACE.to_string())));
    let events = kinds.flat_map(|(t, suffix)| DROPS.iter().enumerate().map(move |(i, d)| (t, suffix.clone(), i, d)));
    for (template, suffix, i, (ge_drop, name, _)) in events {
        let name = format!("{name}{suffix}");
        let key = fnv1(name.as_bytes());
        let mut c = template.0.clone();
        c.key = key;
        c.export_key = fnv1(format!("ui_drop/{name}").as_bytes());
        let (col, row) = ((i % 10) as f32, (i / 10) as f32);
        let at = [anchor[0] - 1_850.0 + 420.0 * col, anchor[1] - 1_500.0 + 1_000.0 * row, anchor[2] + 250.0];
        let mut fields = template.1.clone();
        let ge = RefSpec { class: CLASS_GE_DROP, collection: *ge_drop };
        set(&mut fields, &all, F_GE_DROP, FieldData::Single(Value::Ref(ge)));
        set(&mut fields, &all, F_NAME, FieldData::Single(Value::Text(Some(name.clone()))));
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
