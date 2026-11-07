use std::collections::HashMap;

use ssx_content::ui::asm::{Asm, Push, op, path};
use ssx_content::ui::write::{Tags, movie};

use super::caps;
use crate::avm1::Vm;
use crate::avm1::value::Value;
use crate::display::Player;
use crate::fs::{FileSource, PackageOrigin};
use crate::library::Library;

struct Files {
    files: HashMap<String, Vec<u8>>,
    packages: Vec<String>,
}

impl FileSource for Files {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.files.get(path).cloned()
    }
    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path)
    }
    fn list(&self, _: &str, _: &str) -> Vec<String> {
        Vec::new()
    }
    fn image(&self, path: &str) -> anyhow::Result<crate::image::ImageData> {
        anyhow::bail!("{path}: no images")
    }
    fn package_origin(&self, path: &str) -> Option<PackageOrigin> {
        self.packages.iter().any(|p| p == path).then(|| PackageOrigin {
            id: ssx_content::ui::movie_id(path, false).expect("a movie path"),
            source: "hostile".into(),
        })
    }
}

fn one_frame(code: Vec<u8>) -> Vec<u8> {
    let mut t = Tags::new();
    t.do_action(&code);
    t.show_frame();
    movie(8, 640, 480, 30, &t, false)
}

struct World {
    vm: Vm,
    lib: Library,
    player: Player,
}

fn world(pkg: Vec<u8>, disc: Option<Vec<u8>>) -> World {
    let mut files = HashMap::new();
    files.insert("data/common/fe/views/pkg.gfx".to_string(), pkg);
    if let Some(d) = disc {
        files.insert("data/common/fe/views/disc.gfx".to_string(), d);
    }
    let packages = vec!["data/common/fe/views/pkg.gfx".to_string()];
    let mut lib = Library::new(Box::new(Files { files, packages }));
    let mut player = Player::new();
    let mut vm = Vm::new();
    let m = lib.load_movie("views/pkg.gfx").expect("the package movie loads");
    assert!(lib.origin(m).is_some());
    vm.load_level(&mut lib, &mut player, m, 0);
    if lib.files().exists("data/common/fe/views/disc.gfx") {
        let d = lib.load_movie("views/disc.gfx").expect("the disc movie loads");
        vm.load_level(&mut lib, &mut player, d, 1);
    }
    World { vm, lib, player }
}

impl World {
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.vm.step_frame(&mut self.lib, &mut self.player);
        }
    }

    fn stopped(&self) -> Vec<String> {
        self.vm.sandbox.stopped.values().cloned().collect()
    }

    fn global(&mut self, name: &str) -> Value {
        let g = Value::Object(self.vm.global);
        self.vm.ctx(&mut self.lib, &mut self.player).get_member(&g, name).unwrap_or_default()
    }
}

#[test]
fn an_endless_loop_is_stopped_within_its_frame() {
    let code = Asm::new().label("top").push(&[Push::Bool(true)]).if_true("top").finish();
    let mut w = world(one_frame(code), None);
    w.frames(3);
    let stops = w.stopped();
    assert_eq!(stops.len(), 1, "{stops:?}");
    assert!(stops[0].contains("actions and work units in one frame"), "{stops:?}");
    assert!(w.vm.diag.actions < 2 * caps::WORK_PER_FRAME, "{}", w.vm.diag.actions);
    assert_eq!(w.vm.take_stops().len(), 1);
}

#[test]
fn recursion_ends_at_the_call_depth_on_a_small_stack() {
    let code = Asm::new()
        .function(
            "f",
            &[],
            Asm::new().push(&[Push::Int(0), Push::Str("f".into())]).op(op::CALL_FUNCTION).op(op::RETURN),
        )
        .push(&[Push::Int(0), Push::Str("f".into())])
        .op(op::CALL_FUNCTION)
        .op(op::POP)
        .finish();
    let (depth, stops) = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let mut w = world(one_frame(code), None);
            w.frames(1);
            (w.vm.sandbox.peaks.call_depth, w.stopped())
        })
        .expect("a thread")
        .join()
        .expect("no stack overflow");
    assert_eq!(depth, crate::avm1::interp::MAX_CALL_DEPTH);
    assert_eq!(stops, vec!["256 levels of recursion".to_string()]);
}

