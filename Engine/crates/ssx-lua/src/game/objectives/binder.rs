use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use super::node::{ObjectiveMessageWait, StatWait, StatsBlock};
use super::{
    Cmp, Do, Effect, FloatBinding, GsType, GsValue, Name, NisActorBinding, NisRequest, Node, Objective, Runner, SetId,
    StatKey, Wait, World, fnv1, game_state_type,
};
use crate::game::Binder;
use crate::game::bindings::Binding;
use crate::game::record::{Hook, Rec, Recorder, world_state_hooked};
use crate::number::to_int;
use crate::value::Value;
use crate::vm::{Lua, LuaError, LuaResult};

#[derive(Clone, Debug, Default)]
pub struct BatchReport {
    pub scripts: Vec<(String, Option<String>)>,
    pub objectives: usize,
    pub overrides: usize,
    pub nodes: BTreeMap<&'static str, usize>,
    pub not_ported: BTreeMap<&'static str, usize>,
    pub problems: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cur {
    Pending(usize),
    Set(usize, usize),
}

pub struct Builder<W: World> {
    pub runner: Runner,
    pub set: SetId,
    pub world: W,
    pub report: BatchReport,
    script: String,
    current: Option<Cur>,
    pending: Vec<Objective>,
    open_block: Option<(Cur, usize)>,
}

pub fn load_batch<W: World + 'static>(
    runner: Runner,
    set: SetId,
    scripts: &[(String, Vec<u8>)],
    world: W,
    rec: Option<&Rec>,
) -> (Runner, W, BatchReport) {
    load_batch_with(runner, set, scripts, world, rec, &|_| {})
}

pub fn load_batch_with<W: World + 'static>(
    runner: Runner,
    set: SetId,
    scripts: &[(String, Vec<u8>)],
    world: W,
    rec: Option<&Rec>,
    setup: &dyn Fn(&mut Lua),
) -> (Runner, W, BatchReport) {
    let b = Rc::new(RefCell::new(Builder {
        runner,
        set,
        world,
        report: BatchReport::default(),
        script: String::new(),
        current: None,
        pending: Vec::new(),
        open_block: None,
    }));
    let own_rec: Rec = Rc::new(RefCell::new(Recorder::default()));
    let rec = rec.unwrap_or(&own_rec);
    let hb = b.clone();
    let hook: Hook = Rc::new(move |l, binding, args| hb.borrow_mut().call(l, binding, args));
    let mut l = world_state_hooked(Binder::Objectives, rec, true, Some(hook));
    setup(&mut l);
    for (name, src) in scripts {
        b.borrow_mut().script = name.clone();
        let r = l.exec(src, name).err().map(|e| e.message());
        b.borrow_mut().report.scripts.push((name.clone(), r));
    }
    drop(l);
    let Ok(cell) = Rc::try_unwrap(b) else { unreachable!("the Lua state that held the builder is gone") };
    let mut bd = cell.into_inner();
    for o in &bd.pending {
        bd.report.problems.push(format!("objective {} begun, never ended ({})", o.name, o.script));
    }
    (bd.runner, bd.world, bd.report)
}

fn name(l: &Lua, a: &[Value], i: usize) -> LuaResult<Name> {
    let s = l.check_str(a, i)?;
    Ok(Name { hash: fnv1(s.bytes()), text: Arc::from(s.to_str_lossy().as_str()) })
}

fn opt_name(l: &Lua, a: &[Value], i: usize) -> LuaResult<Option<Name>> {
    match a.get(i - 1) {
        None | Some(Value::Nil) => Ok(None),
        _ => name(l, a, i).map(Some),
    }
}

fn num(l: &Lua, a: &[Value], i: usize) -> LuaResult<f32> {
    Ok(l.check_num(a, i)? as f32)
}

fn to_num(a: &[Value], i: usize) -> f32 {
    a.get(i - 1).and_then(Value::to_num).unwrap_or(0.0) as f32
}

