use super::*;
use crate::display::hit::tests::{at, hit_movie};
use crate::display::tests::empty_lib;
use crate::library::Library;

fn log(cx: &mut Ctx, this: &Value, event: &str) {
    let name = match this {
        Value::Undefined => String::new(),
        _ => cx
            .get_member(this, "_name")
            .ok()
            .and_then(|n| cx.to_string(&n).ok())
            .map(|s| s.to_string())
            .unwrap_or_default(),
    };
    let line = Value::str(&format!("{name}.{event}"));
    let global = Value::Object(cx.vm.global);
    let list = cx.get_member(&global, "log").unwrap_or_default();
    if let Some(o) = list.as_object().and_then(|o| cx.vm.heap.get_mut(o))
        && let Kind::Array(v) = &mut o.kind
    {
        v.push(line);
    }
}

macro_rules! handler {
    ($f:ident, $e:literal) => {
        fn $f(cx: &mut Ctx, this: Value, _: &[Value]) -> Result<Value, super::super::Fault> {
            log(cx, &this, $e);
            Ok(Value::Undefined)
        }
    };
}
handler!(press, "press");
handler!(release, "release");
handler!(release_outside, "releaseOutside");
handler!(roll_over, "rollOver");
handler!(roll_out, "rollOut");
handler!(drag_over, "dragOver");
handler!(drag_out, "dragOut");

fn mouse_move(cx: &mut Ctx, this: Value, _: &[Value]) -> Result<Value, super::super::Fault> {
    let root = Value::Object(cx.object_for(cx.player.levels[&0]));
    let probe = cx.get_member(&root, "probe")?;
    let (x, y) = (cx.get_member(&probe, "_xmouse")?, cx.get_member(&probe, "_ymouse")?);
    cx.set_member(&this, "x", x)?;
    cx.set_member(&this, "y", y)?;
    log(cx, &Value::Undefined, "mouseMove");
    Ok(Value::Undefined)
}

fn mouse_down(cx: &mut Ctx, _: Value, _: &[Value]) -> Result<Value, super::super::Fault> {
    log(cx, &Value::Undefined, "mouseDown");
    Ok(Value::Undefined)
}

struct World {
    vm: Vm,
    lib: Library,
    player: Player,
}

impl World {
    fn new() -> World {
        let mut lib = empty_lib();
        let m = hit_movie(
            &mut lib,
            vec![at(1, 20, "a", 0, 0, 1.0), at(2, 20, "b", 20, 0, 1.0), at(3, 20, "probe", 100, 50, 2.0)],
        );
        let mut vm = Vm::new();
        let mut player = Player::new();
        vm.load_level(&mut lib, &mut player, m, 0);
        let mut w = World { vm, lib, player };
        {
            let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
            let list = Value::Object(cx.new_array(Vec::new()));
            let global = Value::Object(cx.vm.global);
            cx.set_member(&global, "log", list).unwrap();
        }
        for name in ["a", "b"] {
            let clip = w.clip(name);
            let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
            let obj = Value::Object(cx.object_for(clip));
            let handlers: [(&str, super::super::object::NativeFn); 7] = [
                ("onPress", press),
                ("onRelease", release),
                ("onReleaseOutside", release_outside),
                ("onRollOver", roll_over),
                ("onRollOut", roll_out),
                ("onDragOver", drag_over),
                ("onDragOut", drag_out),
            ];
            for (n, f) in handlers {
                let f = Value::Object(cx.new_native(f));
                cx.set_member(&obj, n, f).unwrap();
            }
        }
        w
    }

    fn clip(&self, name: &str) -> InstanceId {
        self.player.child_by_name(self.player.levels[&0], name, true).expect(name)
    }

