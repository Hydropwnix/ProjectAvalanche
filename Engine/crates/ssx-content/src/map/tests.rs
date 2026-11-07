use std::collections::BTreeMap;

use super::*;
use crate::model::FileReader;

struct Files(BTreeMap<String, Vec<u8>>);

impl FileReader for Files {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        if b.len() as u64 > max { Err(format!("{path}: over {max} bytes")) } else { Ok(b.clone()) }
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        Ok(self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?.len() as u64)
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.0.get(path).ok_or_else(|| format!("{path}: not in the package"))?;
        let (o, l) =
            (usize::try_from(offset).map_err(|e| e.to_string())?, usize::try_from(len).map_err(|e| e.to_string())?);
        b.get(o..o.saturating_add(l)).map(<[u8]>::to_vec).ok_or_else(|| format!("{path}: short"))
    }
}

#[derive(Default)]
struct G {
    bin: Vec<u8>,
    accessors: Vec<String>,
    views: Vec<String>,
    meshes: Vec<String>,
    nodes: Vec<String>,
}

impl G {
    fn view(&mut self, bytes: &[u8]) -> usize {
        let start = self.bin.len();
        self.bin.extend(bytes);
        self.views.push(format!("{{\"buffer\":0,\"byteOffset\":{start},\"byteLength\":{}}}", bytes.len()));
        self.views.len() - 1
    }

    fn mesh(&mut self, positions: &[[f32; 3]], indices: &[u32]) -> usize {
        let p: Vec<u8> = positions.iter().flatten().flat_map(|x| x.to_le_bytes()).collect();
        let v = self.view(&p);
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for q in positions {
            for k in 0..3 {
                lo[k] = lo[k].min(q[k]);
                hi[k] = hi[k].max(q[k]);
            }
        }
        let num = |x: f32| if x.is_finite() { format!("{x:?}") } else { "0.0".into() };
        self.accessors.push(format!(
            "{{\"bufferView\":{v},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}}",
            positions.len(),
            num(lo[0]),
            num(lo[1]),
            num(lo[2]),
            num(hi[0]),
            num(hi[1]),
            num(hi[2])
        ));
        let pa = self.accessors.len() - 1;
        let i: Vec<u8> = indices.iter().flat_map(|x| x.to_le_bytes()).collect();
        let iv = self.view(&i);
        self.accessors.push(format!(
            "{{\"bufferView\":{iv},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}",
            indices.len()
        ));
        let ia = self.accessors.len() - 1;
        self.meshes.push(format!("{{\"primitives\":[{{\"attributes\":{{\"POSITION\":{pa}}},\"indices\":{ia}}}]}}"));
        self.meshes.len() - 1
    }

    fn quad(&mut self, half: f32, y: f32) -> usize {
        let p = [[-half, y, -half], [half, y, -half], [half, y, half], [-half, y, half]];
        self.mesh(&p, &[0, 3, 1, 1, 3, 2])
    }

    fn node(&mut self, json: &str) -> usize {
        self.nodes.push(json.to_string());
        self.nodes.len() - 1
    }

    fn text(&self, roots: &[usize]) -> String {
        let roots: Vec<String> = roots.iter().map(ToString::to_string).collect();
        format!(
            "{{\"asset\":{{\"version\":\"2.0\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"meshes\":[{}],\
             \"accessors\":[{}],\"bufferViews\":[{}],\"buffers\":[{{\"byteLength\":{},\"uri\":\"data:application/octet-stream;base64,{}\"}}]}}",
            roots.join(","),
            self.nodes.join(","),
            self.meshes.join(","),
            self.accessors.join(","),
            self.views.join(","),
            self.bin.len(),
            template::base64(&self.bin)
        )
    }
}

const RACE: &str = "[map]\ntitle = \"T\"\nscene = \"s.gltf\"\nmodes = [\"race\"]\n";

fn read(toml: &str, scene: &str) -> Result<Map, MapError> {
    let files = Files([("s.gltf".to_string(), scene.as_bytes().to_vec())].into_iter().collect());
    read_map(toml.as_bytes(), "map.toml", &files)
}

fn refused(toml: &str, scene: &str, want: &str) -> MapError {
    match read(toml, scene) {
        Ok(_) => panic!("accepted; wanted {want:?}"),
        Err(e) => {
            assert!(e.to_string().contains(want), "{e} does not say {want:?}");
            e
        }
    }
}