fn to_integer(a: &[Value], i: usize) -> i32 {
    a.get(i - 1).and_then(Value::to_num).map_or(0, to_int)
}

fn to_bool(a: &[Value], i: usize) -> bool {
    a.get(i - 1).is_some_and(Value::truthy)
}

fn text(l: &Lua, a: &[Value], i: usize) -> LuaResult<String> {
    Ok(l.check_str(a, i)?.to_str_lossy())
}

fn cmp(l: &Lua, a: &[Value], i: usize) -> LuaResult<Cmp> {
    let v = l.check_int(a, i)?;
    Cmp::from_i32(v).ok_or_else(|| l.rt_error(&format!("comparison {v} out of the tables (0…5)")))
}

impl<W: World> Builder<W> {
    fn objective(&mut self, c: Cur) -> &mut Objective {
        match c {
            Cur::Pending(k) => &mut self.pending[k],
            Cur::Set(s, i) => &mut self.runner.sets[s][i],
        }
    }

    fn current(&mut self, l: &Lua) -> LuaResult<Cur> {
        self.current.ok_or_else(|| l.rt_error("no current objective (BeginObjective missing)"))
    }

    fn append(&mut self, l: &Lua, b: &'static Binding, n: Node) -> LuaResult<()> {
        let c = self.current(l)?;
        *self.report.nodes.entry(b.name).or_default() += 1;
        let o = self.objective(c);
        if let Some(last) = o.nodes.last()
            && !last.complete()
            && matches!(last, Node::Stats(_))
        {
            let what = format!("{} swallowed by the open stats block in {} ({})", b.name, o.name, o.script);
            if !matches!(b.name, "WAIT_IsStat" | "WAIT_IsLocationUniqueStat" | "WAIT_IsBadgeSet" | "StatsEnd") {
                self.report.problems.push(what);
            }
        }
        self.objective(c).append(n);
        Ok(())
    }

    fn append_stat(&mut self, l: &Lua, b: &'static Binding, w: Wait) -> LuaResult<()> {
        self.append(l, b, Node::Wait(w.clone()))?;
        if let Some((c, k)) = self.open_block
            && let Some(Node::Stats(s)) = self.objective(c).nodes.get_mut(k)
        {
            s.waits.push(w);
        }
        Ok(())
    }

