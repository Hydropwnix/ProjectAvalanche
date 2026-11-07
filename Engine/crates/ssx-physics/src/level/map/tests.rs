use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use ssx_content::map::{BoundaryArea, Map, VolumeShape, read_map, template};
use ssx_content::model::FileReader;

use super::*;
use crate::collision::CollisionWorld;
use crate::session::Session;

struct Scene(String);

impl FileReader for Scene {
    fn read(&self, path: &str, _max: u64) -> Result<Vec<u8>, String> {
        if path == "maps/course.gltf" { Ok(self.0.clone().into_bytes()) } else { Err(format!("{path}: absent")) }
    }
    fn size(&self, path: &str) -> Result<u64, String> {
        self.read(path, u64::MAX).map(|b| b.len() as u64)
    }
    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let b = self.read(path, u64::MAX)?;
        let o = usize::try_from(offset).map_err(|e| e.to_string())?;
        let l = usize::try_from(len).map_err(|e| e.to_string())?;
        b.get(o..o.saturating_add(l)).map(<[u8]>::to_vec).ok_or_else(|| "short".into())
    }
}

fn template_map() -> Map {
    read_map(template::MAP_TOML.as_bytes(), "map.toml", &Scene(template::gltf())).expect("the template reads")
}

#[test]
fn the_template_becomes_terrain_and_rail_clusters_as_the_disc_has_them() {
    let map = template_map();
    let mut c = CollisionWorld::default();
    let stats = add_map_collision(&mut c, &map.definition, &BTreeMap::new()).expect("collision");
    c.finish();
    assert_eq!((stats.terrain_triangles, stats.rail_triangles, stats.dropped), (432, 12, 0));
    let terrain: Vec<_> = c.clusters.iter().filter(|k| k.surface_id == TERRAIN_SURFACE).collect();
    assert_eq!(terrain.len(), stats.terrain_clusters);
    assert!(terrain.iter().all(|k| k.is_terrain() && k.class == 0 && usize::from(k.tri_count) <= CLUSTER_TRIANGLES));
    let rails: Vec<_> = c.clusters.iter().filter(|k| k.class == 12).collect();
    let metal = ssx_content::model::collision::surface_id("metal").expect("a surface");
    assert!(!rails.is_empty() && rails.iter().all(|k| k.is_terrain() && k.surface_id == metal));
    let hit = c.ray_cast(Vec3::new(3.0, 100.0, 47.0), -Vec3::Y, 500.0).expect("ground");
    assert!((hit.point.y - template::ground(47.0)).abs() < 0.02, "{}", hit.point);
    assert!(hit.normal.y > 0.9, "the welded normals face up: {}", hit.normal);
    let first = terrain[0].vert_start as usize;
    assert_eq!(c.nibbles[first], 0x000F_0F0F);
}

#[test]
fn the_triangle_cap_counts_the_placed_objects() {
    let mut map = template_map();
    let id: ssx_content::ContentId = "pack:object/rock".parse().expect("an id");
    map.definition.props.push(ssx_content::map::Prop {
        node: "course/rock".into(),
        look: ssx_content::map::PropLook::Object(id.clone()),
        placement: ssx_content::map::Placement { scale: [1.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], position: [0.0; 3] },
        ao: 1.0,
        gltf_node: 0,
        material: None,
    });
    let mut c = CollisionWorld::default();
    let err = add_map_collision(&mut c, &map.definition, &BTreeMap::new()).expect_err("unresolved");
    assert!(err.to_string().contains("course/rock"), "{err}");
    let part = ssx_content::model::CollisionCluster {
        vertices: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        triangles: vec![[0, 2, 1]; 60_000],
        surface_id: 10,
        group: 0,
        edges: vec![ssx_content::model::TriangleEdges { flags: 0x113, edge_cos: [1.0; 3] }; 60_000],
        material: [0.2, 0.2, 0.1],
    };
    let rock = ssx_content::model::Collision { clusters: vec![part; 31], cylinders: 0 };
    let object = ssx_assets::content::maps::MapObject {
        id: id.clone(),
        source: "pack".into(),
        layer: ssx_content::Layer::ClientMods,
        collision: Some(Arc::new(rock)),
        pob: None,
        model: None,
        audio: None,
    };
    let objects = BTreeMap::from([(id, object)]);
    let err = add_map_collision(&mut c, &map.definition, &objects).expect_err("over the cap");
    assert!(err.to_string().contains("at most 1824266"), "{err}");
    assert!(c.clusters.is_empty(), "refused before anything is added");
}

