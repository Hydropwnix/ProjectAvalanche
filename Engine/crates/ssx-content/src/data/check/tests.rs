use super::*;
use crate::data::schema::{Class, FieldSchema};

fn field(ty: FieldType) -> FieldSchema {
    FieldSchema { ty, class: Class::Simulation, source: String::new(), note: String::new(), optional: false }
}

fn schema() -> RecordSchema {
    let mut point = RecordSchema::new("Point");
    let f = |min, max| FieldType::Scalar(Scalar::Float { min, max, f32: true });
    point.fields.insert("x".into(), field(f(-1.0, 1.0)));
    let mut s = RecordSchema::new("TestAsset");
    s.fields.insert("Speed".into(), field(f(0.25, 64.0)));
    s.fields.insert("Count".into(), field(FieldType::Scalar(Scalar::Int { min: 1, max: 8 })));
    s.fields.insert("Mode".into(), field(FieldType::Scalar(Scalar::IntOf(vec![0, 2]))));
    s.fields.insert("On".into(), field(FieldType::Scalar(Scalar::Bool)));
    s.fields.insert("Fixed".into(), field(FieldType::Scalar(Scalar::Fixed("a pointer"))));
    let curve = FieldType::Tuple(vec![f(0.0, 10.0), f(-5.0, 5.0)]);
    let list = FieldType::List { elem: Box::new(curve), min: 1, max: 4, ascending: true };
    s.fields.insert("Points".into(), field(list));
    s.fields.insert("At".into(), field(FieldType::Table(Box::new(point))));
    let guid = Scalar::Guid { classes: vec!["CurveAsset".into()], null: true, one_of: None };
    s.fields.insert("Curve".into(), field(FieldType::Scalar(guid)));
    s
}

fn problems(text: &str, mode: Mode) -> Vec<String> {
    match check_text(&schema(), text.as_bytes(), mode, None) {
        Ok(_) => Vec::new(),
        Err(p) => p.into_iter().map(|p| format!("{}:{} {} {}", p.line, p.column, p.path, p.message)).collect(),
    }
}

#[test]
fn a_good_patch_passes_and_a_whole_record_needs_every_field() {
    assert!(problems("Speed = 2\nPoints = [[0, 1], [1, 2]]\n[At]\nx = 0.5\n", Mode::Patch).is_empty());
    let p = problems("Speed = 2\n", Mode::Whole);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].contains("lacks 6 field(s)") && !p[0].contains("Fixed"), "{p:?}");
}

#[test]
fn every_problem_at_its_line_and_column() {
    let text = "Sped = 2\nCount = 9\nMode = 1\nOn = 1\n[At]\nx = nan\ny = 1\n[[Points]]\n";
    let p = problems(text, Mode::Patch);
    let want = [
        "1:1 Sped \"Sped\" is no field of TestAsset (did you mean \"Speed\"?)",
        "2:1 Count 9 is out of range: an integer 1 … 8",
        "3:1 Mode 1 is not one of [0, 2]",
        "4:1 On true or false expected, not an integer",
        "6:1 At.x NaN is not a finite number",
        "7:1 At.y \"y\" is no field of Point (did you mean \"x\"?)",
        "8:1 Points[0] 2 values expected",
    ];
    for w in want {
        assert!(p.iter().any(|x| x == w), "missing {w:?} in {p:#?}");
    }
}

#[test]
fn numbers_ranges_and_curves() {
    assert!(problems("Speed = inf\n", Mode::Patch)[0].contains("not a finite number"));
    assert!(problems("Speed = 0.2\n", Mode::Patch)[0].contains("out of range"));
    assert!(problems("Speed = 1e300\n", Mode::Patch)[0].contains("does not fit a 32-bit float"));
    assert!(problems("Speed = 64\n", Mode::Patch).is_empty(), "integers are numbers");
    assert!(problems("Points = [[1, 0], [0, 0]]\n", Mode::Patch)[0].contains("must not decrease"));
    assert!(problems("Points = []\n", Mode::Patch)[0].contains("0 items"));
    assert!(problems("Fixed = 1\n", Mode::Patch)[0].contains("not settable"));
}