    fn call(&mut self, l: &mut Lua, b: &'static Binding, a: &[Value]) -> LuaResult<Option<Box<dyn std::any::Any>>> {
        let d = |e: Effect| Node::Do(Do::Effect(e));
        let node = match b.name {
            "BeginObjective" => return self.begin(l, a, false),
            "OverrideObjective" => return self.begin(l, a, true),
            "EndObjective" => return self.end(l).map(|_| None),
            "AND" | "OR" => {
                let c = self.current(l)?;
                let Some(prev) = self.objective(c).nodes.pop() else {
                    return Err(l.rt_error(&format!("{}() with no node before it", b.name)));
                };
                let a = Box::new(prev);
                if b.name == "AND" {
                    Node::And { a, b: None, a_held: false }
                } else {
                    Node::Or { a, b: None, a_held: false, b_held: false }
                }
            }
            "NOT" => Node::Not(None),
            "DO_AIChaseTarget" => d(Effect::AiChaseTarget { entity: name(l, a, 1)?, target: name(l, a, 2)? }),
            "DO_AIMoveToLocator" => {
                let opt = |i: usize| -> LuaResult<f32> {
                    match a.get(i - 1) {
                        None | Some(Value::Nil) => Ok(0.0),
                        _ => num(l, a, i),
                    }
                };
                let (entity, locator) = (name(l, a, 1)?, name(l, a, 2)?);
                let offset = [opt(3)?, opt(4)?, opt(5)?];
                d(Effect::AiMoveToLocator { entity, locator, offset, yaw_deg: opt(6)? })
            }
            "DO_AudioPostEvent" => {
                d(Effect::AudioPostEvent { entity: name(l, a, 1)?, event: name(l, a, 2)?, parameter: name(l, a, 3)? })
            }
            "DO_AudioSetGameVar" => d(Effect::AudioSetGameVar { var: l.check_int(a, 1)?, value: l.check_int(a, 2)? }),
            "DO_AudioSetPresentationType" => d(Effect::AudioSetPresentationType(l.check_int(a, 1)?)),
            "DO_AudioPathEvent" => d(Effect::AudioPathEvent { command: l.check_int(a, 1)?, value: l.check_int(a, 2)? }),
            "DO_AwardBadge" => d(Effect::AwardBadge { badge: name(l, a, 1)?, category: l.check_int(a, 2)? }),
            "DO_AwardTrackBonus" => d(Effect::AwardTrackBonus { bonus: name(l, a, 1)?, flag: to_bool(a, 2) }),
            "DO_AwardAchievement" => d(Effect::AwardAchievement(name(l, a, 1)?)),
            "DO_SetStatsWriteable" => d(Effect::SetStatsWriteable(to_bool(a, 1))),
            "DO_PlayNis" => d(Effect::PlayNis(nis_request(l, a)?)),
            "DO_FadeIn" | "DO_FadeOut" => {
                d(Effect::Fade { fade_out: b.name == "DO_FadeOut", seconds: num(l, a, 1)?, delay: num(l, a, 2)? })
            }
            "DO_EndLevel" => d(Effect::EndLevel(to_bool(a, 1))),
            "DO_AddGlobalLightingParamOverrideFromParam" => {
                d(Effect::LightingOverrideFromParam { target: name(l, a, 1)?, source: name(l, a, 2)? })
            }
            "DO_RemoveGlobalLightingParamOverride" => d(Effect::LightingOverrideRemove(name(l, a, 1)?)),
            "DO_FlushGlobalLighting" => d(Effect::LightingFlush(l.opt_num(a, 1, 0.0)? as f32)),
            "DO_OverrideCascadeShadowParams" => {
                let p = [num(l, a, 1)?, num(l, a, 2)?, num(l, a, 3)?, num(l, a, 4)?, num(l, a, 5)?];
                let _ = l.opt_num(a, 6, 0.0)?;
                d(Effect::CascadeShadowOverride(p))
            }
            "DO_RemoveCascadeShadowParamsOverride" => {
                let _ = l.opt_num(a, 1, 0.0)?;
                d(Effect::CascadeShadowRemove)
            }
            "DO_MoveToNisStartLocation" => d(Effect::MoveToNisStartLocation {
                nis: text(l, a, 1)?,
                role: name(l, a, 2)?,
                entity: name(l, a, 3)?,
                locator: name(l, a, 4)?,
            }),
            "DO_PostEffectCue" => d(Effect::PostEffectCue { name: name(l, a, 1)?, cue: l.check_int(a, 2)? }),
            "DO_PostEvent" => d(Effect::PostEvent { name: name(l, a, 1)?, entity: opt_name(l, a, 2)? }),
            "DO_PostFXStart" => d(Effect::PostFxStart {
                name: name(l, a, 1)?,
                start: l.opt_num(a, 2, 0.0)? as f32,
                end: l.opt_num(a, 3, 1.0)? as f32,
            }),
            "DO_RestartObjective" => Node::Restart,
            "DO_SetGameStateValue" => match set_game_state(l, a)? {
                Some(e) => d(e),
                None => return Ok(None),
            },
            "DO_SetNisAudioType" => d(Effect::SetNisAudioType(name(l, a, 1)?)),
            "DO_SetPermanentNamedFlag" | "DO_SetTemporaryNamedFlag" => Node::Do(Do::NamedFlag {
                name: name(l, a, 1)?,
                value: to_bool(a, 2),
                temporary: b.name == "DO_SetTemporaryNamedFlag",
            }),
            "DO_SurviveUntilYouDieFog" => Node::Do(Do::SurviveFog {
                source: name(l, a, 1)?,
                target: name(l, a, 2)?,
                blend: to_num(a, 3),
                fraction: to_num(a, 4),
                target_on: false,
                begun: false,
            }),
            "DO_BoostNotification" => d(Effect::BoostNotification),
            "DO_DisableSplitsCallout" => d(Effect::DisableSplitsCallout(true)),
            "DO_NisPrompt" => d(Effect::NisPrompt),
            "DO_NisTitle" => d(Effect::NisTitle),
            "DO_TutorialPopup" => d(Effect::TutorialPopup(text(l, a, 1)?)),
            "DO_TutorialMessage" => {
                d(Effect::TutorialMessage { title: text(l, a, 1)?, body: text(l, a, 2)?, seconds: to_num(a, 3) })
            }
            "DO_TutorialTrickObjectiveReset" => {
                d(Effect::TutorialTrickReset { entity: name(l, a, 1)?, help: text(l, a, 2)? })
            }
            "DO_TutorialTrickObjectiveAdd" => {
                let (state, cmp, value) = (l.check_int(a, 1)?, l.check_int(a, 2)?, l.check_int(a, 3)?);
                d(Effect::TutorialTrickAdd {
                    state,
                    cmp,
                    value,
                    goofy: if state == 0xF3 { mirrored_grab(value) } else { value },
                    scale: to_num(a, 4),
                    count: to_integer(a, 5),
                    text: text(l, a, 6)?,
                })
            }
            "DO_TutorialTrickObjectiveShow" => {
                d(Effect::TutorialTrickShow { button: l.check_int(a, 1)?, text: text(l, a, 2)? })
            }
            "DO_Visibility" => d(Effect::Visibility { set: name(l, a, 1)?, state: text(l, a, 2)? }),
            "DO_WarpEntity" => d(Effect::WarpEntity { entity: name(l, a, 1)?, locator: name(l, a, 2)? }),
            "DO_WriteLocatorToGameState" => d(Effect::WriteLocatorToGameState {
                entity: name(l, a, 1)?,
                locator: name(l, a, 2)?,
                position_id: l.check_int(a, 3)?,
                rotation_id: l.check_int(a, 4)?,
                offset: [to_num(a, 5), to_num(a, 6), to_num(a, 7)],
            }),
            "IMMEDIATE_LoadPostFXOverlayTexture" => {
                let names = a.iter().filter_map(|v| v.to_str_coerce()).map(|s| s.to_str_lossy()).collect();
                let c = self.current(l)?;
                let objective = self.objective(c).name.clone();
                self.world.effect(&objective, &Effect::LoadPostFxOverlayTextures(names));
                Node::Nop
            }
            _ => return self.call_wait(l, b, a),
        };
        self.append(l, b, node)?;
        Ok(None)
    }
}

