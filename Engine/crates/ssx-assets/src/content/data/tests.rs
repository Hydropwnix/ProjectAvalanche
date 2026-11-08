use ssx_content::data::Class;
use ssx_content::policy::ClientMods;
use ssx_content::{Asset, ContentId, Layer, MemorySource};

use super::super::store::GameContent;

fn disc() -> toml::Table {
    toml::toml! { x = 1.0
    y = 2 }
}

fn patch(text: &str) -> toml::Table {
    text.parse().unwrap()
}

fn store(key: &str, id: &ContentId, mounts: Vec<(Layer, i32, MemorySource)>) -> std::sync::Arc<GameContent> {
    let root = std::env::temp_dir().join(format!("ssx-m9-{key}-{}-absent", std::process::id()));
    let content = GameContent::of(&root);
    let mut r = content.registry.write().unwrap();
    r.mount(Layer::Base, 0, Box::new(MemorySource::new("ssx").with(id.clone(), Asset::Record(disc()))));
    for (layer, priority, source) in mounts {
        r.mount(layer, priority, Box::new(source));
    }
    drop(r);
    content
}

#[test]
fn the_policy_switches_client_mods_off_for_the_simulation() {
    let id: ContentId = "ssx:tuning/a/b".parse().unwrap();
    let content = store(
        "policy",
        &id,
        vec![
            (Layer::ClientMods, 0, MemorySource::new("client").with_patch(id.clone(), patch("x = 5.0"))),
            (Layer::Server, 0, MemorySource::new("server").with_patch(id.clone(), patch("y = 3"))),
        ],
    );
    let x_y = |t: &toml::Table| (t["x"].as_float().unwrap(), t["y"].as_integer().unwrap());
    let read = |p: ClientMods, class: Class| {
        content.set_sim_policy(p);
        x_y(&content.record_for(&id, disc(), class).unwrap().0)
    };
    assert_eq!(read(ClientMods::Any, Class::Simulation), (5.0, 3));
    assert_eq!(read(ClientMods::Presentation, Class::Simulation), (1.0, 3));
    assert_eq!(read(ClientMods::None, Class::Simulation), (1.0, 3));
    assert_eq!(read(ClientMods::Presentation, Class::Presentation), (5.0, 3));
    assert_eq!(read(ClientMods::None, Class::Presentation), (1.0, 3));
    content.set_sim_policy(ClientMods::Any);
}

#[test]
fn the_join_compares_under_the_id_basis_on_its_thread_alone() {
    let id: ContentId = "ssx:tuning/a/d".parse().unwrap();
    let content = store(
        "basis",
        &id,
        vec![
            (Layer::ClientMods, 0, MemorySource::new("mydata").with_patch(id.clone(), patch("x = 5.0"))),
            (Layer::Server, 0, MemorySource::new("rules").with_patch(id.clone(), patch("y = 3"))),
        ],
    );
    let x_y = |c: &GameContent| {
        let t = c.sim_record(&id, disc()).unwrap().0;
        (t["x"].as_float().unwrap(), t["y"].as_integer().unwrap())
    };
    content.set_sim_policy(ClientMods::Any);
    assert_eq!(ClientMods::Any.id_basis(), ClientMods::Presentation);
    assert_eq!(content.layer_of(&id), "the game's disc, patched by your mod mydata, the server's resource rules");
    let (inside, layer, ride) = content.with_sim_policy(ClientMods::Any.id_basis(), || {
        let ride = std::thread::scope(|s| s.spawn(|| x_y(&content)).join().unwrap());
        (x_y(&content), content.layer_of(&id), ride)
    });
    assert_eq!(inside, (1.0, 3), "the join's thread reads the server's content");
    assert_eq!(layer, "the game's disc, patched by the server's resource rules");
    assert_eq!(ride, (5.0, 3), "the ride's thread keeps the session's policy");
    assert_eq!(x_y(&content), (5.0, 3), "the policy before is back");
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        content.with_sim_policy(ClientMods::None, || panic!("a comparison that unwinds"))
    }));
    assert!(unwound.is_err());
    assert_eq!(content.sim_policy(), ClientMods::Any, "back after an unwind too");
}

#[test]
fn a_tracks_decision_is_kept_per_policy() {
    let content = store("decisions", &"ssx:tuning/a/e".parse().unwrap(), Vec::new());
    let generation = content.generation();
    let cell = |p: ClientMods| content.track_decision("alps0101", generation, p as u8);
    let (any, basis) = (cell(ClientMods::Any), cell(ClientMods::Any.id_basis()));
    assert!(!std::sync::Arc::ptr_eq(&any, &basis), "one cell per policy");
    assert!(std::sync::Arc::ptr_eq(&any, &cell(ClientMods::Any)), "the same policy's cell again");
    assert!(std::sync::Arc::ptr_eq(&basis, &cell(ClientMods::Presentation)));
    assert!(!std::sync::Arc::ptr_eq(&basis, &cell(ClientMods::None)));
    assert!(content.decisions().is_empty(), "no decision made yet");
}

#[test]
fn patches_of_one_record_merge_in_priority_order() {
    let id: ContentId = "ssx:tuning/a/c".parse().unwrap();
    let content = store(
        "order",
        &id,
        vec![
            (Layer::ClientMods, 1, MemorySource::new("later").with_patch(id.clone(), patch("x = 1.2"))),
            (Layer::ClientMods, 0, MemorySource::new("earlier").with_patch(id.clone(), patch("x = 1.25\ny = 7"))),
        ],
    );
    let (t, source, layer) = content.sim_record(&id, disc()).unwrap();
    assert_eq!((t["x"].as_float(), t["y"].as_integer()), (Some(1.2), Some(7)));
    assert_eq!(layer, Layer::Base);
    assert_eq!(source, "ssx, patched by earlier, later");
}

#[test]
fn records_the_readers_refuse_are_put_back_one_by_one() {
    let mut state = vec![0_i32; 3];
    let items = vec![(0_usize, 5_i32), (1, 9), (2, 4), (7, 1), (1, 1)];
    let set = |s: &mut Vec<i32>, &(i, v): &(usize, i32)| s.get_mut(i).map(|x| std::mem::replace(x, v));
    let restore = |s: &mut Vec<i32>, &(i, _): &(usize, i32), before: i32| s[i] = before;
    let reads = |s: &Vec<i32>| {
        let sum: i32 = s.iter().sum();
        if sum <= 10 { Ok(()) } else { Err(format!("sum {sum}")) }
    };
    let (kept, refused) = super::tuning::keep_each(&mut state, items, set, restore, reads);
    assert_eq!(kept, vec![(0, 5), (2, 4), (1, 1)]);
    assert_eq!(refused, vec![((1, 9), "sum 14".to_string()), ((7, 1), "the bundle has no such asset".to_string())]);
    assert_eq!(state, vec![5, 1, 4]);
}
