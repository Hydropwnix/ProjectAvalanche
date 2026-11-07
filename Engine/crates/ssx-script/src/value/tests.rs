use mlua::{Lua, Value};

use super::*;

fn s(t: &str) -> NetValue {
    NetValue::String(t.as_bytes().to_vec())
}

fn table(entries: Vec<(NetValue, NetValue)>) -> NetValue {
    NetValue::Table(entries)
}

fn nested(depth: usize) -> NetValue {
    let mut v = NetValue::Number(1.0);
    for _ in 0..depth {
        v = table(vec![(NetValue::Number(1.0), v)]);
    }
    v
}

fn sorted(v: &NetValue) -> NetValue {
    match v {
        NetValue::Table(t) => {
            let mut t: Vec<(NetValue, NetValue)> = t.iter().map(|(k, v)| (sorted(k), sorted(v))).collect();
            t.sort_by_key(|(k, _)| format!("{k:?}"));
            NetValue::Table(t)
        }
        other => other.clone(),
    }
}

fn lua_value(lua: &Lua, src: &str) -> Value {
    lua.load(src).eval().unwrap()
}

#[test]
fn values_round_trip_byte_for_byte() {
    let l = ValueLimits::REMOTE;
    let v = table(vec![
        (s("a"), NetValue::Bool(true)),
        (NetValue::Number(1.0), NetValue::Number(-0.0)),
        (NetValue::Number(2.5), NetValue::Number(f64::INFINITY)),
        (NetValue::Bool(false), NetValue::Element(ElementId(7))),
        (s("nan"), NetValue::Number(f64::NAN)),
        (s("t"), nested(3)),
    ]);
    let bytes = encode(&v, &l).unwrap();
    assert_eq!(bytes.len(), v.encoded_len());
    let back = decode(&bytes, &l).unwrap();
    assert_eq!(encode(&back, &l).unwrap(), bytes);
    let list = vec![NetValue::Nil, s("x"), NetValue::Number(3.0)];
    assert_eq!(decode_list(&encode_list(&list, &l).unwrap(), &l).unwrap(), list);
}

#[test]
fn depth_sixteen_is_carried_and_seventeen_refused() {
    let l = ValueLimits::REMOTE;
    assert!(encode(&nested(16), &l).is_ok());
    assert_eq!(encode(&nested(17), &l), Err(ValueError::TooDeep(16)));
    let mut bytes = Vec::new();
    for _ in 0..17 {
        bytes.extend([5, 1, 3]);
        bytes.extend(1.0f64.to_bits().to_le_bytes());
    }
    bytes.extend([3]);
    bytes.extend(1.0f64.to_bits().to_le_bytes());
    assert_eq!(decode(&bytes, &l), Err(ValueError::TooDeep(16)));
}

#[test]
fn strings_over_64_kib_are_refused() {
    let l = ValueLimits::LATENT;
    let ok = NetValue::String(vec![b'x'; MAX_STRING]);
    let over = NetValue::String(vec![b'x'; MAX_STRING + 1]);
    assert!(encode(&ok, &l).is_ok());
    assert_eq!(encode(&over, &l), Err(ValueError::StringTooLong { len: MAX_STRING + 1, max: MAX_STRING }));
    let mut bytes = vec![4, 0x81, 0x80, 0x04];
    bytes.extend(vec![b'x'; MAX_STRING + 1]);
    assert!(matches!(decode(&bytes, &l), Err(ValueError::StringTooLong { .. })));
}