#[test]
fn array_and_string_bombs_are_refused() {
    let code = Asm::new()
        .push(&[Push::Str("a".into()), Push::Int(0), Push::Str("Array".into())])
        .op(op::NEW_OBJECT)
        .op(op::SET_VARIABLE)
        .then(path("a"))
        .push(&[Push::Str("length".into()), Push::Num(1e9)])
        .op(op::SET_MEMBER)
        .finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    assert_eq!(w.stopped(), vec!["an array of more than 1 048 576 elements".to_string()]);
    let code = Asm::new()
        .push(&[Push::Str("s".into()), Push::Str("x".into())])
        .op(op::SET_VARIABLE)
        .label("top")
        .push(&[Push::Str("s".into())])
        .then(path("s"))
        .then(path("s"))
        .op(op::ADD2)
        .op(op::SET_VARIABLE)
        .push(&[Push::Bool(true)])
        .if_true("top")
        .finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    assert_eq!(w.stopped(), vec!["a string of more than 1 MiB".to_string()]);
    assert!(w.vm.sandbox.peaks.string_bytes <= caps::STRING_BYTES);
}

#[test]
fn a_display_list_bomb_is_refused() {
    let code = Asm::new()
        .push(&[Push::Str("c".into())])
        .then(path("this"))
        .op(op::SET_VARIABLE)
        .label("top")
        .push(&[Push::Str("c".into()), Push::Int(1), Push::Str("x".into()), Push::Int(2)])
        .then(path("c"))
        .push(&[Push::Str("createEmptyMovieClip".into())])
        .op(op::CALL_METHOD)
        .op(op::SET_VARIABLE)
        .push(&[Push::Bool(true)])
        .if_true("top")
        .finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    assert!(w.player.refused > 0);
    assert!(w.player.peak_nesting < crate::display::MAX_NESTING);
    assert!(w.player.instance_count() <= crate::display::MAX_INSTANCES);
}

#[test]
fn a_stack_bomb_is_stopped() {
    let forty: Vec<Push> = (0..40).map(Push::Int).collect();
    let code = Asm::new().label("top").push(&forty).push(&[Push::Bool(true)]).if_true("top").finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    assert_eq!(w.stopped(), vec!["more than 65 536 values on the stack".to_string()]);
    assert!(w.vm.sandbox.peaks.stack <= caps::STACK_VALUES + 64, "{}", w.vm.sandbox.peaks.stack);
}

#[test]
fn a_wide_display_list_bomb_is_stopped_and_unloaded() {
    let code = Asm::new()
        .push(&[Push::Str("i".into()), Push::Int(0)])
        .op(op::SET_VARIABLE)
        .label("top")
        .push(&[Push::Str("i".into())])
        .then(path("i"))
        .op(op::INCREMENT)
        .op(op::SET_VARIABLE)
        .then(path("i"))
        .push(&[Push::Str("c".into()), Push::Int(2)])
        .then(path("this"))
        .push(&[Push::Str("createEmptyMovieClip".into())])
        .op(op::CALL_METHOD)
        .op(op::POP)
        .push(&[Push::Bool(true)])
        .if_true("top")
        .finish();
    let mut w = world(one_frame(code), Some(one_frame(Asm::new().finish())));
    w.frames(1);
    let stops = w.stopped();
    assert!(stops.len() == 1 && stops[0].contains("actions and work units"), "{stops:?}");
    assert!(w.player.peak_instances > 100 && w.player.peak_instances <= crate::display::MAX_OWNED_INSTANCES + 1);
    assert_eq!(w.player.instance_count(), 2);
    let m = w.player.get(w.player.levels[&0]).map(|i| i.movie).expect("level 0");
    assert_eq!(w.player.owned_by(m), 0);
}

#[test]
fn a_package_movies_own_instances_are_capped() {
    let mut w = world(one_frame(Asm::new().finish()), Some(one_frame(Asm::new().finish())));
    w.frames(1);
    let (pkg_root, disc_root) = (w.player.levels[&0], w.player.levels[&1]);
    let m = w.player.get(pkg_root).map(|i| i.movie).expect("level 0");
    assert_eq!(w.player.owned_by(m), 1, "its level's root");
    w.player.making_for = Some(m);
    let made = (0..crate::display::MAX_OWNED_INSTANCES as i32 + 10)
        .filter(|&i| w.player.create_empty_movie_clip(disc_root, &format!("c{i}"), i).is_some())
        .count();
    w.player.making_for = None;
    assert_eq!(made, crate::display::MAX_OWNED_INSTANCES - 1);
    assert_eq!(w.player.over_cap, vec![m]);
    w.frames(1);
    assert_eq!(w.stopped().len(), 1);
    assert_eq!(w.player.owned_by(m), 0);
    assert_eq!(w.player.instance_count(), 2);
}

