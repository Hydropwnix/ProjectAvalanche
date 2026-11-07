use super::*;

fn spec() -> InstanceSpec {
    InstanceSpec::new("exp_alps_vorassay_drop_3", Mode::Freeride, Rewinds::Limit(3), true, GhostStyle::Hologram)
        .expect("a valid spec")
}

#[test]
fn round_trips_and_every_field_value() {
    for rewinds in [Rewinds::On, Rewinds::Off, Rewinds::Limit(1), Rewinds::Limit(MAX_REWIND_LIMIT)] {
        for ghosts in [false, true] {
            for style in [GhostStyle::Character, GhostStyle::Hologram] {
                for mode in Mode::ALL {
                    let s = InstanceSpec::new("chapter_2_drop_h2h", mode, rewinds, ghosts, style).unwrap();
                    assert_eq!(InstanceSpec::decode(&s.encode()), Ok(s.clone()));
                    assert!(s.encode().len() <= MAX_SPEC_SIZE);
                }
            }
        }
    }
    let good = spec().encode();
    for field in 0..5 {
        for v in 0..=255u8 {
            let mut b = good.clone();
            b[field] = v;
            let ok = match field {
                0 => (1..=4).contains(&v),
                1 => v == 0 || v == 255 || (1..=MAX_REWIND_LIMIT).contains(&v),
                2 => v <= 1,
                3 => v <= 1,
                _ => v == 1,
            };
            assert_eq!(InstanceSpec::decode(&b).is_ok(), ok, "byte {field} = {v}");
        }
    }
    let mut long = good.clone();
    long[5] += 1;
    assert!(InstanceSpec::decode(&long).is_err());
    let mut extra = good.clone();
    extra.push(b'x');
    assert!(InstanceSpec::decode(&extra).is_err());
    for n in 0..good.len() {
        assert!(InstanceSpec::decode(&good[..n]).is_err(), "{n} bytes");
    }
}

#[test]
fn drop_names_are_the_discs_spelling() {
    for ok in ["a", "exp_alps_vorassay_drop_3", "chapter_11_drop_h2h", &"z".repeat(MAX_DROP_NAME)] {
        assert_eq!(drop_name(ok), Ok(ok));
    }
    for bad in ["", "Exp_alps", "exp alps", "exp-alps", "../x", "exp\u{0}", "é", &"z".repeat(MAX_DROP_NAME + 1)] {
        assert!(drop_name(bad).is_err(), "{bad:?}");
        let mut b = spec().encode();
        b.truncate(5);
        b.push(bad.len().min(255) as u8);
        b.extend_from_slice(&bad.as_bytes()[..bad.len().min(255)]);
        assert!(InstanceSpec::decode(&b).is_err(), "{bad:?} in a spec");
    }
    assert!(InstanceSpec::new("x", Mode::Freeride, Rewinds::Limit(0), false, GhostStyle::Character).is_err());
}

#[test]
fn appearance_gear_bits_are_defined_or_refused() {
    for gear in 0..=u16::MAX {
        let a = Appearance::new(7, gear);
        assert_eq!(a.is_ok(), gear & !Appearance::GEAR == 0, "{gear:#06x}");
        if let Ok(a) = a {
            assert_eq!(Appearance::decode(&a.encode()), Ok(a));
        }
    }
    for n in [0, 5, 7] {
        assert!(Appearance::decode(&vec![0; n]).is_err());
    }
}

#[test]
fn hostile_bytes_never_panic() {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for len in 0..80usize {
        for _ in 0..5_000 {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    x as u8
                })
                .collect();
            if let Ok(s) = InstanceSpec::decode(&bytes) {
                assert_eq!(s.encode(), bytes);
            }
        }
    }
}

