use std::collections::BTreeSet;

use mlua::MultiValue;

use super::*;

#[test]
fn the_committed_definitions_are_the_generated_ones() {
    for side in [Side::Server, Side::Client] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("defs").join(defs::file_name(side));
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == defs::generate(side, API_LEVEL),
            "{} differs from the registry: run `cargo run -p ssx-script --example api_defs`",
            path.display()
        );
    }
}

fn without_literals(text: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        match (quoted, escaped, c) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (true, false, '"') => quoted = false,
            (true, false, _) => {}
            (false, _, '"') => quoted = true,
            (false, _, c) => out.push(c),
        }
    }
    out
}

#[test]
fn every_type_the_definitions_name_is_declared() {
    let declared: BTreeSet<&str> = ["nil", "boolean", "number", "string", "any", "NetValue"]
        .into_iter()
        .chain(defs::HANDLE_TYPES)
        .chain(RECORDS.iter().map(|(n, _)| *n))
        .collect();
    let mut named = BTreeSet::new();
    let mut add = |t: &Ty| {
        let text = without_literals(&defs::type_text(t));
        for word in text.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
            named.insert(word.to_string());
        }
    };
    for s in registry() {
        s.params.iter().for_each(|p| add(&p.ty));
        s.returns.iter().for_each(&mut add);
        s.rest.iter().for_each(&mut add);
    }
    for (_, fields) in RECORDS {
        fields.iter().for_each(|(_, t)| add(t));
    }
    let unknown: Vec<&String> = named.iter().filter(|n| !declared.contains(n.as_str())).collect();
    assert!(unknown.is_empty(), "types named but not declared: {unknown:?}");
}

#[test]
fn names_are_unique_and_every_spec_has_a_side() {
    let mut seen = BTreeSet::new();
    for s in registry() {
        assert!(seen.insert((s.name, s.sides == Sides::Client)), "{} twice", s.name);
        assert!(s.api >= 1 && s.api <= API_LEVEL, "{}", s.name);
    }
}

fn valid(lua: &Lua, ty: &Ty) -> Value {
    match ty {
        Ty::Bool => Value::Boolean(true),
        Ty::Number { min, .. } => Value::Number(*min),
        Ty::Int { min, .. } => Value::Number(*min as f64),
        Ty::Name(NameKind::ContentId) => Value::String(lua.create_string("ava:marker/ring").unwrap()),
        Ty::Str { .. } | Ty::Name(_) => Value::String(lua.create_string("a").unwrap()),
        Ty::Enum(options) => Value::String(lua.create_string(options[0]).unwrap()),
        Ty::Element | Ty::Elements => {
            Value::UserData(lua.create_userdata(lua::ElementRef(crate::value::ElementId(1))).unwrap())
        }
        Ty::Function => Value::Function(lua.create_function(|_, ()| Ok(())).unwrap()),
        Ty::Timer => Value::UserData(lua.create_userdata(lua::TimerRef(1)).unwrap()),
        Ty::Latent => Value::UserData(lua.create_userdata(lua::LatentRef(1)).unwrap()),
        Ty::Vec3 { .. } => {
            let t = lua.create_table().unwrap();
            for k in ["x", "y", "z"] {
                t.raw_set(k, 1.0).unwrap();
            }
            Value::Table(t)
        }
        Ty::Record(_) => Value::Table(lua.create_table().unwrap()),
        _ => Value::Nil,
    }
}