#[test]
fn name_lookups_on_a_crowded_clip_are_charged() {
    let mut make = Asm::new().push(&[Push::Str("i".into()), Push::Int(0)]).op(op::SET_VARIABLE).label("make");
    make = make
        .push(&[Push::Str("i".into())])
        .then(path("i"))
        .op(op::INCREMENT)
        .op(op::SET_VARIABLE)
        .then(path("i"))
        .push(&[Push::Str("c".into()), Push::Int(2)])
        .then(path("this"))
        .push(&[Push::Str("createEmptyMovieClip".into())])
        .op(op::CALL_METHOD)
        .op(op::POP)
        .then(path("i"))
        .push(&[Push::Int(10_000)])
        .op(op::LESS2)
        .if_true("make");
    let code = make
        .label("look")
        .then(path("this"))
        .push(&[Push::Str("nothing".into())])
        .op(op::GET_MEMBER)
        .op(op::POP)
        .push(&[Push::Bool(true)])
        .if_true("look")
        .finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    let stops = w.stopped();
    assert!(stops.len() == 1 && stops[0].contains("actions and work units"), "{stops:?}");
    assert!(w.vm.diag.actions < 400_000, "{}", w.vm.diag.actions);
}

#[test]
fn calls_into_the_game_are_checked() {
    let long = "x".repeat(2048);
    let call = |object: &str, method: &str, args: Vec<Push>| {
        let n = args.len() as i32;
        let mut a = Asm::new().push(&args.into_iter().rev().collect::<Vec<_>>()).push(&[Push::Int(n)]);
        a = a.then(path(object)).push(&[Push::Str(method.into())]).op(op::CALL_METHOD).op(op::POP);
        a
    };
    let code = Asm::new()
        .then(call("Dispatcher", "Action", vec![Push::Str("A_GameQuit".into()), Push::Int(0)]))
        .then(call("LocalDataStore", "Subscribe", vec![Push::Str("_level0".into()), Push::Str(long)]))
        .then(call(
            "ResourceHandler",
            "AcquireResource",
            vec![Push::Str("_level0".into()), Push::Int(0), Push::Str("TXT_OK".into())],
        ))
        .then(call("Dispatcher", "DispatchEvent", vec![Push::Num(f64::INFINITY), Push::Str("x".into())]))
        .finish();
    let mut w = world(one_frame(code), None);
    w.frames(1);
    assert_eq!(w.vm.natives.refused, 3);
    let queued = w.vm.natives.take_queue();
    assert_eq!(queued.len(), 1);
    assert_eq!((queued[0].0, queued[0].1), ("ResourceHandler", "AcquireResource"));
    let mut a = Asm::new();
    for _ in 0..caps::CALLS_PER_FRAME + 1 {
        a = a.then(call("InputProvider", "Subscribe", vec![Push::Str("_level0".into())]));
    }
    let mut w = world(one_frame(a.finish()), None);
    w.frames(1);
    assert!(w.stopped()[0].contains("calls into the game"), "{:?}", w.stopped());
}

#[test]
fn a_disc_movie_calling_a_stopped_function_gets_undefined() {
    let pkg = Asm::new()
        .then(path("_global"))
        .push(&[Push::Str("spin".into())])
        .function("", &[], Asm::new().label("top").push(&[Push::Bool(true)]).if_true("top"))
        .op(op::SET_MEMBER)
        .finish();
    let disc = Asm::new()
        .push(&[Push::Str("r".into()), Push::Int(0)])
        .then(path("_global"))
        .push(&[Push::Str("spin".into())])
        .op(op::CALL_METHOD)
        .op(op::SET_VARIABLE)
        .then(path("_global"))
        .push(&[Push::Str("after".into()), Push::Str("went on".into())])
        .op(op::SET_MEMBER)
        .finish();
    let mut w = world(one_frame(pkg), Some(one_frame(disc)));
    w.frames(2);
    assert_eq!(w.stopped().len(), 1);
    assert!(matches!(w.global("after"), Value::String(s) if &*s == "went on"));
}
