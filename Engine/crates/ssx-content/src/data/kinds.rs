use serde::Deserialize;
use serde::de::{self, Visitor};

use super::schema::{Class, FieldSchema, FieldType, RecordSchema, Scalar};
use crate::id::Kind;
use crate::marker::Shape;
use crate::rider::character::CLEARED_SLOTS;
use crate::rider::{GEAR_STATS, GEAR_TYPES, MAX_MATERIAL_BINDINGS, MAX_STAT_PAIRS, SLOTS};

pub fn fields_of<T: for<'de> Deserialize<'de>>() -> &'static [&'static str] {
    let mut names = FieldNames(&[]);
    let _ = T::deserialize(&mut names);
    names.0
}

struct FieldNames(&'static [&'static str]);

impl<'de> de::Deserializer<'de> for &mut FieldNames {
    type Error = de::value::Error;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
        Err(de::Error::custom("not a struct"))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        fields: &'static [&'static str],
        _: V,
    ) -> Result<V::Value, Self::Error> {
        self.0 = fields;
        Err(de::Error::custom("the fields are read"))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit unit_struct
        newtype_struct seq tuple tuple_struct map enum identifier ignored_any
    }
}

fn f(ty: FieldType, class: Class, source: &str) -> FieldSchema {
    FieldSchema { ty, class, source: source.into(), note: String::new(), optional: true }
}

fn s(x: Scalar) -> FieldType {
    FieldType::Scalar(x)
}

fn id(kind: Kind) -> FieldType {
    s(Scalar::Id(kind))
}

fn text(max: usize, one_of: &[&str]) -> FieldType {
    s(Scalar::Text { max, one_of: one_of.iter().map(|x| x.to_string()).collect() })
}

fn int(min: i64, max: i64) -> FieldType {
    s(Scalar::Int { min, max })
}

fn stat_pairs() -> FieldType {
    let pair = FieldType::Tuple(vec![int(-1, i64::from(GEAR_STATS) - 1), int(0, 255)]);
    FieldType::List { elem: Box::new(pair), min: 0, max: MAX_STAT_PAIRS, ascending: false }
}

fn materials() -> FieldType {
    let key = Scalar::Text { max: 128, one_of: Vec::new() };
    FieldType::Map { key, value: Box::new(id(Kind::Material)), max: MAX_MATERIAL_BINDINGS }
}

fn generated(name: &str, names: &[&str], typed: Vec<(&str, FieldSchema)>) -> RecordSchema {
    let mut rs = RecordSchema::new(name);
    for &n in names {
        let field = typed.iter().find(|(t, _)| *t == n).map(|(_, x)| x.clone());
        let field = field.unwrap_or_else(|| f(s(Scalar::Fixed("not described")), Class::Presentation, ""));
        rs.fields.insert(n.to_string(), field);
    }
    rs
}

pub const HITS: [&str; 5] = ["none", "sphere", "cylinder", "ring", "gate"];

const P: Class = Class::Presentation;
const SIM: Class = Class::Simulation;

pub fn character() -> RecordSchema {
    let mut career = generated(
        "career",
        fields_of::<crate::rider::character::RawCareer>(),
        vec![
            ("title", f(id(Kind::String), P, "a string id (0xC068AC7B)")),
            ("display_name", f(text(32, &[]), P, "1 … 32 of A–Z a–z 0–9 space . _ -, a letter first")),
            ("description", f(id(Kind::String), P, "")),
            ("cost", f(int(0, crate::rider::character::MAX_COST), P, "credits")),
            ("locked", f(s(Scalar::Bool), P, "")),
            ("stat_bonuses", f(stat_pairs(), SIM, "the loadout's commit adds them (0x82498BA0)")),
            ("portrait", f(id(Kind::Texture), P, "")),
            ("round_icon", f(id(Kind::Texture), P, "")),
            ("icon", f(id(Kind::Texture), P, "")),
            ("default", f(s(Scalar::Bool), P, "")),
            ("0x940b9b22", f(int(i64::MIN, i64::MAX), P, "not traced")),
        ],
    );
    for n in ["title", "display_name", "cost"] {
        if let Some(x) = career.fields.get_mut(n) {
            x.optional = false;
        }
    }
    let mut outfit = RecordSchema::new("outfit");
    let slot = FieldType::OneOf(vec![text(4, &["none"]), id(Kind::Gear)]);
    for name in SLOTS.iter().filter(|n| !CLEARED_SLOTS.contains(n)) {
        outfit.fields.insert(name.to_string(), f(slot.clone(), P, "a package's gear look, or none"));
    }
    let lines = FieldType::Map {
        key: Scalar::Id(Kind::Sound),
        value: Box::new(id(Kind::Sound)),
        max: crate::rider::character::MAX_VOICE_LINES,
    };
    generated(
        "character.toml",
        fields_of::<crate::rider::character::Raw>(),
        vec![
            ("stands_in_for", f(id(Kind::Character), SIM, "a disc rider: the ride's character")),
            ("face", f(id(Kind::Character), P, "a disc rider")),
            ("voice", f(id(Kind::Character), P, "a disc rider")),
            ("voice_lines", f(lines, P, "a disc speech event to a sound")),
            ("default_board", f(id(Kind::Board), P, "")),
            ("outfit", f(FieldType::Table(Box::new(outfit)), P, "not iceaxe, oxygentank, wingsuit (0x8293F298)")),
            ("career", f(FieldType::Table(Box::new(career)), P, "")),
            ("materials", f(materials(), P, "")),
        ],
    )
}

