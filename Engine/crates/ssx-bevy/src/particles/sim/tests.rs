use ssx_formats::lynx::ParticleSystem;
use ssx_formats::lynx_schema;

use super::actions::{kill_age, turbulence_table, vec_cos_approx};
use super::defs::{ActionDef, compile_action};
use super::emit::{EmitEnv, emit_count};
use super::params::default_lanes;
use super::*;

fn action(xml: &str) -> ActionDef {
    let doc = format!(
        "<particleSystem><particleEffect name=\"e\"><particleGroup name=\"g\">{xml}</particleGroup></particleEffect></particleSystem>"
    );
    let sys = ParticleSystem::parse(doc.as_bytes()).unwrap();
    compile_action(&sys.effects[0].groups[0].actions[0])
}

#[test]
fn states_blend_against_the_base() {
    let a = action(
        r#"<particleAction className="ParticleActionEmitBox"><parameter name="iEmitRate" value="150" />
           <parameter name="fSizeMean" value="0.25" />
           <state id="0001"><parameter name="iEmitRate" value="80" /></state>
           <state id="0002"><parameter name="iEmitRate" value="40" /><parameter name="fSizeMean" value="3.0" /></state>
           </particleAction>"#,
    );
    let w = Weights::from_list(&[(1, 0.75), (2, 0.25)]);
    let v = View::new(&a.params, &w);
    assert_eq!(v.i("iEmitRate"), 70);
    assert!((v.f("fSizeMean") - (0.25 * 0.75 + 3.0 * 0.25)).abs() < 1e-6);
    assert_eq!(v.i("iEmitCycles"), -1);
}

#[test]
fn unset_parameters_read_the_schema_rows() {
    let w = Weights::default();
    for c in lynx_schema::CLASSES {
        let params = Params::new(c.name, &[], &[]);
        let v = View::new(&params, &w);
        for row in c.params {
            let want = default_lanes(c.param(row.name).expect("the row").default);
            assert_eq!(v.v(row.name, 0).to_array().map(f32::to_bits), want.to_array().map(f32::to_bits));
            assert_eq!(v.i(row.name), want.x as i32, "{} {}", c.name, row.name);
            assert!(v.built_in(row.name));
        }
        assert_eq!((v.v("no such parameter", 0), v.i("no such parameter")), (Vec4::ZERO, 0));
        assert!(!v.built_in("no such parameter"));
    }
    let unknown = Params::new("no such class", &[], &[]);
    let v = View::new(&unknown, &w);
    assert_eq!((v.f("fScale"), v.built_in("fScale")), (0.0, false));
}

#[test]
fn emit_count_uses_the_frame_modulo() {
    let a = action(
        r#"<particleAction className="ParticleActionEmitBox"><parameter name="iEmitRate" value="45" /></particleAction>"#,
    );
    let w = Weights::default();
    let p = View::new(&a.params, &w);
    let mut rng = Rng::default();
    let mut st = ActionState { time_started_ms: 1 };
    let total: usize = (1..=10)
        .map(|frame| {
            let ctx = Context {
                time_ms: 1000 + frame as i32 * 33,
                dt: 1.0 / 30.0,
                frame,
                dt_smoothed: 1.0 / 30.0,
                camera: Vec3::ZERO,
                player_shadow: 1.0,
                weather: Weather::default(),
            };
            let env = EmitEnv { ctx: &ctx, inst_start: 1, gate: true, dist: 0.0, lod: 1e8, free: 100 };
            emit_count(&p, &mut st, &env, &mut rng)
        })
        .sum();
    assert_eq!(total, 15);
}

fn instanced(instancing: i32) -> (Vec<EffectInstance>, SetArena) {
    let xml = format!(
        r#"<particleSystem><particleEffect name="e"><particleGroup name="g">
           <parameter name="iInstancing" value="{instancing}" />
           <particleAction className="ParticleActionEmitBox"><parameter name="iEmitRate" value="3" />
           <parameter name="bEmitRatePerFrame" value="1" /></particleAction>
           <particleAction className="ParticleActionRender"><parameter name="sShader" value="lynx.fx" />
           </particleAction></particleGroup></particleEffect></particleSystem>"#
    );
    let sys = ParticleSystem::parse(xml.as_bytes()).unwrap();
    let def = EffectDef::compile(&sys.effects[0]);
    let mut arena = SetArena::default();
    let mut insts: Vec<EffectInstance> = (0..2).map(|_| EffectInstance::new(def.clone(), &mut arena)).collect();
    let rec = RecordView {
        scale: Vec3::ONE,
        rotation: Quat::IDENTITY,
        position: Vec3::ZERO,
        start: 0.0,
        stop: -1.0,
        instant_off: false,
        ao: 1.0,
        blend: Vec::new(),
    };
    let vis = Visibility { metric: 1.0, visible: true };
    for i in &mut insts {
        i.apply_record(&rec, 1.0, 1.0 / 30.0, vis, &mut arena);
    }
    (insts, arena)
}

#[test]
fn shared_sets_follow_the_instancing_flags() {
    let ctx = Context {
        time_ms: 1000,
        dt: 1.0 / 30.0,
        frame: 1,
        dt_smoothed: 1.0 / 30.0,
        camera: Vec3::ZERO,
        player_shadow: 1.0,
        weather: Weather::default(),
    };
    let (mut insts, mut arena) = instanced(2);
    assert_eq!(insts[0].sets, insts[1].sets);
    let mut rng = Rng::default();
    for i in &mut insts {
        i.update(&ctx, &mut rng, &mut arena);
    }
    assert_eq!(arena.sets[insts[0].sets[0]].particles.len(), 6);
    assert_eq!((insts[0].rendered.len(), insts[1].rendered.len()), (1, 0));
    let (mut insts, mut arena) = instanced(1);
    for i in &mut insts {
        i.update(&ctx, &mut rng, &mut arena);
    }
    assert_eq!(arena.sets[insts[0].sets[0]].particles.len(), 3);
    assert_eq!((insts[0].rendered.len(), insts[1].rendered.len()), (1, 1));
    let (insts, _) = instanced(0);
    assert_ne!(insts[0].sets, insts[1].sets);
}

#[test]
fn cos_approx_matches_cos() {
    for i in 0..256 {
        let x = i as f32 * 0.024_543_693;
        assert!((vec_cos_approx(x) - x.cos()).abs() < 2e-6, "{x}");
    }
    assert!((turbulence_table()[0] - 1.0).abs() < 1e-6 && turbulence_table()[128] + 1.0 < 1e-5);
}

#[test]
fn kill_age_orderings() {
    let p = |birth: f32| Particle { birth, ..Default::default() };
    let a = action(
        r#"<particleAction className="ParticleActionKillAge"><parameter name="iAgeMs" value="100" /></particleAction>"#,
    );
    let w = Weights::default();
    let v = View::new(&a.params, &w);
    let mut ps = vec![p(0.0), p(950.0), p(10.0), p(990.0)];
    kill_age(&v, &mut ps, 1000);
    let births: Vec<f32> = ps.iter().map(|q| q.birth).collect();
    assert_eq!(births, vec![990.0, 950.0]);
}