#[test]
fn only_the_canonical_encoding_decodes() {
    let l = ValueLimits::REMOTE;
    let num = |bits: u64| [vec![3], bits.to_le_bytes().to_vec()].concat();
    assert!(decode(&num(CANONICAL_NAN), &l).is_ok());
    assert!(matches!(decode(&num(0x7FF8_0000_0000_0001), &l), Err(ValueError::NonCanonical(_))));
    assert!(matches!(decode(&num(0xFFF8_0000_0000_0000), &l), Err(ValueError::NonCanonical(_))));
    let odd = NetValue::Number(f64::from_bits(0x7FF0_0000_0000_0042));
    assert_eq!(encode(&odd, &l).unwrap(), num(CANONICAL_NAN));
    assert!(matches!(decode(&[4, 0x80, 0x00], &l), Err(ValueError::NonCanonical(_))), "a needless length byte");
    assert!(matches!(decode(&[4, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F], &l), Err(ValueError::NonCanonical(_))));
    let neg_zero_key = [vec![5, 1], num((-0.0f64).to_bits()), vec![2]].concat();
    assert!(matches!(decode(&neg_zero_key, &l), Err(ValueError::NonCanonical(_))));
    let nil_value = [vec![5, 1], num(1.0f64.to_bits()), vec![0]].concat();
    assert!(matches!(decode(&nil_value, &l), Err(ValueError::NonCanonical(_))));
    let dup = [vec![5, 2], num(1.0f64.to_bits()), vec![2], num(1.0f64.to_bits()), vec![1]].concat();
    assert_eq!(decode(&dup, &l), Err(ValueError::DuplicateKey));
    let elem = [vec![6], (1u64 << 53).to_le_bytes().to_vec()].concat();
    assert!(matches!(decode(&elem, &l), Err(ValueError::NonCanonical(_))));
    assert_eq!(decode(&[0, 0], &l), Err(ValueError::Trailing(1)));
    assert_eq!(decode(&[7], &l), Err(ValueError::BadTag(7)));
    assert_eq!(decode(&[], &l), Err(ValueError::Truncated));
}

#[test]
fn counts_are_checked_against_the_bytes_left_before_anything_is_kept() {
    let l = ValueLimits::LATENT;
    assert_eq!(decode(&[5, 0xFF, 0xFF, 0xFF, 0xFF, 0x0F], &l), Err(ValueError::Truncated));
    assert_eq!(decode(&[4, 0x80, 0x80, 0x04], &l), Err(ValueError::Truncated));
    assert_eq!(decode_list(&[0x81, 0x02], &l), Err(ValueError::TooMany), "257 values");
    assert_eq!(decode_list(&[3, 0, 0], &l), Err(ValueError::Truncated));
    let big = vec![0u8; ValueLimits::REMOTE.max_bytes + 1];
    assert_eq!(decode(&big, &ValueLimits::REMOTE), Err(ValueError::TooLarge(64 << 10)));
}

#[test]
fn script_values_are_copied_with_their_caps() {
    let lua = Lua::new();
    let l = ValueLimits::REMOTE;
    let v = from_lua(&lua_value(&lua, "return {1, 2, x = 'y', [true] = {z = -0}}"), &l).unwrap();
    let back = to_lua(&lua, &v, &|_, _| Ok(Value::Nil)).unwrap();
    assert_eq!(sorted(&from_lua(&back, &l).unwrap()), sorted(&v));
    let cycle = lua_value(&lua, "local t = {} t.self = t return t");
    assert_eq!(from_lua(&cycle, &l), Err(ValueError::Cycle));
    let meta = lua_value(&lua, "return setmetatable({}, {})");
    assert_eq!(from_lua(&meta, &l), Err(ValueError::Metatable));
    for (src, what) in [
        ("return function() end", "function"),
        ("return coroutine.create(function() end)", "thread"),
        ("return buffer.create(4)", "buffer"),
        ("return {f = print}", "function"),
    ] {
        assert_eq!(from_lua(&lua_value(&lua, src), &l), Err(ValueError::NotAValue(what)), "{src}");
    }
    let bad_key = lua_value(&lua, "return {[{}] = 1}");
    assert_eq!(from_lua(&bad_key, &l), Err(ValueError::BadKey("table")));
    let deep = lua_value(&lua, &format!("return {}1{}", "{".repeat(17), "}".repeat(17)));
    assert_eq!(from_lua(&deep, &l), Err(ValueError::TooDeep(16)));
    let long = lua_value(&lua, "return string.rep('x', 65537)");
    assert!(matches!(from_lua(&long, &l), Err(ValueError::StringTooLong { .. })));
}