fn race(more: impl FnOnce(&mut G) -> Vec<usize>) -> String {
    let mut g = G::default();
    let ground = g.quad(50.0, 0.0);
    let cube = g.mesh(
        &[
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
        ],
        &[0, 2, 1, 0, 3, 2, 4, 5, 2],
    );
    let mut kids = vec![
        g.node(&format!("{{\"name\":\"ground\",\"mesh\":{ground},\"extras\":{{\"ava_type\":\"terrain\",\"ava_surface\":\"powder\"}}}}")),
        g.node("{\"name\":\"start\",\"translation\":[0,1,-40],\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":0}}"),
        g.node(&format!(
            "{{\"name\":\"finish\",\"mesh\":{cube},\"translation\":[0,2,40],\"scale\":[20,4,2],\"extras\":{{\"ava_type\":\"finish\"}}}}"
        )),
    ];
    kids.extend(more(&mut g));
    let list: Vec<String> = kids.iter().map(ToString::to_string).collect();
    let root = g.node(&format!("{{\"name\":\"course\",\"children\":[{}]}}", list.join(",")));
    g.text(&[root])
}

#[test]
fn a_small_race_reads_in_the_world_frame() {
    let scene = race(|g| {
        vec![
            g.node("{\"name\":\"cp\",\"translation\":[0,1,0],\"extras\":{\"ava_type\":\"checkpoint\",\"ava_order\":0,\"ava_radius\":5}}"),
            g.node("{\"name\":\"lake\",\"translation\":[0,-10,0],\"extras\":{\"ava_type\":\"death\",\"ava_radius\":30}}"),
        ]
    });
    let m = read(RACE, &scene).expect("reads");
    let d = &m.definition;
    assert_eq!((d.terrain.len(), d.spawns.len(), d.checkpoints.len(), d.deaths.len()), (1, 1, 1, 1));
    assert_eq!(d.terrain[0].weights[0], TERRAIN_SURFACES[0].1);
    assert_eq!(d.spawns[0].position, [0.0, 1.0, -40.0]);
    assert_eq!(d.spawns[0].node, "course/start");
    let f = d.finish.as_ref().expect("a finish");
    assert_eq!(f.name, "end");
    match f.shape {
        VolumeShape::Box { centre, half_extents, .. } => {
            assert_eq!(centre, [0.0, 2.0, 40.0]);
            assert_eq!(half_extents, [10.0, 2.0, 1.0]);
        }
        VolumeShape::Sphere { .. } => panic!("a box"),
    }
    assert_eq!(sim_hash(&m), sim_hash(&read(RACE, &scene).expect("reads")), "one hash for one map");
    let lit = read(&format!("{RACE}lighting = \"ssx:lighting/night_clear_lighting\"\n"), &scene).expect("reads");
    let sky = read(&format!("{RACE}sky = \"pack:sky/aurora\"\n"), &scene).expect("reads");
    assert_ne!(sim_hash(&lit), sim_hash(&m));
    assert_eq!(sim_hash(&sky), sim_hash(&m));
}

#[test]
fn the_template_reads_and_its_committed_copy_is_the_generated_one() {
    let toml = template::MAP_TOML;
    let files = Files([("maps/course.gltf".to_string(), template::gltf().into_bytes())].into_iter().collect());
    let m = read_map(toml.as_bytes(), "map.toml", &files).expect("the template reads");
    let d = &m.definition;
    assert_eq!((d.spawns.len(), d.checkpoints.len(), d.rails.len(), d.markers.len()), (1, 2, 1, 1));
    assert!(d.finish.is_some() && d.deaths.len() == 1 && d.terrain.len() == 1);
    assert_eq!(d.rails[0].triangles[0].group, RAIL_GROUP);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/templates/map");
    let committed = std::fs::read_to_string(dir.join("maps/course.gltf")).unwrap_or_default();
    assert!(
        committed == template::gltf(),
        "maps/course.gltf differs from the generator: run `cargo run -p ssx-content --example map_template`"
    );
}

#[test]
fn a_mirrored_node_keeps_its_triangles_facing_up() {
    let scene = race(|g| {
        let q = g.quad(5.0, 0.0);
        vec![g.node(&format!(
            "{{\"name\":\"ledge\",\"mesh\":{q},\"scale\":[-1,1,1],\"extras\":{{\"ava_type\":\"collision\",\"ava_surface\":\"wood\"}}}}"
        ))]
    });
    let m = read(RACE, &scene).expect("reads");
    let t = &m.definition.collision[0].triangles[0].corners;
    let (u, v) = ([t[1][0] - t[0][0], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][2] - t[0][2]]);
    assert!(u[1] * v[0] - u[0] * v[1] > 0.0, "counter-clockwise from above");
}

