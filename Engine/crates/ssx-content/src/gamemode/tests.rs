use super::*;

fn map(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn problems(text: &str) -> Vec<String> {
    parse_toml(text.as_bytes()).expect_err("refused").into_iter().map(|p| p.2).collect()
}

#[test]
fn the_template_reads() {
    let t = parse_toml(template::gamemode_toml("me:map/gully").as_bytes()).unwrap();
    assert_eq!(t.event_type, EventType::RaceGhosts);
    assert_eq!(t.maps.len(), 1);
    assert_eq!(t.maps[0].targets, [13_000, 15_000, 20_000]);
    assert_eq!(t.maps[0].time_limit_ms, Some(90_000));
    let r = t.rules(&map("me:map/gully"), Some(30_000), true).unwrap();
    assert_eq!(r.time_limit_ms(), Some(90_000), "the gamemode's map entry over the map's own");
    assert_eq!(r.board_values(), [13_000, 15_000, 20_000]);
    assert_eq!(t.rules(&map("me:map/other"), None, true), Err(PairError::NotTaken));
    assert_eq!(t.rules(&map("me:map/gully"), None, false), Err(PairError::NoFinish));
}

#[test]
fn targets_in_the_types_unit() {
    let text = "[gamemode]\ntitle = \"T\"\nevent = \"trick_ghosts\"\n[[gamemode.map]]\nid = \"a:map/x\"\n\
                targets = [90000, 60000, 30000]\n";
    assert_eq!(parse_toml(text.as_bytes()).unwrap().maps[0].targets, [90_000, 60_000, 30_000]);
    let text = "[gamemode]\ntitle = \"S\"\nevent = \"survive_to_death\"\n[[gamemode.map]]\nid = \"a:map/x\"\n\
                targets = [3000.5, 2000, 1000]\n";
    assert_eq!(parse_toml(text.as_bytes()).unwrap().maps[0].targets, [3_000_500, 2_000_000, 1_000_000]);
    let p = problems(
        "[gamemode]\ntitle = \"T\"\nevent = \"trick_score\"\n[[gamemode.map]]\nid = \"a:map/x\"\n\
         targets = [1.5, 1, 0]\n",
    );
    assert!(p.iter().any(|m| m.contains("map.targets[0]: whole points")), "{p:?}");
    assert!(p.iter().any(|m| m.contains("map.targets[2]: above 0")), "{p:?}");
}

#[test]
fn hostile_records_name_their_field() {
    let p = problems("[gamemode]\nevent = \"tag\"\ntime_up = \"later\"\ntime_limit_s = 4000\n");
    for want in ["no title", "event: one of", "time_up: fail or result", "time_limit_s: above 0", "takes no map"] {
        assert!(p.iter().any(|m| m.contains(want)), "{want}: {p:?}");
    }
    let p = problems(
        "[gamemode]\ntitle = \"R\"\nevent = \"race_clock\"\ntime_up = \"result\"\n[[gamemode.map]]\n\
         id = \"a:trick/x\"\n[[gamemode.map]]\nid = \"a:map/y\"\ntargets = [60, 50, 70]\n[[gamemode.map]]\n\
         id = \"a:map/y\"\n",
    );
    for want in
        ["time_up: a race has no result", "map.id: a `map` id", "map.targets: target 2", "map.id: a:map/y twice"]
    {
        assert!(p.iter().any(|m| m.contains(want)), "{want}: {p:?}");
    }
    let p = problems(
        "[gamemode]\ntitle = \"F\"\nevent = \"freeride\"\nany_map = true\n[[gamemode.map]]\nid = \"a:map/x\"\ntargets = [3, 2, 1]\n",
    );
    assert!(p.iter().any(|m| m.contains("ranks no board")), "{p:?}");
    let p = problems("[gamemode]\ntitle = \"x\"\nevent = \"freeride\"\nany_map = true\nscripts = 1\n");
    assert!(p.iter().any(|m| m.starts_with("scripts:") || m.contains("unknown field")), "{p:?}");
    let many: String = (0..=MAX_MAPS).map(|k| format!("[[gamemode.map]]\nid = \"a:map/m{k}\"\n")).collect();
    let p = problems(&format!("[gamemode]\ntitle = \"x\"\nevent = \"race_clock\"\n{many}"));
    assert!(p.iter().any(|m| m.contains("at most 256")), "{p:?}");
    let p = problems("[gamemode]\ntitle = \"a\\u0007b\"\nevent = \"freeride\"\nany_map = true\n");
    assert!(p.iter().any(|m| m.contains("no control characters")), "{p:?}");
}

#[test]
fn any_map_takes_the_maps_limit() {
    let t =
        parse_toml(b"[gamemode]\ntitle = \"D\"\nevent = \"survive_to_death\"\nany_map = true\ntime_up = \"result\"\n")
            .unwrap();
    let r = t.rules(&map("b:map/z"), Some(120_000), true).unwrap();
    assert_eq!((r.time_limit_ms(), r.time_up()), (Some(120_000), TimeUp::Result));
    assert_eq!(r.board_values(), [2_000_000, 1_500_000, 1_000_000], "no targets: the padding (k + 2) × 500 000");
    let free = parse_toml(b"[gamemode]\ntitle = \"F\"\nevent = \"freeride\"\nany_map = true\n").unwrap();
    assert!(free.rules(&map("b:map/z"), None, false).is_ok(), "free ride needs no finish");
}

#[test]
fn the_sim_hash_is_the_rules() {
    let a = Rules::new(EventType::RaceGhosts, &[1, 2, 3], Some(60_000), None).unwrap();
    let b = Rules::new(EventType::RaceGhosts, &[1, 2, 4], Some(60_000), None).unwrap();
    let c = Rules::new(EventType::RaceClock, &[1, 2, 3], Some(60_000), None).unwrap();
    assert_ne!(sim_hash(&a), sim_hash(&b));
    assert_ne!(sim_hash(&a), sim_hash(&c));
    assert_eq!(sim_hash(&a), sim_hash(&a.clone()));
}

#[test]
fn a_maps_records_per_mode() {
    use crate::profile_state::{ModState, StateValue};
    let mut st = ModState::default();
    let m = map("a:map/x");
    let run = |value, failed, rewinds| RunOutcome { value, failed, rewinds };
    let t = EventType::RaceGhosts;
    assert_eq!(record::add_run(&mut st, &m, "race", t, run(20_000, false, 3), Some("aa")), Ok(true));
    assert_eq!(record::add_run(&mut st, &m, "race", t, run(25_000, false, 1), Some("bb")), Ok(false));
    assert_eq!(record::add_run(&mut st, &m, "b:gamemode/g", t, run(9_000, true, 0), None), Ok(false));
    let r = record::read(&st, &m, "race").unwrap();
    assert_eq!((r.best, r.attempts, r.deaths, r.fewest_rewinds), (Some(20_000), 2, 0, Some(1)));
    assert_eq!(record::best_sim(&st, &m, "race").as_deref(), Some("aa"));
    let g = record::read(&st, &m, "b:gamemode/g").unwrap();
    assert_eq!((g.best, g.attempts, g.deaths), (None, 1, 1), "a failed race keeps no best");
    st.set(&m, "m1_attempts", StateValue::Int(-5));
    st.set(&m, "m1_best", StateValue::Text("fast".into()));
    let r = record::read(&st, &m, "race").unwrap();
    assert_eq!((r.best, r.attempts), (None, 0));
    let mut full = ModState::default();
    for k in 0..record::MAX_SLOTS {
        assert!(record::add_run(&mut full, &m, &format!("mode{k}"), t, run(1, false, 0), None).is_ok());
    }
    let before = full.clone();
    assert_eq!(record::add_run(&mut full, &m, "one_more", t, run(1, false, 0), None), Err(record::RecordError::Full));
    assert_eq!(full, before, "nothing written");
}

#[test]
fn survive_to_death_takes_only_a_helicopter_start() {
    use crate::map::Intro;
    let death = parse_toml(b"[gamemode]\ntitle = \"D\"\nevent = \"survive_to_death\"\nany_map = true\n").unwrap();
    assert_eq!(death.takes_start(Intro::Spawn), Err(PairError::NoHelicopter));
    assert_eq!(death.takes_start(Intro::Heli), Ok(()));
    let race = parse_toml(b"[gamemode]\ntitle = \"R\"\nevent = \"race_clock\"\nany_map = true\n").unwrap();
    assert_eq!((race.takes_start(Intro::Spawn), race.takes_start(Intro::Heli)), (Ok(()), Ok(())));
}