#[test]
fn rewinds_read_back_as_info_spells_them() {
    for r in [Rewinds::On, Rewinds::Off, Rewinds::Limit(1), Rewinds::Limit(3), Rewinds::Limit(MAX_REWIND_LIMIT)] {
        assert_eq!(Rewinds::parse(&r.name()), Some(r));
    }
    for bad in ["", "0", "00", "03", "100", "-1", "+3", " 3", "On", "1e1", "255", "٣"] {
        assert_eq!(Rewinds::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn modes_states_and_statuses_read_only_their_values() {
    for m in Mode::ALL {
        assert_eq!(Mode::parse(m.name()), Some(m));
        assert_eq!(Mode::from_byte(m as u8), Some(m));
        assert_eq!(m.has_rounds(), m != Mode::Freeride);
    }
    for bad in ["", "Race", "races", "free ride", "survival"] {
        assert_eq!(Mode::parse(bad), None, "{bad:?}");
    }
    for v in 0..=255u8 {
        assert_eq!(Mode::from_byte(v).is_some(), (1..=4).contains(&v), "mode {v}");
        assert_eq!(InstanceState::from_byte(v).is_some(), (1..=5).contains(&v), "state {v}");
        assert_eq!(PlayerStatus::from_byte(v).is_some(), (1..=7).contains(&v), "status {v}");
    }
    for s in InstanceState::ALL {
        assert_eq!(InstanceState::from_byte(s as u8), Some(s));
    }
}

fn map_spec(mode: Mode, event_type: u8) -> InstanceSpec {
    let level = Level::map("slope:map/course", event_type).expect("a map");
    InstanceSpec::on_level(level, mode, Rewinds::On, false, GhostStyle::Character).expect("a valid spec")
}

#[test]
fn a_map_round_trips_with_its_event_type_of_the_modes_kind() {
    for mode in Mode::ALL {
        for t in 0..=255u8 {
            let fits = match mode {
                Mode::Freeride => t == 8,
                Mode::Race => (5..=7).contains(&t),
                Mode::Trick => t <= 2,
                Mode::Survive => (3..=4).contains(&t),
            };
            let level = Level::map("slope:map/course", t);
            assert_eq!(level.is_ok(), t <= MAX_EVENT_TYPE, "{t}");
            let spec = level.and_then(|l| InstanceSpec::on_level(l, mode, Rewinds::Off, true, GhostStyle::Hologram));
            assert_eq!(spec.is_ok(), fits, "{mode:?} {t}");
            if let Ok(s) = spec {
                assert_eq!(InstanceSpec::decode(&s.encode()), Ok(s.clone()));
                assert_eq!((s.drop(), s.level.name()), (None, "slope:map/course"));
            }
        }
    }
    let id = format!("a:map/{}", "b/".repeat(124) + "cd");
    assert_eq!(id.len(), MAX_MAP_ID);
    let gamemode = format!("a:gamemode/{}", "c".repeat(MAX_MAP_ID - 11));
    let level = Level::map_under(&id, 5, Some(&gamemode)).unwrap();
    let s = InstanceSpec::on_level(level, Mode::Race, Rewinds::On, false, GhostStyle::Character).unwrap();
    assert_eq!(s.encode().len(), MAX_SPEC_SIZE);
    assert_eq!(InstanceSpec::decode(&s.encode()), Ok(s));
    let good = map_spec(Mode::Race, 5).encode();
    assert_eq!(&good[4..8], &[2, 5, 0, 16]);
    assert_eq!(&good[24..], &[0, 0]);
    for n in 0..good.len() {
        assert!(InstanceSpec::decode(&good[..n]).is_err(), "{n} bytes");
    }
    for (at, v) in [(5, 9u8), (5, 8), (6, 1), (7, 15), (7, 17), (25, 1), (24, 1)] {
        let mut b = good.clone();
        b[at] = v;
        assert!(InstanceSpec::decode(&b).is_err(), "byte {at} = {v}");
    }
    let mut extra = good.clone();
    extra.push(b'x');
    assert!(InstanceSpec::decode(&extra).is_err());
}

#[test]
fn map_ids_are_the_content_registrys_spelling() {
    let long = format!("a:map/{}", "c".repeat(MAX_MAP_ID - 6));
    for ok in ["slope:map/course", "a:map/b", "x-y.z_0:map/a/b.c/d-e", long.as_str()] {
        assert_eq!(map_id(ok), Ok(ok), "{ok}");
    }
    let longer = format!("{long}c");
    for bad in [
        "",
        "slope",
        "slope:course",
        "slope:object/course",
        "slope:map/",
        "slope:map//x",
        "slope:map/x/",
        "slope:map/./x",
        "slope:map/..",
        "Slope:map/x",
        "slope:Map/x",
        "slope:map/X",
        ":map/x",
        "a b:map/x",
        "a:map/x y",
        "a:map/é",
        "a:map/x\u{0}",
        "a:b:map/x",
        "..:map/x",
        longer.as_str(),
    ] {
        assert!(map_id(bad).is_err(), "{bad:?}");
        assert!(Level::map(bad, 5).is_err(), "{bad:?}");
        let mut b = map_spec(Mode::Race, 5).encode();
        b.truncate(6);
        b.extend_from_slice(&(bad.len() as u16).to_be_bytes());
        b.extend_from_slice(bad.as_bytes());
        b.extend_from_slice(bad.as_bytes());
        b.extend_from_slice(&[0, 0]);
        assert!(InstanceSpec::decode(&b).is_err(), "{bad:?} in a spec");
    }
}

#[test]
fn a_maps_gamemode_is_its_content_id_or_none() {
    let g = "gully:gamemode/race";
    let level = Level::map_under("gully:map/course", 6, Some(g)).expect("a gamemode");
    let s = InstanceSpec::on_level(level.clone(), Mode::Race, Rewinds::On, true, GhostStyle::Hologram).unwrap();
    let bytes = s.encode();
    assert_eq!(&bytes[24..], [&[0u8, g.len() as u8][..], g.as_bytes()].concat());
    assert_eq!(InstanceSpec::decode(&bytes), Ok(s));
    assert_eq!(level.to_string(), "map gully:map/course (gully:gamemode/race)");
    for bad in ["gully:map/race", "gully:gamemode/", "gully:gamemodes/x", "Gully:gamemode/x", "gully:gamemode/../x"] {
        assert!(gamemode_id(bad).is_err(), "{bad}");
        assert!(Level::map_under("gully:map/course", 6, Some(bad)).is_err(), "{bad}");
        let mut b = map_spec(Mode::Race, 5).encode();
        b.truncate(24);
        b.extend_from_slice(&(bad.len() as u16).to_be_bytes());
        b.extend_from_slice(bad.as_bytes());
        assert!(InstanceSpec::decode(&b).is_err(), "{bad} in a spec");
    }
    let mut b = map_spec(Mode::Race, 5).encode();
    b.truncate(24);
    b.extend_from_slice(&[0, 9, b'a', b':']);
    assert!(InstanceSpec::decode(&b).is_err());
    let mut b = map_spec(Mode::Race, 5).encode();
    b.truncate(24);
    b.extend_from_slice(&[0, 2, 0xC3, 0x28]);
    assert!(InstanceSpec::decode(&b).is_err());
}