impl<W: World> Builder<W> {
    fn call_wait(
        &mut self,
        l: &mut Lua,
        b: &'static Binding,
        a: &[Value],
    ) -> LuaResult<Option<Box<dyn std::any::Any>>> {
        let w = match b.name {
            "WAIT_IsBadgeSet" | "WAIT_IsTrackBonusSet" => {
                let kind = if b.name == "WAIT_IsBadgeSet" { 2 } else { 0 };
                let w = Wait::BadgeSet { badge: name(l, a, 1)?, kind, value: to_bool(a, 2) };
                if kind == 2 {
                    self.append_stat(l, b, w)?;
                    return Ok(None);
                }
                w
            }
            "WAIT_CheckNamedFlag" => Wait::NamedFlag(name(l, a, 1)?),
            "WAIT_CheckEventType" => Wait::EventType(l.check_int(a, 1)?),
            "WAIT_IsStat" | "WAIT_IsLocationUniqueStat" => {
                let (location, k) = if b.name == "WAIT_IsStat" { (None, 1) } else { (Some(name(l, a, 2)?), 2) };
                let key = StatKey { name: name(l, a, 1)?, scope: l.check_int(a, k + 1)?, location };
                let w = StatWait { key, cmp: cmp(l, a, k + 2)?, value: l.check_num(a, k + 3)?, kind: 2, progress: 0.0 };
                self.append_stat(l, b, Wait::Stat(w))?;
                return Ok(None);
            }
            "StatsBegin" => {
                let c = self.current(l)?;
                let badge = name(l, a, 1)?;
                let block = StatsBlock { badge, waits: Vec::new(), closed: false, swallowed: 0, reported: 0.0 };
                self.append(l, b, Node::Stats(block))?;
                let k = self.objective(c).nodes.len() - 1;
                if let Some(Node::Stats(_)) = self.objective(c).nodes.get(k) {
                    self.open_block = Some((c, k));
                } else {
                    let what = format!("StatsBegin swallowed in {}", self.objective(c).name);
                    self.report.problems.push(what);
                }
                return Ok(None);
            }
            "StatsEnd" => {
                self.append(l, b, Node::Nop)?;
                if let Some((c, k)) = self.open_block.take()
                    && let Some(Node::Stats(s)) = self.objective(c).nodes.get_mut(k)
                {
                    s.closed = true;
                }
                return Ok(None);
            }
            "WAIT_GameStateValue" => match game_state_wait(l, a)? {
                Some(w) => w,
                None => return Ok(None),
            },
            "WAIT_GlobalLightingIsNightCondition" => Wait::Night,
            "WAIT_ObjectiveComplete" => Wait::ObjectiveComplete(name(l, a, 1)?),
            "WAIT_PostedEvent" => Wait::PostedEvent { event: name(l, a, 1)?, entity: opt_name(l, a, 2)? },
            "WAIT_RemainingDistance" => Wait::RemainingDistance { entity: name(l, a, 1)?, distance: num(l, a, 2)? },
            "WAIT_Time" => Wait::Time { duration: num(l, a, 1)?, elapsed: 0.0 },
            "WAIT_TriggerVolumeByGroup" => {
                let g = l.check_num(a, 1)? as i64 as u16;
                Wait::TriggerByGroup((1u64.checked_shl(u32::from(g & 0x3F)).unwrap_or(0) & 0xFFFF) as u16)
            }
            "WAIT_TriggerVolumeByName" => Wait::TriggerByName(name(l, a, 1)?),
            "WAIT_ObjectiveMessage" => Wait::ObjectiveMessage(ObjectiveMessageWait {
                entity: name(l, a, 1)?,
                message: None,
                shown: false,
                time: 0.0,
            }),
            "WAIT_IsEquipped" => {
                Wait::Equipped { item: l.check_int(a, 1)?, equipment_type: l.check_int(a, 2)?, held: false }
            }
            "WAIT_WorldEntityValue" => {
                let x = float_binding(l, a, 1)?;
                let c = cmp(l, a, 2)?;
                Wait::WorldEntityValue { a: x, cmp: c, b: float_binding(l, a, 3)? }
            }
            "WAIT_TutorialMessageReady" => Wait::TutorialMessageReady,
            "WAIT_TutorialTrickObjective" => Wait::TutorialTricks { entity: name(l, a, 1)? },
            "FloatGameStateValue" => {
                let entity = name(l, a, 1)?;
                let id = u16::try_from(l.check_int(a, 2)?).unwrap_or(0);
                return Ok(Some(Box::new(FloatBinding::GameState { entity, id })));
            }
            "FloatExpressionValue" => {
                let x = float_binding(l, a, 1)?;
                let op = l.check_int(a, 2)?;
                let y = float_binding(l, a, 3)?;
                let op = u8::try_from(op).ok().filter(|o| *o < 4).ok_or_else(|| l.rt_error("operator out of 0…3"))?;
                return Ok(Some(Box::new(FloatBinding::Expr(Box::new(x), op, Box::new(y)))));
            }
            other => {
                *self.report.not_ported.entry(other).or_default() += 1;
                return Ok(None);
            }
        };
        self.append(l, b, Node::Wait(w))?;
        Ok(None)
    }