    fn send(&mut self, events: &[PointerEvent]) -> Vec<String> {
        for e in events {
            self.vm.pointer_event(*e);
        }
        self.vm.advance(&mut self.lib, &mut self.player, 0.0);
        let mut cx = self.vm.ctx(&mut self.lib, &mut self.player);
        let global = Value::Object(cx.vm.global);
        let list = cx.get_member(&global, "log").unwrap();
        let out = match list.as_object().and_then(|o| cx.vm.heap.get_mut(o)).map(|o| &mut o.kind) {
            Some(Kind::Array(v)) => std::mem::take(v),
            _ => Vec::new(),
        };
        out.iter().map(|v| cx.to_string(v).unwrap().to_string()).collect()
    }
}

fn mv(x: f32, y: f32) -> PointerEvent {
    PointerEvent::Move { x, y }
}

const DOWN: PointerEvent = PointerEvent::Button { down: true };
const UP: PointerEvent = PointerEvent::Button { down: false };

#[test]
fn roll_over_press_and_release_inside() {
    let mut w = World::new();
    assert_eq!(w.send(&[mv(5.0, 5.0)]), ["a.rollOver"]);
    assert_eq!(w.send(&[DOWN]), ["a.press"]);
    assert_eq!(w.send(&[UP]), ["a.release"]);
    assert_eq!(w.send(&[mv(25.0, 5.0)]), ["a.rollOut", "b.rollOver"]);
    assert_eq!(w.send(&[mv(50.0, 5.0)]), ["b.rollOut"]);
    assert_eq!(w.send(&[DOWN, UP]), Vec::<String>::new(), "a press over nothing");
}

#[test]
fn a_pressed_clip_captures_the_pointer_and_hears_the_release_outside() {
    let mut w = World::new();
    w.send(&[mv(5.0, 5.0)]);
    assert_eq!(w.send(&[DOWN]), ["a.press"]);
    assert_eq!(w.send(&[mv(25.0, 5.0)]), ["a.dragOut"]);
    assert_eq!(w.send(&[mv(5.0, 5.0)]), ["a.dragOver"]);
    assert_eq!(w.send(&[mv(25.0, 5.0)]), ["a.dragOut"]);
    assert_eq!(w.send(&[UP]), ["a.releaseOutside", "b.rollOver"]);
    assert_eq!(w.send(&[DOWN, UP]), ["b.press", "b.release"]);
}

#[test]
fn a_disabled_clip_hears_nothing_and_still_covers_the_one_below() {
    let mut w = World::new();
    let a = w.clip("a");
    let b = w.clip("b");
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let obj = Value::Object(cx.object_for(b));
        cx.set_member(&obj, "enabled", Value::Bool(false)).unwrap();
        cx.set_member(&obj, "_x", Value::Number(0.0)).unwrap();
    }
    assert_eq!(w.send(&[mv(5.0, 5.0), DOWN, UP]), Vec::<String>::new(), "b is on top and disabled");
    let picked = w.vm.ctx(&mut w.lib, &mut w.player).pointer_pick();
    assert_eq!(picked, Some(b));
    assert_ne!(picked, Some(a));
    assert_eq!(w.vm.pointer_cursor(&mut w.lib, &mut w.player), Cursor::Arrow, "no hand over a disabled button");
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let obj = Value::Object(cx.object_for(b));
        cx.set_member(&obj, "enabled", Value::Bool(true)).unwrap();
    }
    assert_eq!(w.vm.pointer_cursor(&mut w.lib, &mut w.player), Cursor::Hand);
    assert_eq!(w.send(&[DOWN, UP]), ["b.press", "b.release"]);
}

#[test]
fn an_invisible_button_lets_the_pointer_through() {
    let mut w = World::new();
    let b = w.clip("b");
    w.player.set_visible(b, false);
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let obj = Value::Object(cx.object_for(b));
        cx.set_member(&obj, "_x", Value::Number(0.0)).unwrap();
    }
    assert_eq!(w.send(&[mv(5.0, 5.0), DOWN, UP]), ["a.rollOver", "a.press", "a.release"]);
}