pub fn item(board: bool) -> RecordSchema {
    let slots: Vec<&str> = GEAR_TYPES.iter().map(|(n, _, _)| *n).filter(|n| *n != "board").collect();
    let names: Vec<&str> =
        fields_of::<crate::rider::item::Raw>().iter().copied().filter(|n| !(board && *n == "slot")).collect();
    let mut rs = generated(
        if board { "board.toml" } else { "gear.toml" },
        &names,
        vec![
            ("slot", f(text(16, &slots), P, "the GearItemType")),
            ("title", f(id(Kind::String), P, "FeAttributes 0x63298D35")),
            ("rider", f(id(Kind::Character), P, "0x432DA24B")),
            ("locked", f(s(Scalar::Bool), P, "")),
            ("default", f(s(Scalar::Bool), P, "")),
            ("level", f(int(0, crate::rider::item::MAX_LEVEL), P, "equipment/info 0x69BBAD40: ten levels")),
            ("quality", f(int(0, crate::rider::item::MAX_QUALITY), P, "QualityTypes")),
            ("cost", f(int(0, crate::rider::character::MAX_COST), P, "credits")),
            ("attributes", f(stat_pairs(), SIM, "GearStats the loadout's commit puts into the game setup")),
            ("icon", f(id(Kind::Texture), P, "")),
            ("0xb6ef00c6", f(int(i64::MIN, i64::MAX), P, "not traced")),
            ("materials", f(materials(), P, "")),
        ],
    );
    for n in ["title", "cost", "slot"] {
        if let Some(x) = rs.fields.get_mut(n) {
            x.optional = false;
        }
    }
    rs
}

pub fn marker() -> RecordSchema {
    let num = |lo: f64, hi: f64| s(Scalar::Float { min: lo, max: hi, f32: false });
    let animation = generated(
        "animation",
        fields_of::<crate::marker::RawAnimation>(),
        vec![
            ("spin", f(num(-720.0, 720.0), P, "degrees a second")),
            ("bob", f(num(0.0, 10.0), P, "metres")),
            ("bob_period", f(num(0.1, 60.0), P, "seconds")),
            ("pulse", f(num(0.0, 1.0), P, "a fraction of the size")),
            ("pulse_period", f(num(0.1, 60.0), P, "seconds")),
        ],
    );
    let shapes: Vec<&str> = Shape::ALL.iter().map(|x| x.name()).collect();
    let hits = HITS;
    let rgba = FieldType::List { elem: Box::new(int(0, 255)), min: 3, max: 4, ascending: false };
    generated(
        "marker.toml",
        fields_of::<crate::marker::RawMarker>(),
        vec![
            ("shape", f(text(16, &shapes), P, "the port's meshes")),
            ("model", f(id(Kind::Model), P, "LOD 0, at most 65 536 vertices and triangles")),
            ("size", f(num(0.1, 100.0), P, "metres")),
            ("color", f(FieldType::OneOf(vec![text(9, &[]), rgba]), P, "\"#RRGGBB[AA]\" or [r, g, b(, a)]")),
            ("glow", f(num(0.0, 8.0), P, "")),
            ("facing", f(text(8, &["world", "camera"]), P, "")),
            ("draw_distance", f(num(1.0, 5000.0), P, "metres")),
            ("icon", f(id(Kind::Texture), P, "")),
            ("sound", f(id(Kind::Sound), P, "")),
            ("hit", f(text(8, &hits), P, "")),
            ("animation", f(FieldType::Table(Box::new(animation)), P, "")),
        ],
    )
}

