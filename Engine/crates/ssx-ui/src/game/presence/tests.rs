use super::*;
use crate::game::fe::MapLaunch;
use crate::game::loc::Language;

fn loc() -> Localization {
    let mut loc = Localization::empty(Language::English);
    for (k, t) in crate::game::port::builtin_strings().unwrap() {
        loc.add_port_string(&k, &t).unwrap();
    }
    for (k, t) in [
        ("TXT_M_GLOBE_CAMPAIGN", "WORLD TOUR"),
        ("TXT_M_GLOBE_EXPLORE", "EXPLORE"),
        ("TXT_M_GLOBAL_EVENTS", "GLOBAL EVENTS"),
        ("TXT_EVENTTYPE_RACE_CLOCK", r##"<font color="#40C4F1">RACE IT!</font>"##),
        ("TXT_EVENTTYPE_RACE_BOTS", r##"<font color="#40C4F1">RACE IT!</font>"##),
        ("TXT_EVENTTYPE_TRICK_SCORE", r##"<font color="#FAE574">TRICK IT!</font>"##),
        ("TXT_EVENTTYPE_FREERIDE", "Freeride"),
        ("TXT_EVENTTYPE_TUTORIAL", r##"<font color="#D6FFF0">TRAINING</font>"##),
        ("TXT_YOUR_TIME_COLON", "Your time:"),
        ("TXT_YOUR_SCORE_COLON", "Your score:"),
        ("TXT_YOU_DESCENDED_COLON", "You descended:"),
        ("TXT_RESULTS_MEDAL_GOLD", "Gold"),
        ("TXT_RESULTS_MEDAL_BRONZE", "Bronze"),
        ("TXT_LE_RESULTS_ORDINAL_ST", "st"),
        ("TXT_LE_RESULTS_ORDINAL_ND", "nd"),
        ("TXT_LE_RESULTS_ORDINAL_RD", "rd"),
        ("TXT_LE_RESULTS_ORDINAL_TH", "th"),
    ] {
        loc.add_port_string(k, t).unwrap();
    }
    loc
}

fn names() -> RideNames {
    RideNames { title: "WORLD TOUR · RACE IT!".into(), place: "Corkscrew, Kilimanjaro".into() }
}

fn ride(part: Part, online: Option<Round>) -> Doing {
    Doing::Ride(Ride { launch: 3, run: 1, part, online })
}

#[test]
fn every_doing_has_its_lines() {
    let mut loc = loc();
    let race = Some(Round { mode: Some("race"), competing: true });
    let free = Some(Round { mode: Some("freeride"), competing: false });
    let unknown = Some(Round { mode: None, competing: false });
    let table: Vec<(Doing, &str, Option<&str>, bool, Timer)> = vec![
        (Doing::Starting, "Starting the game", None, false, Timer::Game),
        (Doing::Menu(Menu::Main), "In the menus", None, false, Timer::Game),
        (Doing::Menu(Menu::Settings), "In the settings", None, false, Timer::Game),
        (Doing::Menu(Menu::Globe(1)), "WORLD TOUR", Some("Choosing a drop"), false, Timer::Game),
        (Doing::Menu(Menu::Globe(0)), "EXPLORE", Some("Choosing a drop"), false, Timer::Game),
        (Doing::Menu(Menu::Globe(3)), "GLOBAL EVENTS", Some("Choosing a drop"), false, Timer::Game),
        (Doing::Menu(Menu::Globe(-1)), "In the menus", Some("Choosing a drop"), false, Timer::Game),
        (Doing::Menu(Menu::Equipment(1)), "WORLD TOUR", Some("Choosing gear"), false, Timer::Game),
        (Doing::Menu(Menu::Maps), "Choosing a map", None, false, Timer::Game),
        (Doing::Menu(Menu::Servers), "Multiplayer", Some("Browsing servers"), false, Timer::Game),
        (Doing::Menu(Menu::Joining), "Online", Some("Joining a server"), false, Timer::Game),
        (Doing::Menu(Menu::OnlineEquipment), "Online", Some("Choosing gear"), false, Timer::Game),
        (
            ride(Part::Loading, None),
            "WORLD TOUR · RACE IT!",
            Some("Loading Corkscrew, Kilimanjaro"),
            false,
            Timer::Game,
        ),
        (ride(Part::Riding, None), "WORLD TOUR · RACE IT!", Some("Corkscrew, Kilimanjaro"), false, Timer::Run),
        (ride(Part::Results, None), "WORLD TOUR · RACE IT!", Some("Your time: 01:23.45 · 1st"), false, Timer::Game),
        (ride(Part::Loading, race), "Online · Race", Some("Loading Corkscrew, Kilimanjaro"), true, Timer::Game),
        (ride(Part::Riding, race), "Online · Race", Some("Corkscrew, Kilimanjaro"), true, Timer::Run),
        (ride(Part::Results, race), "Online · Race", Some("Round results"), false, Timer::Game),
        (ride(Part::Riding, free), "Online · Freeride", Some("Corkscrew, Kilimanjaro"), false, Timer::Run),
        (ride(Part::Riding, unknown), "Online", Some("Corkscrew, Kilimanjaro"), false, Timer::Run),
    ];
    for (doing, details, state, competing, timer) in table {
        let l = lines(&doing, &names(), Some("Your time: 01:23.45 · 1st"), &mut loc);
        let want = Lines { details: details.to_string(), state: state.map(str::to_string), competing, timer };
        assert_eq!(l, want, "{doing:?}");
    }
    let none = RideNames { title: "Riding".into(), place: String::new() };
    assert_eq!(lines(&ride(Part::Loading, None), &none, None, &mut loc).state.as_deref(), Some("Loading"));
    assert_eq!(lines(&ride(Part::Riding, None), &none, None, &mut loc).state, None);
    assert_eq!(lines(&ride(Part::Results, None), &none, None, &mut loc).state, None);
    assert!(loc.missing.is_empty(), "{:?}", loc.missing);
}

#[test]
fn the_menus_by_screen_and_by_the_port_menus_pages() {
    let none = |_: &str| false;
    assert_eq!(menu(screens::id::MAIN_MENU, -1, &none), Menu::Main);
    assert_eq!(menu(screens::id::WELCOME_HUB, -1, &none), Menu::Main);
    for s in [7, 8, 9, 10] {
        assert_eq!(menu(s, -1, &none), Menu::Settings, "{s}");
    }
    for s in (0xC..=0x11).chain(0x42..=0x44) {
        assert_eq!(menu(s, 1, &none), Menu::Globe(1), "{s:#x}");
    }
    assert_eq!(menu(screens::id::EQUIPMENT, 0, &none), Menu::Equipment(0));
    assert_eq!(menu(0x31, 3, &none), Menu::Equipment(3));
    let on = |name: &'static str| move |n: &str| n == name || n == "AvaHub";
    assert_eq!(menu(screens::id::MAIN_MENU, -1, &on("AvaMaps")), Menu::Maps);
    assert_eq!(menu(screens::id::MAIN_MENU, -1, &on("AvaServerBrowser")), Menu::Servers);
    assert_eq!(menu(screens::id::MAIN_MENU, -1, &on("AvaBrowserFilters")), Menu::Servers);
    for o in OPTIONS_STATES {
        assert_eq!(menu(screens::id::MAIN_MENU, -1, &on(o)), Menu::Settings, "{o}");
    }
    assert_eq!(menu(screens::id::MAIN_MENU, -1, &on("AvaCredits")), Menu::Main);
    assert!(starting(0) && starting(screens::id::BOOT) && !starting(screens::id::MAIN_MENU));
}

#[test]
fn texts_are_read_without_their_markup() {
    assert_eq!(plain(r##"<font color="#40C4F1">RACE IT!</font>"##), "RACE IT!");
    assert_eq!(plain("Corkscrew"), "Corkscrew");
    assert_eq!(plain("Rock &amp; Roll"), "Rock & Roll");
    assert_eq!(plain("<p>one</p><p>two</p>"), "one two");
    assert_eq!(plain(""), "");
}

#[test]
fn the_results_line_has_the_measure_and_the_standing() {
    let mut loc = loc();
    let run = |time_ms, points, descent_mm, medal, rank| EventResult {
        event_type: None,
        points,
        time_ms,
        descent_mm,
        rewinds: 0,
        loops: 0,
        failed: false,
        value: 0,
        rivals: Vec::new(),
        rank,
        medal,
        place: 0,
        geotag_credits: 0,
        geotag_count: 0,
    };
    assert_eq!(outcome(&mut loc, 7, &run(83_450, 0, 0, None, Some(0))), "Your time: 01:23.45 · 1st");
    assert_eq!(outcome(&mut loc, 7, &run(83_450, 0, 0, None, Some(11))), "Your time: 01:23.45 · 12th");
    assert_eq!(outcome(&mut loc, 5, &run(61_000, 0, 0, None, Some(0))), "Your time: 01:01.00");
    assert_eq!(outcome(&mut loc, 0, &run(0, 123_456, 0, Some(Medal::Gold), Some(0))), "Your score: 123,456 · Gold");
    let failed = EventResult { failed: true, ..run(1, 2, 3, None, None) };
    assert_eq!(outcome(&mut loc, 3, &failed), "Did not finish");
    assert!(loc.missing.is_empty(), "{:?}", loc.missing);
}

#[test]
fn a_maps_names_are_its_mode_and_its_title() {
    let mut loc = loc();
    let map = |mode: &str| Launch {
        event_type: 0,
        map: Some(MapLaunch { id: "m".into(), title: "Big Air".into(), track: String::new(), mode: mode.into() }),
        ..Launch::default()
    };
    assert_eq!(
        ride_names(None, &mut loc, &map("race")),
        RideNames { title: "Map · Race".into(), place: "Big Air".into() }
    );
    assert_eq!(ride_names(None, &mut loc, &map("mod.gamemode")).title, "Map · TRICK IT!");
}

#[test]
fn a_drops_names_without_the_career() {
    let mut loc = loc();
    let launch = Launch { drop: 1, event_type: 5, game_mode: 1, ..Launch::default() };
    assert_eq!(
        ride_names(None, &mut loc, &launch),
        RideNames { title: "WORLD TOUR · RACE IT!".into(), place: "".into() }
    );
    let free = Launch { free_ride: true, game_mode: 0, ..launch.clone() };
    assert_eq!(ride_names(None, &mut loc, &free).title, "EXPLORE · Freeride");
    let tutorial = Launch { drop: TUTORIAL_DROPS[0], ..launch.clone() };
    assert_eq!(ride_names(None, &mut loc, &tutorial).title, "WORLD TOUR · TRAINING");
    let neither = Launch { game_mode: -1, event_type: 99, ..launch };
    assert_eq!(ride_names(None, &mut loc, &neither).title, "Riding");
}