#[test]
fn mouse_listeners_and_xmouse_in_the_clips_coordinates() {
    let mut w = World::new();
    let listener = {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let l = Value::Object(cx.new_object());
        let (m, d) = (Value::Object(cx.new_native(mouse_move)), Value::Object(cx.new_native(mouse_down)));
        cx.set_member(&l, "onMouseMove", m).unwrap();
        cx.set_member(&l, "onMouseDown", d).unwrap();
        let global = Value::Object(cx.vm.global);
        let mouse = cx.get_member(&global, "Mouse").unwrap();
        let (add, _) = cx.get_method(&mouse, "addListener").unwrap();
        cx.call_top(&add, mouse, std::slice::from_ref(&l));
        l
    };
    assert_eq!(w.send(&[mv(110.0, 61.0)]), [".mouseMove"]);
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        assert!(matches!(cx.get_member(&listener, "x").unwrap(), Value::Number(n) if n == 5.0));
        assert!(matches!(cx.get_member(&listener, "y").unwrap(), Value::Number(n) if n == 5.5));
    }
    w.send(&[mv(105.03, 50.0)]);
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        assert!(matches!(cx.get_member(&listener, "x").unwrap(), Value::Number(n) if n == 2.5));
    }
    assert_eq!(w.send(&[DOWN]), [".mouseDown"], "the listener first; no button under the pointer");
}

#[test]
fn mouse_hide_and_show_return_the_old_visibility_and_set_the_cursor() {
    let mut w = World::new();
    let mouse = {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let global = Value::Object(cx.vm.global);
        cx.get_member(&global, "Mouse").unwrap()
    };
    let call = |cx: &mut Ctx, m: &str| {
        let (f, _) = cx.get_method(&mouse, m).unwrap();
        cx.call_top(&f, mouse.clone(), &[])
    };
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        assert!(matches!(call(&mut cx, "hide"), Value::Number(n) if n == 1.0));
        assert!(matches!(call(&mut cx, "hide"), Value::Number(n) if n == 0.0));
    }
    assert_eq!(w.vm.pointer_cursor(&mut w.lib, &mut w.player), Cursor::Hidden);
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        assert!(matches!(call(&mut cx, "show"), Value::Number(n) if n == 0.0));
        assert!(matches!(call(&mut cx, "show"), Value::Number(n) if n == 1.0));
    }
    assert_eq!(w.vm.pointer_cursor(&mut w.lib, &mut w.player), Cursor::Arrow);
}

#[test]
fn hit_test_point_forms_and_the_queue_bound() {
    let mut w = World::new();
    let a = w.clip("a");
    {
        let mut cx = w.vm.ctx(&mut w.lib, &mut w.player);
        let obj = Value::Object(cx.object_for(a));
        let (f, _) = cx.get_method(&obj, "hitTest").unwrap();
        let mut t = |args: &[Value]| matches!(cx.call_top(&f, obj.clone(), args), Value::Bool(true));
        let n = Value::Number;
        assert!(t(&[n(5.0), n(5.0), Value::Bool(true)]));
        assert!(!t(&[n(15.0), n(5.0), Value::Bool(true)]));
        assert!(t(&[n(10.0), n(10.0)]), "the bounds' edge");
        assert!(!t(&[n(f64::NAN), n(5.0)]));
    }
    for i in 0..1000 {
        w.vm.pointer_event(PointerEvent::Button { down: i % 2 == 0 });
    }
    assert_eq!(w.vm.pointer.queue.len(), MAX_QUEUED);
    for i in 0..10 {
        w.vm.pointer_event(mv(i as f32, 0.0));
    }
    assert_eq!(w.vm.pointer.queue.back(), Some(&mv(9.0, 0.0)), "consecutive moves keep the last");
    assert_eq!(w.vm.pointer.queue.len(), MAX_QUEUED);
}
