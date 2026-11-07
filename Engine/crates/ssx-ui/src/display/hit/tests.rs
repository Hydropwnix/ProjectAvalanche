use std::collections::HashMap;
use std::sync::Arc;

use ssx_formats::gfx::place::PlaceObject;
use ssx_formats::gfx::shape::{DefineShape, ShapeRecord};
use ssx_formats::gfx::types::{Matrix, Rect};

use super::*;
use crate::display::tests::{empty_lib, po, rect_shape};
use crate::library::{FrameDef, FrameTag, MovieDef, MovieId, TimelineDef};

pub(crate) fn hit_movie(lib: &mut Library, root: Vec<PlaceObject>) -> MovieId {
    let id = lib.movies().len();
    let mut chars = HashMap::new();
    chars.insert(1, Character::Shape(Arc::new(rect_shape(1, 200, 200))));
    let mut tri: DefineShape = rect_shape(2, 400, 400);
    tri.shape.records = vec![
        ShapeRecord::StyleChange { move_to: Some((0, 0)), fill0: None, fill1: Some(1), line: None, new_styles: None },
        ShapeRecord::Straight { dx: 0, dy: 400 },
        ShapeRecord::Straight { dx: 400, dy: 0 },
        ShapeRecord::Straight { dx: -400, dy: -400 },
    ];
    tri.bounds = Rect { x_min: 0, x_max: 400, y_min: 0, y_max: 400 };
    chars.insert(2, Character::Shape(Arc::new(tri)));
    for (sprite, shape) in [(20u16, 1u16), (21, 2), (22, 1)] {
        let frames = vec![FrameDef {
            tags: vec![FrameTag::Place(Arc::new(po(1, Some(shape), false, 0)))],
            ..Default::default()
        }];
        chars.insert(sprite, Character::Sprite(Arc::new(TimelineDef { movie: id, id: sprite, frames })));
    }
    let mut scaling_grids = HashMap::new();
    scaling_grids.insert(22, RectF::new(40.0, 40.0, 160.0, 160.0));
    let tags = root.into_iter().map(|p| FrameTag::Place(Arc::new(p))).collect();
    lib.insert_movie(MovieDef {
        path: format!("hit{id}.gfx"),
        dir: String::new(),
        version: 8,
        frame_rate: 30.0,
        stage: RectF::new(0.0, 0.0, 25600.0, 14400.0),
        background: None,
        swf_name: "Hit".into(),
        root: Arc::new(TimelineDef { movie: id, id: 0, frames: vec![FrameDef { tags, ..Default::default() }] }),
        chars,
        exports: HashMap::new(),
        imports: HashMap::new(),
        scaling_grids,
    })
}

pub(crate) fn at(depth: u16, ch: u16, name: &str, x: i32, y: i32, s: f32) -> PlaceObject {
    let mut p = po(depth, Some(ch), false, x * 20);
    p.matrix = Some(Matrix { a: s, d: s, tx: x * 20, ty: y * 20, ..Matrix::IDENTITY });
    p.name = Some(name.into());
    p.flags |= 0x20;
    p
}

fn world(root: Vec<PlaceObject>) -> (Library, Player) {
    let mut lib = empty_lib();
    let m = hit_movie(&mut lib, root);
    let mut player = Player::new();
    player.load_level(&lib, m, 0);
    (lib, player)
}

fn named(player: &Player, name: &str) -> InstanceId {
    let root = player.levels[&0];
    player.child_by_name(root, name, true).expect(name)
}

fn px(x: f32, y: f32) -> (f32, f32) {
    (x * 20.0, y * 20.0)
}

fn all(player: &Player) -> impl FnMut(InstanceId) -> Option<Option<InstanceId>> + use<> {
    let root = player.levels[&0];
    move |i| (i != root).then_some(None)
}

#[test]
fn the_topmost_by_depth_is_picked_and_the_shape_not_the_bounds_decides() {
    let (lib, player) = world(vec![at(1, 20, "a", 0, 0, 1.0), at(2, 20, "b", 5, 0, 1.0), at(3, 21, "t", 100, 0, 1.0)]);
    let (a, b, t) = (named(&player, "a"), named(&player, "b"), named(&player, "t"));
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    assert_eq!(hit.pick(px(2.0, 5.0), &mut all(&player)), Some(a));
    assert_eq!(hit.pick(px(7.0, 5.0), &mut all(&player)), Some(b), "the higher depth wins where both are hit");
    assert_eq!(hit.pick(px(12.0, 5.0), &mut all(&player)), Some(b));
    assert_eq!(hit.pick(px(16.0, 5.0), &mut all(&player)), None);
    assert_eq!(hit.pick(px(103.0, 17.0), &mut all(&player)), Some(t));
    assert_eq!(hit.pick(px(117.0, 3.0), &mut all(&player)), None);
    assert!(hit.bounds(t, px(117.0, 3.0)), "the bounds hold the point the shape misses");
    assert!(!hit.shape(t, px(117.0, 3.0), Visibility::Ignore));
    let mut only_b = |i: InstanceId| (i == b).then_some(None);
    assert_eq!(hit.pick(px(2.0, 5.0), &mut only_b), None);
}