#[test]
fn the_boundary_grid_reads_as_a_disc_grid() {
    let area = BoundaryArea {
        triangles: vec![[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0]], [[0.0, 0.0], [100.0, 100.0], [0.0, 100.0]]],
        ..BoundaryArea::default()
    };
    let raster = ssx_content::map::boundary_raster(&area).expect("a grid");
    let g = boundary_grid(&raster);
    assert_eq!(g.cell(Vec3::new(50.0, 7.0, 50.0)), 0);
    let far = g.cell(Vec3::new(-500.0, 0.0, 50.0));
    assert_ne!(far & 0xC000, 0);
    let d = crate::rider::boundary::cell_direction(far);
    assert!(d.x > 0.99, "{d}");
}

fn checkpoint(z: f32, order: u32) -> ssx_content::map::Volume {
    let shape = VolumeShape::Sphere { centre: [0.0, 0.0, z], radius: 5.0 };
    ssx_content::map::Volume { node: format!("cp{order}"), name: "checkpoint".into(), order, shape }
}

#[test]
fn the_finish_opens_after_the_gates_in_order() {
    let mut world = crate::testutil::world(CollisionWorld::default());
    let finish = ssx_content::map::Volume {
        node: "finish".into(),
        name: "end".into(),
        order: 0,
        shape: VolumeShape::Box { centre: [0.0, 0.0, 100.0], rotation: [0.0, 0.0, 0.0, 1.0], half_extents: [9.0; 3] },
    };
    world.triggers.volumes.push(trigger_volume(&finish, FINISH_GROUPS, 0));
    let gates = vec![trigger_volume(&checkpoint(20.0, 0), "", 0), trigger_volume(&checkpoint(60.0, 1), "", 1)];
    let mut g = Gates::new(gates, Some(0), Some(30_000));
    let mut rs = Session::default();
    g.reset(&mut rs);
    let in_finish = Vec3::new(0.0, 0.0, 100.0);
    let touches_end = |rs: &mut Session, w: &World| {
        rs.tracker.update(&w.triggers, in_finish);
        rs.tracker.touching_name(&w.triggers, "end")
    };
    assert!(!touches_end(&mut rs, &world), "closed while a gate is left");
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 22.0)).passed, None, "not before the start");
    let run = |rs: &mut Session| {
        rs.event.state = crate::event::EventState::Running;
        rs.event.clock.running = true;
    };
    run(&mut rs);
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 60.0)).passed, None, "the second gate first");
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 22.0)).passed, Some(0));
    let step = g.update(&mut rs, Vec3::new(4.0, 0.0, 60.0));
    assert_eq!((step.passed, step.opened), (Some(1), true));
    assert!(touches_end(&mut rs, &world), "open after the last gate");
    assert!(world.triggers.volumes[0].groups != 0 && world.triggers.volumes[0].interacts != 0);
    rs.event.loops += 1;
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 60.0)).passed, None, "the loop's first gate first");
    assert!(!touches_end(&mut rs, &world) && g.next == 0, "closed again by the loop");
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 22.0)).passed, Some(0));
    rs.reset();
    g.reset(&mut rs);
    assert!(!touches_end(&mut rs, &world) && g.next == 0);
    run(&mut rs);
    rs.event.clock.seconds = 30.0;
    assert!(g.update(&mut rs, Vec3::ZERO).time_up && rs.event.time_up && !rs.event_ended);
    assert!(!g.update(&mut rs, Vec3::ZERO).time_up, "once");
    assert_eq!(g.update(&mut rs, Vec3::new(0.0, 0.0, 22.0)).passed, None, "not after the limit");
    let mut timer = crate::death::DeathTimer::default();
    let out = rs.event.update(&mut timer, &crate::event::EventInputs::default());
    let r = rs.event.result.as_ref().expect("a result");
    assert!(out.results && r.failed && r.time_ms == 30_000 && !rs.event.time_up);
}
