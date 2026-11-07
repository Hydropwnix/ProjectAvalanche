use super::super::{Signal, SignalKind};
use super::*;

const ROOT: CtrlId = 0;
const INDY: CtrlId = 4;
const SEATBELT: CtrlId = 18;

fn chooser(signal: usize, entries: &[(i64, CtrlId)]) -> Ctrl {
    Ctrl::Chooser {
        policy: "EnumChooserPolicyAsset".into(),
        policy_signal: Some(signal),
        cache_last_choice: false,
        entries: entries
            .iter()
            .map(|&(v, c)| ChooserEntry { controller: Some(c), selector: Selector::Enum(v) })
            .collect(),
        choose_on_reset: false,
        choose_every_update: false,
        choose_as_blend_slave: false,
        looping: false,
        choose_on_loop: false,
        phase_match_previous: false,
        blend: None,
    }
}

fn clip(anim: String, n: u8) -> Ctrl {
    Ctrl::Clip {
        anim,
        anim_guid: Some(Guid([n; 16])),
        target: "AnimRig".into(),
        num_ticks: 30.0,
        fps: 30.0,
        fps_scale: 1.0,
        delta_trajectory: 0,
        distance: 0.0,
    }
}

fn graph() -> Graph {
    let mut g = Graph::default();
    for name in ["GSENUM_TrickType", "GSENUM_TrickTypeLocked", "GSENUM_TrickHand"] {
        g.signals.push(Signal { name: name.into(), kind: SignalKind::Enum(0), trigger: false });
    }
    let air = Ctrl::Modular(vec![(Some(1), true), (Some(2), true), (Some(3), true)]);
    g.ctrls.push(("SF_AirTricks".into(), air));
    g.ctrls.push(("chooser_enter".into(), chooser(0, &[(3, INDY), (8, SEATBELT)])));
    g.ctrls.push(("chooser_exit".into(), chooser(1, &[(3, INDY), (8, SEATBELT)])));
    g.ctrls.push(("chooser_hand".into(), chooser(2, &[(0, SEATBELT)])));
    let places = (0..12).map(|r| (Some(5 + r), true)).collect();
    g.ctrls.push(("indy".into(), Ctrl::Modular(places)));
    for (r, (_, suffix)) in ssx_content::trick::ROLES.iter().enumerate() {
        g.ctrls.push((format!("clip_indy{suffix}"), clip(format!("1_trick_air_indy{suffix}"), r as u8)));
    }
    g.ctrls.push(("chooser_grind".into(), chooser(0, &[(14, SEATBELT)])));
    g.ctrls.push(("clip_seatbelt".into(), clip("1_trick_air_seatbelt".into(), 99)));
    g
}

fn trick(base: &str) -> (ContentId, TrickToml) {
    let text = ssx_content::trick::template::trick_toml("p").replace("base = \"indy\"", &format!("base = \"{base}\""));
    let t = ssx_content::trick::parse_toml(text.as_bytes()).expect("the template reads");
    ("p:trick/air_walk".parse().expect("an id"), t)
}

fn guids() -> Vec<Guid> {
    (0..12).map(|r| Guid([200 + r; 16])).collect()
}

fn entries(g: &Graph, c: CtrlId) -> Vec<(i64, Option<CtrlId>)> {
    let Ctrl::Chooser { entries, .. } = &g.ctrls[c].1 else { panic!("{c} is a chooser") };
    entries.iter().map(|e| (if let Selector::Enum(v) = e.selector { v } else { -1 }, e.controller)).collect()
}

#[test]
fn the_trick_goes_in_before_each_trick_choosers_last_and_its_base_is_copied_once() {
    let mut g = graph();
    let before = g.ctrls.clone();
    let mut choosers = trick_choosers(&g);
    choosers.sort_unstable();
    assert_eq!(choosers, [1, 2], "the two trick choosers under SF_AirTricks, not the hand's or the grind's");
    let (id, t) = trick("indy");
    add_trick(&mut g, &choosers, &id, &t, &guids()).expect("the trick fits");
    let start = before.len();
    assert_eq!(g.ctrls.len(), start + 13, "the Indy's controllers once: its node and its 12 clips");
    for c in [1, 2] {
        assert_eq!(entries(&g, c), [(3, Some(INDY)), (32, Some(start)), (8, Some(SEATBELT))]);
    }
    for (i, c) in before.iter().enumerate().filter(|(i, _)| ![1, 2].contains(i)) {
        assert_eq!(&g.ctrls[i], c, "{} unchanged", c.0);
    }
    let Ctrl::Modular(places) = &g.ctrls[start].1 else { panic!("the copy of the Indy's node") };
    for (r, (child, _)) in places.iter().enumerate() {
        let Ctrl::Clip { anim, anim_guid, .. } = &g.ctrls[child.expect("a child")].1 else { panic!("a clip") };
        assert_eq!(anim, &format!("p:trick/air_walk {}", ssx_content::trick::ROLES[r].0));
        assert_eq!(*anim_guid, Some(guids()[r]));
    }
}

#[test]
fn a_trick_the_graph_cannot_take_changes_nothing() {
    let (id, t) = trick("indy");
    let mut g = graph();
    let choosers = trick_choosers(&g);
    add_trick(&mut g, &choosers, &id, &t, &guids()).expect("the trick fits");
    let once = g.ctrls.clone();
    let again = add_trick(&mut g, &choosers, &id, &t, &guids());
    assert!(again.expect_err("its value is there").contains("has an entry of 32 already"));
    assert_eq!(g.ctrls, once);
    let mut g = graph();
    let before = g.ctrls.clone();
    let (_, melon) = trick("melon");
    let no_base = add_trick(&mut g, &choosers, &id, &melon, &guids());
    assert!(no_base.expect_err("no Melon entry").contains("no entry of the base's value 4"));
    assert!(add_trick(&mut g, &choosers, &id, &t, &guids()[..11]).is_err(), "11 GUIDs for 12 places");
    assert_eq!(g.ctrls, before);
    let mut g = graph();
    if let Ctrl::Modular(places) = &mut g.ctrls[INDY].1 {
        places.pop();
    }
    let before = g.ctrls.clone();
    let short = add_trick(&mut g, &choosers, &id, &t, &guids());
    assert!(short.expect_err("11 places").contains("the base plays 11 of its 12 clip places"));
    assert_eq!(g.ctrls, before);
}

#[test]
fn a_graph_without_air_tricks_has_no_trick_choosers() {
    let mut g = graph();
    g.ctrls[ROOT].0 = "SF_Other".into();
    assert!(trick_choosers(&g).is_empty());
}