#[test]
fn a_mask_hides_what_it_covers_where_the_point_misses_it() {
    let mut mask = at(1, 20, "m", 0, 0, 1.0);
    mask.clip_depth = Some(2);
    mask.flags |= 0x40;
    let (lib, player) = world(vec![mask, at(2, 20, "c", 0, 0, 4.0), at(3, 20, "d", 0, 100, 1.0)]);
    let (m, c, d) = (named(&player, "m"), named(&player, "c"), named(&player, "d"));
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    assert_eq!(hit.pick(px(5.0, 5.0), &mut all(&player)), Some(c), "inside the mask");
    assert_eq!(hit.pick(px(30.0, 5.0), &mut all(&player)), None, "on c but outside its mask");
    assert!(hit.shown_at(c, px(5.0, 5.0)));
    assert!(!hit.shown_at(c, px(30.0, 5.0)));
    assert!(hit.shown_at(d, px(5.0, 105.0)), "d is not covered");
    let mut only_m = |i: InstanceId| (i == m).then_some(None);
    assert_eq!(hit.pick(px(5.0, 5.0), &mut only_m), None);
}

#[test]
fn invisible_clips_are_not_picked_but_the_shape_test_ignores_visibility() {
    let mut hidden = at(2, 20, "h", 0, 0, 1.0);
    hidden.version = 3;
    hidden.flags3 |= 0x20;
    hidden.visible = Some(0);
    let (lib, player) = world(vec![at(1, 20, "under", 0, 0, 1.0), hidden]);
    let (under, h) = (named(&player, "under"), named(&player, "h"));
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    assert_eq!(
        hit.pick(px(5.0, 5.0), &mut all(&player)),
        Some(under),
        "the invisible clip above lets the pick through"
    );
    assert!(hit.shape(h, px(5.0, 5.0), Visibility::Ignore), "hitTest(x, y, true) does not look at _visible");
    assert!(!hit.shape(h, px(5.0, 5.0), Visibility::Respect));
    assert!(!hit.shown_at(h, px(5.0, 5.0)));
}

#[test]
fn a_hit_area_replaces_the_clips_own_shape() {
    let (lib, player) = world(vec![at(1, 20, "btn", 0, 0, 1.0), at(2, 20, "area", 100, 0, 1.0)]);
    let (btn, area) = (named(&player, "btn"), named(&player, "area"));
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    let mut with_area = |i: InstanceId| (i == btn).then_some(Some(area));
    assert_eq!(hit.pick(px(5.0, 5.0), &mut with_area), None, "its own shape no longer counts");
    assert_eq!(hit.pick(px(105.0, 5.0), &mut with_area), Some(btn), "the hit area's does");
}

#[test]
fn a_nine_slice_sprite_is_hit_where_it_is_drawn() {
    let mut p = at(1, 22, "panel", 0, 0, 1.0);
    p.matrix = Some(Matrix { a: 4.0, d: 1.0, ..Matrix::IDENTITY });
    let (lib, player) = world(vec![p]);
    let panel = named(&player, "panel");
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    assert!(hit.shape(panel, px(1.0, 5.0), Visibility::Respect));
    assert!(hit.shape(panel, px(39.0, 5.0), Visibility::Respect));
    assert!(!hit.shape(panel, px(41.0, 5.0), Visibility::Respect));
    assert_eq!(hit.pick(px(20.0, 9.0), &mut all(&player)), Some(panel));
}

#[test]
fn a_degenerate_matrix_hits_nothing() {
    let mut p = at(1, 20, "flat", 0, 0, 1.0);
    p.matrix = Some(Matrix { a: 0.0, d: 0.0, ..Matrix::IDENTITY });
    let (lib, player) = world(vec![p]);
    let flat = named(&player, "flat");
    let mut meshes = HitMeshes::default();
    let mut hit = Hit { player: &player, lib: &lib, meshes: &mut meshes };
    assert!(!hit.shape(flat, (0.0, 0.0), Visibility::Ignore));
    assert_eq!(hit.pick((0.0, 0.0), &mut all(&player)), None);
    assert_eq!(player.to_local(flat, (0.0, 0.0)), None);
}