#[test]
fn a_shared_subtable_is_counted_each_time_it_appears() {
    let lua = Lua::new();
    let src = "local t = {string.rep('x', 4096)} for i = 1, 12 do t = {t, t} end return t";
    let v = lua_value(&lua, src);
    assert_eq!(from_lua(&v, &ValueLimits::LOCAL), Err(ValueError::TooLarge(1 << 20)));
    let small = lua_value(&lua, "local t = {1} return {t, t}");
    assert_eq!(from_lua(&small, &ValueLimits::LOCAL).unwrap().encoded_len(), 1 + 1 + 2 * (9 + 1 + 1 + 9 + 9));
}

#[test]
fn the_byte_cap_matches_the_encoding() {
    let lua = Lua::new();
    let l = ValueLimits { max_bytes: 100, ..ValueLimits::REMOTE };
    for n in 0..120 {
        let v = lua_value(&lua, &format!("return string.rep('x', {n})"));
        let copied = from_lua(&v, &l);
        let expect = 1 + super::codec::varint_len(n) + n;
        assert_eq!(copied.is_ok(), expect <= 100, "{n}");
        if let Ok(c) = copied {
            assert_eq!(encode(&c, &l).unwrap().len(), expect);
        }
    }
}

#[test]
fn json_both_ways() {
    let v = json::decode(br#"{"a": [1, 2.5, null, "x"], "b": {"c": true}, "d": null}"#).unwrap();
    assert_eq!(v.field("d"), None);
    let NetValue::Table(a) = v.field("a").unwrap() else { panic!() };
    assert_eq!(a.len(), 3, "the null left a hole");
    let text =
        json::encode(&table(vec![(NetValue::Number(1.0), s("x")), (NetValue::Number(2.0), NetValue::Number(3.0))]))
            .unwrap();
    assert_eq!(text, r#"["x",3]"#);
    assert_eq!(json::encode(&table(vec![])).unwrap(), "{}");
    assert_eq!(json::encode(&NetValue::Number(1e300)).unwrap(), "1e300");
    assert_eq!(json::encode(&NetValue::Number(-7.0)).unwrap(), "-7");
    for bad in [NetValue::Number(f64::NAN), NetValue::Number(f64::INFINITY), NetValue::String(vec![0xFF])] {
        assert!(json::encode(&bad).is_err(), "{bad:?}");
    }
    assert!(json::encode(&table(vec![(NetValue::Number(1.0), s("x")), (s("k"), s("y"))])).is_err());
    assert!(json::decode(br#"{"a": 1, "a": 2}"#).unwrap_err().0.contains("equal keys"));
    assert!(
        json::decode(format!("{}1{}", "[".repeat(17), "]".repeat(17)).as_bytes()).unwrap_err().0.contains("deeper")
    );
    assert!(json::decode(format!("{}1{}", "[".repeat(16), "]".repeat(16)).as_bytes()).is_ok());
    assert!(json::decode(b"[1] x").is_err(), "trailing text");
    assert!(json::decode(&vec![b' '; json::MAX_JSON + 1]).is_err());
    let wide = format!("[{}]", vec!["\"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\""; 25_000].join(","));
    assert!(json::decode(wide.as_bytes()).unwrap_err().0.contains("over"), "the decoded size is capped");
    let big = NetValue::String(vec![b'"'; MAX_STRING]);
    let many = table((1..=16).map(|i| (NetValue::Number(f64::from(i)), big.clone())).collect());
    assert!(json::encode(&many).unwrap_err().0.contains("over"), "escaped quotes grow the text past its cap");
    let text = b"440040615010610040615010615016850168";
    let NetValue::Number(n) = json::decode(text).unwrap() else { panic!() };
    assert_eq!(n, "440040615010610040615010615016850168".parse::<f64>().unwrap());
    assert_eq!(json::decode(json::encode(&NetValue::Number(n)).unwrap().as_bytes()).unwrap(), NetValue::Number(n));
    for bad in [&b"[1,]"[..], b"{\"a\" 1}", b"01", b"1.", b".5", b"+1", b"\"\\ud800\"", b"\"a\x01\"", b"1e999", b"nul"]
    {
        assert!(json::decode(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
    }
    assert_eq!(json::decode(br#""\ud83d\ude00 \u00e9""#).unwrap(), s("\u{1F600} \u{e9}"));
    assert_eq!(json::encode(&s("a\"b\\c\n\u{1}")).unwrap(), r#""a\"b\\c\n\u0001""#);
}