#[test]
fn hostile_scenes_are_refused_with_the_node_and_key() {
    let nan = race(|g| {
        let m = g.mesh(&[[0.0, f32::NAN, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]], &[0, 2, 1]);
        vec![g.node(&format!(
            "{{\"name\":\"bad\",\"mesh\":{m},\"extras\":{{\"ava_type\":\"collision\",\"ava_surface\":\"wood\"}}}}"
        ))]
    });
    refused(
        RACE,
        &nan,
        "node \"course/bad\": meshes[2].primitives[0]: POSITION: accessors[4]: a value that is not a finite number",
    );
    let far =
        race(|g| {
            vec![g.node(
                "{\"name\":\"far\",\"translation\":[200000,0,0],\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":1}}",
            )]
        });
    refused(RACE, &far, "node \"course/far\": a position beyond");
    let unknown =
        race(|g| vec![g.node("{\"name\":\"x\",\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":1,\"ava_colour\":1}}")]);
    refused(RACE, &unknown, "node \"course/x\" ava_colour: not one of the map's keys");
    let wrong =
        race(|g| vec![g.node("{\"name\":\"x\",\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":1,\"ava_radius\":1}}")]);
    refused(RACE, &wrong, "ava_radius: not read on a spawn node");
    let twice = race(|g| vec![g.node("{\"name\":\"x\",\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":0}}")]);
    refused(RACE, &twice, "slot 0 twice");
    let untyped = race(|g| {
        let q = g.quad(1.0, 0.0);
        vec![g.node(&format!("{{\"name\":\"y\",\"mesh\":{q}}}"))]
    });
    refused(RACE, &untyped, "node \"course/y\" ava_type: a mesh without ava_type");
    let big = format!("{{\"name\":\"x\",\"extras\":{{\"note\":\"{}\"}}}}", "a".repeat(1 << 20));
    refused(RACE, &race(|g| vec![g.node(&big)]), "at most 4096 are read");
    let end = race(|g| {
        vec![g.node("{\"name\":\"t\",\"extras\":{\"ava_type\":\"trigger\",\"ava_name\":\"end\",\"ava_radius\":2}}")]
    });
    refused(RACE, &end, "\"end\" is the finish's name");
}

#[test]
fn hostile_node_names_are_escaped_and_paths_bounded() {
    let esc =
        race(|g| vec![g.node("{\"name\":\"a\\u001b[2J\\\"b\",\"extras\":{\"ava_type\":\"spawn\",\"ava_colour\":1}}")]);
    let e = refused(RACE, &esc, "node \"course/a\\u{1b}[2J\\\"b\" ava_colour").to_string();
    assert!(!e.contains('\u{1b}'), "{e:?}");
    let deep = race(|g| {
        let mut child = g.node("{\"name\":\"leaf\",\"extras\":{\"ava_type\":\"spawn\",\"ava_colour\":1}}");
        for k in 0..60 {
            child = g.node(&format!("{{\"name\":\"{}{k}\",\"children\":[{child}]}}", "x".repeat(100)));
        }
        vec![child]
    });
    let e = refused(RACE, &deep, "/leaf\" ava_colour").to_string();
    assert!(e.contains("node \"…/") && e.chars().count() < 1024, "{} characters: {e}", e.chars().count());
    let cameras = race(|g| (0..100).map(|k| g.node(&format!("{{\"name\":\"c{k}\",\"camera\":0}}"))).collect())
        .replacen(
            "\"scene\":0,",
            "\"cameras\":[{\"type\":\"perspective\",\"perspective\":{\"yfov\":1.0,\"znear\":0.1}}],\"scene\":0,",
            1,
        );
    let map = read(RACE, &cameras).expect("cameras without ava_type are not refused");
    assert_eq!(
        map.definition.notes,
        vec!["100 cameras without ava_type are not used (the first: node \"course/c0\")".to_string()]
    );
}

#[test]
fn cycles_shared_nodes_and_deep_trees_are_refused() {
    let mut g = G::default();
    let a = g.node("{\"name\":\"a\",\"children\":[1]}");
    g.node("{\"name\":\"b\",\"children\":[0]}");
    refused(RACE, &g.text(&[a]), "reached twice");
    let mut g = G::default();
    g.node("{\"name\":\"shared\"}");
    g.node("{\"children\":[0]}");
    g.node("{\"children\":[0]}");
    refused(RACE, &g.text(&[1, 2]), "nodes[0] is reached twice");
    let mut g = G::default();
    for i in 0..10_000 {
        let next = if i + 1 < 10_000 { format!(",\"children\":[{}]", i + 1) } else { String::new() };
        g.node(&format!("{{\"name\":\"n{i}\"{next}}}"));
    }
    refused(RACE, &g.text(&[0]), "10000 nodes; at most 4096");
    let mut g = G::default();
    for i in 0..100 {
        let next = if i + 1 < 100 { format!(",\"children\":[{}]", i + 1) } else { String::new() };
        g.node(&format!("{{\"name\":\"n{i}\"{next}}}"));
    }
    refused(RACE, &g.text(&[0]), "nested deeper than 64");
}