    fn begin(&mut self, l: &Lua, a: &[Value], over: bool) -> LuaResult<Option<Box<dyn std::any::Any>>> {
        let n = name(l, a, 1)?;
        let found = if over { super::find(&self.runner.sets, n.hash) } else { None };
        match found {
            Some((s, i)) => {
                self.runner.clear(s, i, &mut self.world);
                let o = &mut self.runner.sets[s][i];
                o.overridden = true;
                o.script = self.script.clone();
                self.current = Some(Cur::Set(s, i));
                self.report.overrides += 1;
            }
            None => {
                self.pending.push(Objective::new(n.clone(), &self.script));
                self.current = Some(Cur::Pending(self.pending.len() - 1));
            }
        }
        Ok(Some(Box::new(n)))
    }

    fn end(&mut self, l: &Lua) -> LuaResult<()> {
        let c = self.current(l)?;
        let (s, i) = match c {
            Cur::Set(s, i) => (s, i),
            Cur::Pending(k) => {
                let o = self.pending.remove(k);
                let s = self.set as usize;
                self.runner.sets[s].push(o);
                self.report.objectives += 1;
                (s, self.runner.sets[s].len() - 1)
            }
        };
        let o = &self.runner.sets[s][i];
        if o.nodes.is_empty() {
            self.report
                .problems
                .push(format!("empty objective {} ({}): the game reads its first node from NULL", o.name, o.script));
        }
        if let Some(n) = o.nodes.iter().find(|n| !n.complete()) {
            self.report.problems.push(format!("objective {} ends with an incomplete node {n:?}", o.name));
        }
        self.runner.start(s, i, &mut self.world);
        if let Some((oc, k)) = self.open_block {
            self.open_block = match (oc, c) {
                _ if oc == c => Some((Cur::Set(s, i), k)),
                (Cur::Pending(b), Cur::Pending(r)) if b > r => Some((Cur::Pending(b - 1), k)),
                _ => Some((oc, k)),
            };
        }
        self.current = None;
        Ok(())
    }
}