pub fn map() -> RecordSchema {
    use crate::map::{MAX_TIME_LIMIT_S, MAX_TITLE_CHARS, Mode};
    let modes: Vec<&str> = Mode::ALL.iter().map(|m| m.name()).collect();
    let table = generated(
        "map",
        fields_of::<crate::map::RawMapTable>(),
        vec![
            ("title", f(text(MAX_TITLE_CHARS * 4, &[]), P, "1 … 64 characters")),
            ("scene", f(text(crate::path::MAX_PATH_LEN, &[]), P, "a package path to a .glb or .gltf file")),
            ("base", f(id(Kind::Drop), SIM, "a disc drop (ssx:drop/<ui_drop name>): a layout")),
            (
                "modes",
                f(
                    FieldType::List { elem: Box::new(text(8, &modes)), min: 1, max: modes.len(), ascending: false },
                    SIM,
                    "",
                ),
            ),
            (
                "lighting",
                f(id(Kind::Lighting), SIM, "a preset of the disc (ssx:lighting/<name>): its sun, night and head lamp"),
            ),
            ("sky", f(id(Kind::Sky), P, "")),
            ("postfx", f(id(Kind::Postfx), P, "a preset of the disc (ssx:postfx/<name>)")),
            (
                "music",
                f(id(Kind::Music), P, "a song of the disc (ssx:music/licensed/<id>) or a package's song or playlist"),
            ),
            (
                "time_limit_s",
                f(
                    s(Scalar::Float { min: 0.0, max: MAX_TIME_LIMIT_S, f32: false }),
                    SIM,
                    "seconds, above 0 (the event clock's ceiling)",
                ),
            ),
            ("intro", f(text(8, &["spawn", "heli"]), SIM, "")),
            ("midground", f(id(Kind::Track), P, "a disc track (ssx:track/<name>) whose mid-ground a new track shows")),
        ],
    );
    generated("map.toml", &["map"], vec![("map", f(FieldType::Table(Box::new(table)), SIM, ""))])
}

pub fn anim() -> RecordSchema {
    use crate::anim::{
        MAX_DURATION_S, MAX_FLOAT_TRACKS, MAX_KEYS, MAX_MAGNITUDE, MAX_NAME_BYTES, RawAnimTable, RawFloat,
    };
    let number = |min: f64, max: f64| s(Scalar::Float { min, max, f32: true });
    let list =
        |elem: FieldType, max: usize, ascending: bool| FieldType::List { elem: Box::new(elem), min: 1, max, ascending };
    let float = generated(
        "float",
        fields_of::<RawFloat>(),
        vec![
            (
                "dof",
                f(text(MAX_NAME_BYTES, &[]), SIM, "a float DOF of the clip's layout (genchar_rig AnimRig / FacePoser)"),
            ),
            ("interpolation", f(text(8, &["step", "linear", "cubic"]), SIM, "linear when left out")),
            (
                "times",
                f(list(number(0.0, f64::from(MAX_DURATION_S)), MAX_KEYS, true), SIM, "seconds, strictly increasing"),
            ),
            (
                "values",
                f(
                    list(number(-4.0 * f64::from(MAX_MAGNITUDE), 4.0 * f64::from(MAX_MAGNITUDE)), MAX_KEYS * 3, false),
                    SIM,
                    "one a key (three a cubic key: in-tangent, value, out-tangent); a value within ±4096, a tangent ±16384",
                ),
            ),
        ],
    );
    let table = generated(
        "anim",
        fields_of::<RawAnimTable>(),
        vec![
            (
                "gltf",
                f(
                    text(crate::path::MAX_PATH_LEN, &[]),
                    SIM,
                    "a package path to a .glb or .gltf file: the joint tracks",
                ),
            ),
            (
                "animation",
                f(text(MAX_NAME_BYTES, &[]), SIM, "the glTF animation by name (the file's only one when left out)"),
            ),
            ("additive", f(s(Scalar::Bool), SIM, "the keys are offsets; equal to the replaced clip's (Anim+0x22)")),
            (
                "float",
                f(
                    FieldType::List {
                        elem: Box::new(FieldType::Table(Box::new(float))),
                        min: 0,
                        max: MAX_FLOAT_TRACKS,
                        ascending: false,
                    },
                    SIM,
                    "a DOF at most once",
                ),
            ),
        ],
    );
    generated("anim.toml", &["anim"], vec![("anim", f(FieldType::Table(Box::new(table)), SIM, ""))])
}