#[test]
fn the_checker_refuses_extra_arguments_wrong_types_and_ranges() {
    let lua = Lua::new();
    for s in registry() {
        if s.rest.is_none() {
            let mut args: Vec<Value> = s.params.iter().map(|p| valid(&lua, &p.ty)).collect();
            args.push(Value::Boolean(true));
            let e = check_args(s, args.into_iter().collect()).err().unwrap_or_default();
            assert!(e.contains("arguments expected"), "{}: {e}", s.name);
        }
        let good: Vec<Value> = s.params.iter().map(|p| valid(&lua, &p.ty)).collect();
        assert!(check_args(s, good.iter().cloned().collect()).is_ok(), "{}: a valid call is refused", s.name);
        for (i, p) in s.params.iter().enumerate() {
            let wrong = match p.ty {
                Ty::Opt(_) | Ty::Value | Ty::Any => continue,
                Ty::Str { .. } | Ty::Name(_) | Ty::Enum(_) => Value::Boolean(true),
                _ => Value::String(lua.create_string("x").unwrap()),
            };
            let mut args = good.clone();
            args[i] = wrong;
            let e = check_args(s, args.into_iter().collect()).err().unwrap_or_default();
            assert!(e.contains(&format!("bad argument #{}", i + 1)), "{} #{}: {e}", s.name, i + 1);
        }
    }
    let timer = registry().find(|s| s.name == "setTimer").unwrap();
    let f = lua.create_function(|_, ()| Ok(())).unwrap();
    let call = |interval: f64, times: f64| {
        let args: MultiValue =
            [Value::Function(f.clone()), Value::Number(interval), Value::Number(times)].into_iter().collect();
        check_args(timer, args).err().unwrap_or_default()
    };
    assert!(call(f64::NAN, 1.0).contains("bad argument #2"));
    assert!(call(1e12, 1.0).contains("bad argument #2"));
    assert!(call(100.0, 1.5).contains("whole number"));
    assert!(call(100.0, -1.0).contains("bad argument #3"));
    assert!(call(100.0, 1.0).is_empty());
    let name = registry().find(|s| s.name == "addEvent").unwrap();
    let long = Value::String(lua.create_string("e".repeat(200)).unwrap());
    assert!(check_args(name, [long].into_iter().collect()).err().unwrap_or_default().contains("at most 128 bytes"));
    let ctrl = Value::String(lua.create_string("a\u{1b}b").unwrap());
    assert!(check_args(name, [ctrl].into_iter().collect()).err().unwrap_or_default().contains("made of"));
}

#[test]
fn a_vm_gets_exactly_the_registrys_functions() {
    for side in [Side::Server, Side::Client] {
        let host = crate::host::ScriptHost::new(crate::host::HostConfig::new(side)).unwrap();
        let path = if side == Side::Server { "server/a.luau" } else { "client/a.luau" };
        let scripts = ssx_content::manifest::Scripts {
            client: vec!["client/*.luau".into()],
            server: vec!["server/*.luau".into()],
            shared: vec![],
        };
        let src = "local out = {} for k, v in pairs(_G) do if type(v) == 'function' then out[#out + 1] = k \
                   elseif type(v) == 'table' and (k == 'json' or k == 'storage' or k == 'draw') then for m in pairs(v) do out[#out + 1] = k .. '.' .. m end end end \
                   table.sort(out) print(table.concat(out, ' '))";
        let files = [(path.to_string(), src.to_string())];
        let pkg = crate::host::package::PackageScripts::from_memory(
            "probe",
            side,
            &scripts,
            &ssx_content::manifest::Files::default(),
            toml::Table::new(),
            &files,
            host.limits(),
        )
        .unwrap();
        host.start(pkg).unwrap();
        let line = host.take_output("probe").unwrap().lines.remove(0);
        let got: BTreeSet<&str> = line.split(' ').collect();
        let base = crate::ScriptVm::new(crate::FileMap::new(), crate::Limits::CLIENT).unwrap();
        let plain: String = base
            .eval("local out = {} for k, v in pairs(_G) do if type(v) == 'function' then out[#out + 1] = k end end table.sort(out) return table.concat(out, ' ')")
            .unwrap();
        let plain: BTreeSet<&str> = plain.split(' ').collect();
        let api: BTreeSet<&str> = got.difference(&plain).copied().collect();
        let want: BTreeSet<&str> = specs(side, API_LEVEL).map(|s| s.name).collect();
        assert_eq!(api, want, "{side:?}");
    }
}