#[test]
fn guids_need_the_bundle() {
    let s = schema();
    let curve = "01020304-0506-0708-090a-0b0c0d0e0f10";
    let text = format!("Curve = \"{curve}\"\n");
    let classes = |g: &[u8; 16]| (g[0] == 4).then(|| "CurveAsset".to_string());
    assert!(check_text(&s, text.as_bytes(), Mode::Patch, Some(&classes)).is_ok(), "first group is little-endian");
    let other = |_: &[u8; 16]| Some("PidAsset".to_string());
    let e = check_text(&s, text.as_bytes(), Mode::Patch, Some(&other)).unwrap_err();
    assert!(e[0].message.contains("is a PidAsset"), "{e:?}");
    let none = |_: &[u8; 16]| None;
    let e = check_text(&s, text.as_bytes(), Mode::Patch, Some(&none)).unwrap_err();
    assert!(e[0].message.contains("holds no asset"), "{e:?}");
    let null = "Curve = \"00000000-0000-0000-0000-000000000000\"\n";
    assert!(check_text(&s, null.as_bytes(), Mode::Patch, Some(&none)).is_ok());
    assert!(problems("Curve = \"0102\"\n", Mode::Patch)[0].contains("is no GUID"));
}

#[test]
fn references_keep_to_the_disc_s() {
    let mut s = schema();
    let mine = [4, 3, 2, 1, 6, 5, 8, 7, 9, 10, 11, 12, 13, 14, 15, 16];
    let guid = Scalar::Guid { classes: vec!["CurveAsset".into()], null: false, one_of: Some(vec![mine]) };
    s.fields.insert("Curve".into(), field(FieldType::Scalar(guid)));
    let curve = |g: &str| format!("Curve = \"{g}\"\n");
    let any = |_: &[u8; 16]| Some("CurveAsset".to_string());
    let ok = curve("01020304-0506-0708-090a-0b0c0d0e0f10");
    assert!(check_text(&s, ok.as_bytes(), Mode::Patch, Some(&any)).is_ok());
    let other = curve("01020304-0506-0708-090a-0b0c0d0e0f11");
    let e = check_text(&s, other.as_bytes(), Mode::Patch, Some(&any)).unwrap_err();
    assert!(e[0].message.contains("one of the 1 reference(s) the disc has here"), "{e:?}");
    assert!(check_text(&s, other.as_bytes(), Mode::Patch, None).is_err(), "refused without the bundle");
    let null = curve("00000000-0000-0000-0000-000000000000");
    assert!(check_text(&s, null.as_bytes(), Mode::Patch, None).is_err(), "no null where the disc has none");
    let only_null = Scalar::Guid { classes: Vec::new(), null: true, one_of: Some(Vec::new()) };
    s.fields.insert("Curve".into(), field(FieldType::Scalar(only_null)));
    assert!(check_text(&s, null.as_bytes(), Mode::Patch, None).is_ok());
    assert!(check_text(&s, ok.as_bytes(), Mode::Patch, None).is_err());
}

#[test]
fn hostile_records_are_refused_without_a_crash() {
    let deep = |open: &str, close: &str| format!("a = {}1{}\n", open.repeat(10_000), close.repeat(10_000));
    for text in [deep("[", "]"), deep("{b = ", "}")] {
        assert!(!problems(&text, Mode::Patch).is_empty());
    }
    let dotted = format!("{} = 1\n", vec!["a"; 10_000].join("."));
    assert!(!problems(&dotted, Mode::Patch).is_empty());
    let headers = format!("[{}]\n", vec!["a"; 10_000].join("."));
    assert!(!problems(&headers, Mode::Patch).is_empty());
    let long = format!("{} = 1\n", "k".repeat(1 << 20));
    let p = problems(&long, Mode::Patch);
    assert!(p[0].len() < 600 && p[0].contains("(1048576 bytes)"), "{}", &p[0][..600.min(p[0].len())]);
    let p = problems("\"a\\u0007b\" = 1\n", Mode::Patch);
    assert!(p[0].contains("a\\u{7}b") && !p[0].contains('\u{7}'), "{p:?}");
    let big = vec![b'#'; 64 << 20];
    let e = check_text(&schema(), &big, Mode::Patch, None).unwrap_err();
    assert!(e[0].message.contains("over"), "{e:?}");
    let many: String = (0..5000).map(|i| format!("f{i} = 1\n")).collect();
    let p = problems(&many, Mode::Patch);
    assert_eq!(p.len(), MAX_PROBLEMS + 1);
    assert!(p.last().unwrap().contains("4800 more problems"));
}