pub fn trick() -> RecordSchema {
    use crate::trick::{BASES, INPUTS, MAX_POINTS, MIN_POINTS, ROLES, RawClips, RawTrickTable};
    let bases: Vec<&str> = BASES.iter().map(|b| b.0).collect();
    let inputs: Vec<&str> = INPUTS.iter().map(|i| i.name()).collect();
    let clips = generated(
        "clips",
        fields_of::<RawClips>(),
        ROLES
            .iter()
            .map(|(r, suffix)| {
                (*r, f(id(Kind::Anim), SIM, &format!("the base's clip place `1_trick_air_<base>{suffix}`")))
            })
            .collect(),
    );
    let table = generated(
        "trick",
        fields_of::<RawTrickTable>(),
        vec![
            ("base", f(text(16, &bases), SIM, "the disc grab whose sequences, windows and blends it plays in")),
            ("input", f(text(16, &inputs), SIM, "an air input the disc leaves free; the goofy stance's is its mirror")),
            ("hand", f(text(8, &["rear", "front", "both"]), SIM, "GSENUM_TrickHand (the base's when left out)")),
            (
                "points",
                f(int(i64::from(MIN_POINTS), i64::from(MAX_POINTS)), SIM, "TRICK_POINTS (the disc's largest: 2400)"),
            ),
            ("name", f(id(Kind::String), P, "the HUD's name")),
            ("name_uber", f(id(Kind::String), P, "the name while Tricky")),
            ("name_super_uber", f(id(Kind::String), P, "the name while Super Tricky")),
            ("clips", f(FieldType::Table(Box::new(clips)), SIM, "a clip of the package for each of the 12 places")),
        ],
    );
    generated("trick.toml", &["trick"], vec![("trick", f(FieldType::Table(Box::new(table)), SIM, ""))])
}

pub fn gamemode() -> RecordSchema {
    use crate::gamemode::{EVENTS, MAX_MAPS, MAX_TIME_LIMIT_S, MAX_TITLE_CHARS, RawGamemodeTable, RawMapEntry};
    let events: Vec<&str> = EVENTS.iter().map(|e| e.0).collect();
    let limit = || s(Scalar::Float { min: 0.0, max: MAX_TIME_LIMIT_S, f32: false });
    let target = s(Scalar::Float { min: 0.0, max: f64::from(u32::MAX), f32: false });
    let entry = generated(
        "map",
        fields_of::<RawMapEntry>(),
        vec![
            ("id", f(id(Kind::Map), SIM, "a map the gamemode takes")),
            (
                "targets",
                f(
                    FieldType::List { elem: Box::new(target), min: 3, max: 3, ascending: false },
                    SIM,
                    "gold, silver, bronze, best first: a race's seconds, a trick's points, a survive's metres",
                ),
            ),
            ("time_limit_s", f(limit(), SIM, "this map's, seconds above 0 (the event clock's ceiling)")),
        ],
    );
    let table = generated(
        "gamemode",
        fields_of::<RawGamemodeTable>(),
        vec![
            ("title", f(text(MAX_TITLE_CHARS * 4, &[]), P, "1 … 64 characters")),
            ("event", f(text(17, &events), SIM, "UI_EventType (the disc's nine event types)")),
            (
                "time_up",
                f(text(6, &["fail", "result"]), SIM, "ScoreManager_EndEventOnDeathTimer 0x824CE208's two ends"),
            ),
            ("time_limit_s", f(limit(), SIM, "every map's that states none, seconds above 0")),
            ("any_map", f(s(Scalar::Bool), SIM, "it takes any map with a finish (free ride: any), without targets")),
            (
                "map",
                f(
                    FieldType::List {
                        elem: Box::new(FieldType::Table(Box::new(entry))),
                        min: 0,
                        max: MAX_MAPS,
                        ascending: false,
                    },
                    SIM,
                    "",
                ),
            ),
        ],
    );
    generated("gamemode.toml", &["gamemode"], vec![("gamemode", f(FieldType::Table(Box::new(table)), SIM, ""))])
}

#[cfg(test)]
mod tests;
