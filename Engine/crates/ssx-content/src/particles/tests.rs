use super::*;

const LNX: &str = r#"<?xml version='1.0' encoding='UTF-8'?>
<particleSystem version="1.7.0"><particleEffect name="fx_test">
  <parameter index="0" name="fDefaultBlendState" value="1" />
  <particleGroup name="spray">
    <parameter index="0" name="iNumParticlesMax" value="64" />
    <particleAction className="ParticleActionEmitBox" instanceName="emit">
      <parameter index="0" name="iEmitRate" value="32" />
      <parameter index="0" name="vCenter" value="{ 0, 1, 2 }" numElements="4" />
      <parameter index="0" name="vCenter" value="{ 0, 1, 3 }" />
      <state id="0001"><parameter index="0" name="iEmitRate" value="7" /></state>
    </particleAction>
    <particleAction className="ParticleActionRender" name="draw">
      <parameter index="0" name="sTexture" value="fx_snowparticulate.dds" />
      <parameter index="0" name="sShader" value="lynx.fx" />
      <parameter index="1" name="vColorRamp" value="{ 1, .65, .33, 0.66 }" />
      <parameter index="0" name="bUseLighting" value="0" />
      <parameter index="0" name="rKnot" value="spray/emit/iEmitRate" />
    </particleAction>
  </particleGroup>
</particleEffect></particleSystem>"#;

fn texture(name: &str) -> Option<String> {
    Some(format!("ssx:texture/fx/textures/{}", name.strip_suffix(".dds")?))
}

fn same(a: &ParticleSystem, b: &ParticleSystem, tex: &dyn Fn(&str) -> Option<String>) {
    let p = |x: &[Param], y: &[Param]| {
        assert_eq!(x.len(), y.len());
        for (u, v) in x.iter().zip(y) {
            assert_eq!((&u.name, u.index, u.num_elements), (&v.name, v.index, v.num_elements));
            match (&u.value, &v.value) {
                (Value::String(s), Value::String(t)) if u.name == TEXTURE_PARAMETER => {
                    assert_eq!(tex(s).as_ref(), Some(t))
                }
                (x, y) => assert_eq!(x, y, "{}", u.name),
            }
        }
    };
    assert_eq!(a.version, b.version);
    assert_eq!(a.effects.len(), b.effects.len());
    for (e, f) in a.effects.iter().zip(&b.effects) {
        assert_eq!(e.name, f.name);
        p(&e.params, &f.params);
        for (g, h) in e.groups.iter().zip(&f.groups) {
            assert_eq!(g.name, h.name);
            p(&g.params, &h.params);
            for (x, y) in g.actions.iter().zip(&h.actions) {
                assert_eq!((&x.class_name, &x.instance_name, &x.name), (&y.class_name, &y.instance_name, &y.name));
                p(&x.params, &y.params);
                for (s, t) in x.states.iter().zip(&y.states) {
                    assert_eq!(s.id, t.id);
                    p(&s.params, &t.params);
                }
            }
        }
    }
}

#[test]
fn a_system_round_trips_through_its_record() {
    let disc = ParticleSystem::parse(LNX.as_bytes()).unwrap();
    let table = system_to_table(&disc, &texture).unwrap();
    let text = toml::to_string(&table).unwrap();
    let back = parse_system(text.as_bytes()).unwrap();
    same(&disc, &back, &texture);
    let vc: Vec<&Param> = back.effects[0].groups[0].actions[0].params.iter().filter(|p| p.name == "vCenter").collect();
    assert_eq!((vc.len(), vc[0].num_elements), (2, Some(4)));
}

#[test]
fn hostile_systems_are_refused_with_where() {
    let bad = |t: &str, want: &str| {
        let e = parse_system(t.as_bytes()).unwrap_err();
        assert!(e[0].2.contains(want), "{e:?} has no {want:?}");
    };
    bad("version = \"1\"\n", "at least one");
    bad(
        "[[effect]]\nname = \"e\"\n[[effect.group]]\nname = \"g\"\n[[effect.group.action]]\nclass = \"Nope\"\n",
        "not a particle action",
    );
    bad(
        "[[effect]]\nname = \"e\"\n[[effect.group]]\nname = \"g\"\n[[effect.group.action]]\nclass = \"ParticleActionEmitBox\"\nparameters = [[\"iEmitRate\", 1.5]]\n",
        "effect[0].group[0].action[0].parameters[0]: iEmitRate: a 32-bit integer",
    );
    bad("[[effect]]\nname = \"e\"\nparameters = [[\"xMatrix\", 1.0]]\n", "a matrix");
    bad("[[effect]]\nname = \"e\"\nparameters = [[\"qWhat\", 1.0]]\n", "no type");
    bad("[[effect]]\nname = \"e\"\nparameters = [[\"fA\", 99999, 1.0]]\n", "the index is 0 to");
    bad(
        "[[effect]]\nname = \"e\"\n[[effect.group]]\nname = \"g\"\n[[effect.group.action]]\nclass = \"ParticleActionRender\"\nparameters = [[\"sTexture\", \"x.dds\"]]\n",
        "sTexture",
    );
    bad("[[effect]]\nname = \"e\"\nwhat = 1\n", "unknown key");
    let many = format!("{}[[effect]]\nname = \"e\"\n", "[[effect]]\nname = \"x\"\n".repeat(16));
    bad(&many, "at most 16");
}

#[test]
fn a_spray_table_round_trips_and_patches_slot_by_slot() {
    let mut t = SprayTable::default();
    t.slots[0][0] = Some(SpraySlot { node: 0x01c4_5447, particles: None, effect: None });
    t.slots[2][5] = Some(SpraySlot {
        node: 0xe150_96b8,
        particles: Some("p:particles/spray".parse().unwrap()),
        effect: Some("fx_spray".into()),
    });
    let table = t.to_table();
    assert_eq!(SprayTable::from_table(&table).unwrap(), t);
    let mut merged = table.clone();
    let patch: toml::Table =
        toml::from_str("[snow.carve]\nparticles = \"p:particles/own\"\neffect = \"own\"\n").unwrap();
    crate::patch::merge(&mut merged, &patch);
    let m = SprayTable::from_table(&merged).unwrap();
    let carve = m.slots[0][0].as_ref().unwrap();
    assert_eq!((carve.node, carve.effect.as_deref()), (0x01c4_5447, Some("own")));
    assert_eq!(m.slots[2][5], t.slots[2][5]);
    for (bad, want) in [
        ("[snow.carve]\nnode = \"12\"\n", "node"),
        ("[snow.carve]\nnode = \"0x01c45447\"\neffect = \"x\"\n", "go together"),
        ("[mud.carve]\nnode = \"0x01c45447\"\n", "unknown key"),
        ("[snow.slide]\nnode = \"0x01c45447\"\n", "unknown key"),
        (
            "[snow.carve]\nnode = \"0x01c45447\"\nparticles = \"p:particles/snowspray/x\"\neffect = \"x\"\n",
            "not a particle system",
        ),
    ] {
        let e = parse_spray(bad.as_bytes()).unwrap_err();
        assert!(e.iter().any(|(_, _, m)| m.contains(want)), "{bad}: {e:?}");
    }
}