#[test]
fn a_map_without_its_requirements_is_refused() {
    let mut g = G::default();
    let q = g.quad(10.0, 0.0);
    let ground = g.node(&format!("{{\"mesh\":{q},\"extras\":{{\"ava_type\":\"terrain\",\"ava_surface\":\"ice\"}}}}"));
    refused(RACE, &g.text(&[ground]), "a new track has a spawn");
    let spawn = g.node("{\"extras\":{\"ava_type\":\"spawn\",\"ava_slot\":0}}");
    refused(RACE, &g.text(&[ground, spawn]), "a race map needs a finish");
    let trick = RACE.replace("[\"race\"]", "[\"trick\"]");
    refused(&trick, &g.text(&[ground, spawn]), "a trick map needs a finish");
    let freeride = RACE.replace("[\"race\"]", "[\"freeride\"]");
    assert!(read(&freeride, &g.text(&[ground, spawn])).is_ok(), "a freeride map needs no finish");
    let heli = format!("{freeride}intro = \"heli\"\n");
    refused(&heli, &g.text(&[ground, spawn]), "needs a heli_path");
    let layout = format!("{freeride}base = \"ssx:drop/alps0101\"\n");
    refused(&layout, &g.text(&[ground, spawn]), "adds no terrain");
}

#[test]
fn map_toml_problems_have_their_lines() {
    let bad = "[map]\ntitle = \"\"\nscene = \"../x.glb\"\nbase = \"pack:drop/a\"\nmodes = [\"race\", \"race\", \"golf\"]\n\
               time_limit_s = 4000\nintro = \"walk\"\nlighting = \"ssx:sky/x\"\n";
    let problems = parse_toml(bad.as_bytes()).expect_err("refused");
    let lines: Vec<usize> = problems.iter().map(|p| p.0).collect();
    assert_eq!(lines, vec![2, 3, 4, 5, 5, 6, 7, 8], "{problems:?}");
    assert!(parse_toml(b"[map]\ntitle = \"x\"\nscene = \"a.glb\"\nmodes = [\"trick\"]\ncolour = 1\n").is_err());
    let own = "[map]\ntitle = \"x\"\nscene = \"a.glb\"\nmodes = [\"race\"]\nlighting = \"pack:lighting/x\"\n";
    let p = parse_toml(own.as_bytes()).expect_err("refused");
    assert!(p[0].0 == 5 && p[0].2.contains("a preset of the disc"), "{p:?}");
    let ok = parse_toml(template::MAP_TOML.as_bytes()).expect("the template's record reads");
    assert_eq!(ok.modes, vec![Mode::Race, Mode::Freeride]);
}

#[test]
fn presentation_references_follow_the_games_kinds() {
    let with = |more: &str| format!("{RACE}{more}\n");
    let problem = |more: &str| {
        let p = parse_toml(with(more).as_bytes()).expect_err("refused");
        p.into_iter().map(|p| p.2).collect::<Vec<_>>().join("; ")
    };
    assert!(problem("postfx = \"pack:postfx/x\"").contains("a preset of the disc"));
    assert!(problem("music = \"ssx:music/pathfinder/ssxdd/0\"").contains("a song of the disc"));
    assert!(problem("midground = \"pack:track/x\"").contains("a track of the disc"));
    assert!(problem("base = \"ssx:drop/a\"\nmidground = \"ssx:track/alps0101\"").contains("only a new track"));
    let ok = parse_toml(with("music = \"ssx:music/licensed/7\"\nmidground = \"ssx:track/alps0101\"").as_bytes())
        .expect("a disc song and a disc track's mid-ground");
    assert_eq!(ok.music.map(|m| m.to_string()).as_deref(), Some("ssx:music/licensed/7"));
    assert!(parse_toml(with("music = \"pack:music/song\"").as_bytes()).is_ok(), "a package's song");
}

#[test]
fn an_object_prop_takes_no_material() {
    let object = race(|g| {
        vec![g.node(
            "{\"name\":\"p\",\"translation\":[0,0,0],\"extras\":{\"ava_type\":\"prop\",\"ava_object\":\"ssx:object/x\",\
             \"ava_material\":\"ssx:material/y\"}}",
        )]
    });
    refused(RACE, &object, "only an own mesh takes one");
    let terrain = race(|g| {
        let q = g.quad(5.0, 1.0);
        vec![g.node(&format!(
            "{{\"name\":\"t\",\"mesh\":{q},\"extras\":{{\"ava_type\":\"terrain\",\"ava_surface\":\"rock\",\
             \"ava_material\":\"ssx:material/terrain_rock_mat\"}}}}"
        ))]
    });
    let m = read(RACE, &terrain).expect("a terrain node names its material");
    let named = m.definition.terrain.iter().filter_map(|t| t.material.as_ref()).map(ToString::to_string);
    assert_eq!(named.collect::<Vec<_>>(), vec!["ssx:material/terrain_rock_mat".to_string()]);
}
