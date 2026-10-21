use super::*;
use crate::game::Binder;
use crate::game::graph::run_batch;

const SCRIPT: &str = r#"
local s = CreateStateSet( "Thing" )
local st = s:AddState( "Default" )
	local t = SubStateTrack( "LampOff" )
	st:AddUpdateTrack( t )
	t = ParticleTrack( "sparks" )
		t:ActivateOnStart()
		t:SetIsGroupName()
	st:AddUpdateTrack( t, 0.25 )
st = s:AddState( "LampOff" )
	local tr = st:AddTransition( "LampOn" )
	tr:AddCondition( NamedFlagCondition( "Night" ) )
st = s:AddState( "LampOn" )
	t = LightTrack( "lamp" )
		t:ActivateOnStart()
		t:DeactivateOnEnd()
	st:AddUpdateTrack( t )
	tr = st:AddTransition( "LampOff" )
	tr:AddCondition( NOT( NamedFlagCondition( "Night" ) ) )
	tr:AddCondition( StateTimeCondition( 1.0 ) )
"#;

#[derive(Default)]
struct W {
    night: bool,
    log: Vec<String>,
}

impl ControllerWorld for W {
    fn named_flag(&mut self, n: &Name) -> bool {
        n.is("Night") && self.night
    }
    fn track(&mut self, c: TrackCall<'_>, phase: Phase) {
        let def = c.def;
        let name = def.arg(1).str().unwrap_or_default().to_string();
        let p = match phase {
            Phase::Begin => "begin".to_string(),
            Phase::Update(d) => format!("update {d:.2}"),
            Phase::End => "end".to_string(),
        };
        self.log.push(format!("{} {name} {p}", def.kind));
    }
}

#[test]
fn sub_states_transitions_and_track_times() {
    let g = run_batch(Binder::StateSets, &[("t.lua".into(), SCRIPT.as_bytes().to_vec())], None);
    assert!(g.scripts[0].1.is_none(), "{:?}", g.scripts);
    let sets = Arc::new(StateSets::from_graph(&g));
    assert!(sets.problems.is_empty(), "{:?}", sets.problems);
    assert_eq!(sets.sets[0].states.len(), 3);
    let mut c = Controller::new(sets.clone(), sets.set("Thing").expect("set"));
    let mut w = W::default();
    c.set_state(None, &mut w);
    c.update(0.5, &mut w);
    assert_eq!(c.states(), ["Default", "LampOff"]);
    assert!(w.log.contains(&"ParticleTrack sparks update 0.25".to_string()), "{:?}", w.log);
    w.night = true;
    w.log.clear();
    c.update(0.5, &mut w);
    assert_eq!(c.states(), ["Default", "LampOn"]);
    assert!(
        w.log.contains(&"LightTrack lamp begin".to_string())
            && w.log.contains(&"LightTrack lamp update 0.50".to_string())
    );
    w.night = false;
    w.log.clear();
    c.update(0.4, &mut w);
    assert_eq!(c.states(), ["Default", "LampOn"], "the state time 0.5 < 1.0 holds the transition");
    c.update(0.6, &mut w);
    assert_eq!(c.states(), ["Default", "LampOn"], "the transitions test before the update adds dt: 0.9 < 1.0");
    c.update(0.1, &mut w);
    assert_eq!(c.states(), ["Default", "LampOff"]);
    assert!(w.log.contains(&"LightTrack lamp end".to_string()));
}

#[test]
fn track_assets_decode_their_fields() {
    let src = r#"
local s = CreateStateSet( "T" )
local st = s:AddState( "A" )
local t = AoTrack()
    t:SetAmbientOcclusion( 0.5 )
    t:SetBlendTime( 0.1 )
st:AddUpdateTrack( t )
st:AddUpdateTrack( RenderAttributeTrack( "dynamicAOMultOverride", 0.0, 0.5 ) )
t = GlobalLightingTrack( "whiteout_lighting", 10.0 )
    t:BlendOutTime( 10.0 )
st:AddUpdateTrack( t )
st:AddUpdateTrack( OutOfBoundsTrack( "Player1", 0.2 ) )
st:AddEnterTrack( SetNamedFlagTrack( "f", 1, TEMPORARY ) )
st:AddEnterTrack( GameStateTrack( FLT_BigHeadScale, 2.0 ) )
t = HeadLampGearTrack()
    t:ActivateOnStart()
    t:DeactivateOnEnd()
st:AddUpdateTrack( t )
st:AddUpdateTrack( PostFxTrack( "awesome_flash", 0.0, 0.5, 1.0 ) )
"#;
    let g = run_batch(Binder::StateSets, &[("t.lua".into(), src.as_bytes().to_vec())], None);
    let sets = StateSets::from_graph(&g);
    assert!(sets.problems.is_empty(), "{:?}", sets.problems);
    let k = &sets.kinds;
    assert_eq!(k[0], Track::Ao { ao: 0.5, blend: 0.1 });
    assert!(matches!(
        &k[1],
        Track::RenderAttribute { value: [0.5, 0.5, 0.5, 0.5], blend: 0.0, reset_on_end: false, .. }
    ));
    assert!(matches!(&k[2], Track::GlobalLighting { blend_in: 10.0, blend_out: Some(10.0), append: false, .. }));
    assert!(matches!(&k[3], Track::OutOfBounds { timer: 0.2, strength: 2.0, .. }));
    assert!(matches!(&k[4], Track::SetNamedFlag { value: true, temporary: true, .. }));
    assert!(matches!(&k[5], Track::GameState { value: GsValue::Float(2.0), .. }));
    assert!(matches!(
        &k[6],
        Track::Light { head_lamp: true, on_start: tracks::Switch::Activate, on_end: tracks::Switch::Deactivate, .. }
    ));
    assert!(matches!(&k[7], Track::PostFx { blend_in: 0.0, blend_out: 0.5, weight: 1.0, .. }));
}
