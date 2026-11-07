use std::collections::VecDeque;

use super::object::{Kind, ObjId};
use super::value::Value;
use super::{Ctx, Vm};
use crate::display::hit::{Hit, HitMeshes};
use crate::display::{InstanceId, Player};
use crate::library::Library;

pub const BUTTON_EVENTS: [&str; 7] =
    ["onPress", "onRelease", "onReleaseOutside", "onRollOver", "onRollOut", "onDragOver", "onDragOut"];

const MAX_QUEUED: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerEvent {
    Move { x: f32, y: f32 },
    Button { down: bool },
}

#[derive(Default)]
pub struct Pointer {
    pub position: (f32, f32),
    pub down: bool,
    pub hovered: Option<InstanceId>,
    pub pressed: Option<InstanceId>,
    pressed_inside: bool,
    queue: VecDeque<PointerEvent>,
    pub hidden: bool,
    pub meshes: HitMeshes,
}

impl Pointer {
    fn target(&self) -> Option<InstanceId> {
        match self.pressed {
            Some(p) if self.pressed_inside => Some(p),
            Some(_) => None,
            None => self.hovered,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    Arrow,
    Hand,
    Hidden,
}

impl Vm {
    // TODO(guess): the port runs the queued pointer events at the start of the next advance, moves merged and at most 256 kept; Scaleform's HandleEvent (0x82646888) feeds its mouse state (0x82667938 move, 0x82667978 buttons) and the AS2 root reads it in `Advance` (0x82651308), where it was not traced; settle: the mouse state's queue and its reader in 0x82651308.
    pub fn pointer_event(&mut self, e: PointerEvent) {
        let q = &mut self.pointer.queue;
        if let (PointerEvent::Move { .. }, Some(PointerEvent::Move { .. })) = (e, q.back()) {
            q.pop_back();
        }
        if q.len() >= MAX_QUEUED {
            q.pop_front();
        }
        q.push_back(e);
    }

    pub fn pointer_cursor(&mut self, lib: &mut Library, player: &mut Player) -> Cursor {
        if self.pointer.hidden {
            return Cursor::Hidden;
        }
        let Some(t) = self.pointer.target() else { return Cursor::Arrow };
        let mut cx = self.ctx(lib, player);
        let enabled = cx.clip_flag(t, "enabled");
        if enabled && cx.clip_flag(t, "useHandCursor") { Cursor::Hand } else { Cursor::Arrow }
    }
}

impl Ctx<'_> {
    pub(crate) fn run_pointer(&mut self) {
        while let Some(e) = self.vm.pointer.queue.pop_front() {
            match e {
                PointerEvent::Move { x, y } => {
                    self.vm.pointer.position = (x * 20.0, y * 20.0);
                    self.broadcast_mouse("onMouseMove", &[]);
                    self.pointer_update();
                }
                PointerEvent::Button { down: true } => {
                    if self.vm.pointer.down {
                        continue;
                    }
                    self.vm.pointer.down = true;
                    self.broadcast_mouse("onMouseDown", &[]);
                    self.pointer_update();
                    if let Some(h) = self.vm.pointer.hovered {
                        self.vm.pointer.pressed = Some(h);
                        self.vm.pointer.pressed_inside = true;
                        self.button_event(h, "onPress");
                    }
                }
                PointerEvent::Button { down: false } => {
                    if !self.vm.pointer.down {
                        continue;
                    }
                    self.vm.pointer.down = false;
                    self.broadcast_mouse("onMouseUp", &[]);
                    self.pointer_release();
                }
            }
        }
    }

    pub fn pointer_pick(&mut self) -> Option<InstanceId> {
        let mut meshes = std::mem::take(&mut self.vm.pointer.meshes);
        let position = self.vm.pointer.position;
        let vm = &*self.vm;
        let player = &*self.player;
        let picked =
            Hit { player, lib: self.lib, meshes: &mut meshes }.pick(position, &mut |id| button_mode(vm, player, id));
        self.vm.pointer.meshes = meshes;
        picked
    }

    fn pointer_update(&mut self) {
        let picked = self.pointer_pick();
        if let Some(p) = self.vm.pointer.pressed {
            let inside = picked == Some(p);
            if inside != self.vm.pointer.pressed_inside {
                self.vm.pointer.pressed_inside = inside;
                self.button_event(p, if inside { "onDragOver" } else { "onDragOut" });
            }
            return;
        }
        let old = self.vm.pointer.hovered;
        if old != picked {
            self.vm.pointer.hovered = picked;
            if let Some(o) = old {
                self.button_event(o, "onRollOut");
            }
            if let Some(n) = picked {
                self.button_event(n, "onRollOver");
            }
        }
    }

    fn pointer_release(&mut self) {
        let Some(p) = self.vm.pointer.pressed.take() else { return };
        let picked = self.pointer_pick();
        if self.vm.pointer.pressed_inside && picked == Some(p) {
            self.vm.pointer.hovered = Some(p);
            self.button_event(p, "onRelease");
        } else {
            self.vm.pointer.hovered = picked;
            self.button_event(p, "onReleaseOutside");
            if let Some(n) = picked {
                self.button_event(n, "onRollOver");
            }
        }
        self.vm.pointer.pressed_inside = false;
    }

    fn button_event(&mut self, clip: InstanceId, name: &str) {
        if self.player.get(clip).is_none() || !self.clip_flag(clip, "enabled") {
            return;
        }
        let obj = Value::Object(self.object_for(clip));
        let Ok((f, _)) = self.get_method(&obj, name) else { return };
        if self.is_function(&f) {
            self.call_top(&f, obj, &[]);
        }
    }

    fn clip_flag(&mut self, clip: InstanceId, name: &str) -> bool {
        let obj = Value::Object(self.object_for(clip));
        match self.get_member(&obj, name) {
            Ok(Value::Undefined) | Err(_) => true,
            Ok(v) => self.to_bool(&v),
        }
    }

    fn broadcast_mouse(&mut self, name: &str, args: &[Value]) {
        let global = Value::Object(self.vm.global);
        let Ok(mouse) = self.get_member(&global, "Mouse") else { return };
        let Ok(list) = self.get_member(&mouse, "_listeners") else { return };
        let listeners: Vec<Value> = match list.as_object().and_then(|o| self.vm.heap.get(o)).map(|o| &o.kind) {
            Some(Kind::Array(v)) => v.clone(),
            _ => return,
        };
        for l in listeners {
            let Ok((f, _)) = self.get_method(&l, name) else { continue };
            if self.is_function(&f) {
                self.call_top(&f, l, args);
            }
        }
    }
}

fn button_mode(vm: &Vm, player: &Player, id: InstanceId) -> Option<Option<InstanceId>> {
    let obj = *vm.display_objs.get(&id)?;
    if !BUTTON_EVENTS.iter().any(|n| has_member(vm, obj, n)) {
        return None;
    }
    let area = member_value(vm, obj, "hitArea").and_then(|v| display_target(vm, player, &v)).filter(|a| *a != id);
    Some(area)
}

const MAX_CHAIN: usize = 256;

fn has_member(vm: &Vm, obj: ObjId, name: &str) -> bool {
    member_value(vm, obj, name).is_some()
}

fn member_value(vm: &Vm, obj: ObjId, name: &str) -> Option<Value> {
    let mut cur = Some(obj);
    for _ in 0..MAX_CHAIN {
        let o = vm.heap.get(cur?)?;
        if let Some(p) = o.props.get(name) {
            return Some(if p.accessor.is_some() { Value::Undefined } else { p.value.clone() });
        }
        cur = o.proto.as_object();
    }
    None
}

fn display_target(vm: &Vm, player: &Player, v: &Value) -> Option<InstanceId> {
    let Kind::Display(d) = &vm.heap.get(v.as_object()?)?.kind else { return None };
    if player.get(d.id).is_some() {
        return Some(d.id);
    }
    let mut parts = d.path.split('.');
    let level: i32 = parts.next()?.strip_prefix("_level")?.parse().ok()?;
    let mut cur = *player.levels.get(&level)?;
    for p in parts {
        cur = player.child_by_name(cur, p, true)?;
    }
    Some(cur)
}

#[cfg(test)]
mod tests;