pub fn mirrored_grab(grab: i32) -> i32 {
    // TODO(guess): entry 0 (`Grab_None`) is not written by the constructor (an uninitialised stack word; no script adds grab 0); settle: the stack slot's earlier contents at 0x824DD968.
    const MIRROR: [i32; 14] = [0, 1, 10, 4, 3, 9, 13, 7, 11, 5, 2, 8, 12, 6];
    usize::try_from(grab).ok().and_then(|i| MIRROR.get(i)).copied().unwrap_or(grab)
}

fn float_binding(l: &Lua, a: &[Value], i: usize) -> LuaResult<FloatBinding> {
    let v = a.get(i - 1).cloned().unwrap_or(Value::Nil);
    if let Some(n) = v.to_num() {
        return Ok(FloatBinding::Const(n as f32));
    }
    if let Value::UserData(r) = v
        && let Some(f) = crate::heap::downcast::<FloatBinding>(l.heap.userdata(r))
    {
        return Ok(f.clone());
    }
    Err(l.rt_error(&format!("argument #{i} is no float value (the game reads a NULL binding)")))
}

fn typed_value(a: &[Value], id: i32, at: usize) -> Option<GsValue> {
    Some(match game_state_type(id)? {
        GsType::Bool => GsValue::Bool(to_bool(a, at)),
        GsType::Float => GsValue::Float(to_num(a, at)),
        GsType::Int => GsValue::Int(to_integer(a, at)),
        GsType::Vector => GsValue::Vector([to_num(a, at), to_num(a, at + 1), to_num(a, at + 2)]),
        GsType::Quat => return None,
    })
}

fn set_game_state(l: &Lua, a: &[Value]) -> LuaResult<Option<Effect>> {
    let entity = name(l, a, 1)?;
    let id = l.check_int(a, 2)?;
    Ok(typed_value(a, id, 3).map(|value| Effect::SetGameState { entity, id: id as u16, value }))
}

fn game_state_wait(l: &Lua, a: &[Value]) -> LuaResult<Option<Wait>> {
    let entity = name(l, a, 1)?;
    let id = l.check_int(a, 2)?;
    let c = cmp(l, a, 3)?;
    Ok(typed_value(a, id, 4).map(|value| Wait::GameState { entity, id: id as u16, cmp: c, value }))
}

fn nis_request(l: &mut Lua, a: &[Value]) -> LuaResult<NisRequest> {
    let t = l.check_table(a, 1)?;
    let field = |l: &Lua, k: &str| l.heap.table(t).get_str(k);
    let err = |l: &Lua, m: &str| -> LuaError { l.rt_error(m) };
    let for_debug = field(l, "forDebug").truthy();
    let name = match field(l, "name").to_str_coerce() {
        Some(s) => s.to_str_lossy(),
        None => return Err(l.arg_error(2, "string expected, got no string")),
    };
    let origin = match field(l, "sceneOrigin").to_num() {
        Some(n) => to_int(n),
        None => return Err(l.arg_error(2, "number expected")),
    };
    if !(0..=3).contains(&origin) {
        return Err(err(l, "Invalid scene origin type."));
    }
    let (mut location, mut locator) = (None, None);
    if origin == 0 {
        let Value::Table(loc) = field(l, "location") else { return Err(l.arg_error(2, "table expected")) };
        let mut v = [0.0f32; 4];
        for (k, x) in v.iter_mut().enumerate() {
            *x = l.heap.table(loc).get_int(k as i64 + 1).to_num().ok_or_else(|| l.arg_error(3, "number expected"))?
                as f32;
        }
        location = Some(v);
    } else if origin >= 2 {
        let s = field(l, "locator").to_str_coerce().ok_or_else(|| l.arg_error(2, "string expected"))?;
        locator = Some(Name { hash: fnv1(s.bytes()), text: Arc::from(s.to_str_lossy().as_str()) });
    }
    let locator_alternate = field(l, "locatorAlternate")
        .to_str_coerce()
        .map(|s| Name { hash: fnv1(s.bytes()), text: Arc::from(s.to_str_lossy().as_str()) });
    let Value::Table(actors_t) = field(l, "actors") else { return Err(l.arg_error(2, "table expected")) };
    let mut actors = Vec::new();
    let mut key = Value::Nil;
    while let Ok(Some((k, v))) = l.heap.table(actors_t).next(&key) {
        key = k;
        let Value::Table(e) = v else { return Err(l.arg_error(4, "table expected")) };
        let get = |k: &str| l.heap.table(e).get_str(k);
        let hashed = |v: Value| {
            v.to_str_coerce().map(|s| Name { hash: fnv1(s.bytes()), text: Arc::from(s.to_str_lossy().as_str()) })
        };
        let role = hashed(get("role")).ok_or_else(|| l.arg_error(5, "string expected"))?;
        let (entity, set_index) = match hashed(get("entity")) {
            Some(n) => (n, None),
            None => match hashed(get("entitySet")) {
                Some(n) => (n, Some(get("index").to_num().map_or(0, to_int))),
                None => return Err(err(l, "Malformed actor binding. Expected 'entity' or 'entitySet'.")),
            },
        };
        actors.push(NisActorBinding { role, entity, set_index });
    }
    Ok(NisRequest { name, for_debug, origin, location, locator, locator_alternate, actors })
}
