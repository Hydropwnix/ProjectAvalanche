use anyhow::{Context, Result, bail, ensure};
use glam::Quat;
use ssx_formats::ant::Value;

use crate::tags::TagWindow;
use crate::tuning::{AssetView, Bundle};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlendCurve {
    pub time: f32,
    pub curve: i64,
}

impl BlendCurve {
    pub fn default_transition() -> Self {
        let b = ssx_assets::anim_graph::Blend::default_transition();
        Self { time: b.blend_time, curve: b.curve }
    }

    fn of(a: &AssetView<'_>) -> Result<Self> {
        ensure!(a.class() == "BlendCurveAsset", "{:?} is a {}, expected a BlendCurveAsset", a.name(), a.class());
        let (time, curve) = (a.f32("BlendTime")?, a.int("CurveType")?);
        ensure!(time.is_finite() && time >= 0.0, "{:?}: BlendTime {time}", a.name());
        ensure!((0..=3).contains(&curve), "{:?}: CurveType {curve} (0…3)", a.name());
        Ok(Self { time, curve })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlowTransition {
    pub blend: BlendCurve,
    pub uber_trigger: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HandleTag {
    pub window: TagWindow,
    pub value: f32,
    pub mode: u32,
    pub reset: Option<f32>,
    pub spline: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct RetractRotation {
    pub clip_name: String,
    pub clip: crate::rootmotion::RootClip,
    pub start_tick: f32,
    pub length: f32,
    pub start_time: f32,
    pub begin: f32,
    pub scale: f32,
    pub mirror: bool,
}

impl RetractRotation {
    fn actor_time(&self, node_ticks: f32) -> f32 {
        (self.start_tick + node_ticks).clamp(0.0, self.length.max(0.0))
    }

    pub fn rotation(&self, n0: f32, dt: f32) -> Option<Quat> {
        let (a0, a1) = (self.actor_time(n0), self.actor_time(n0 + dt));
        if a1 < self.begin - 1e-4 {
            return None;
        }
        let clip_time = |a: f32| ((a - self.start_time) * self.scale).clamp(0.0, self.clip.num_ticks());
        let (_, r) = self.clip.joint_delta(clip_time(a0.max(self.begin)), clip_time(a1)).ok()?;
        Some(if self.mirror { Quat::from_xyzw(-r.x, -r.y, r.z, r.w) } else { r })
    }

    fn load<'a>(exit: &AssetView<'a>) -> Result<Option<Self>> {
        let subject = exit.asset("SubjectController")?;
        ensure!(subject.class() == "ModularControllerAsset", "{}'s subject is a {}", exit.name(), subject.class());
        let mut items = Vec::new();
        for item in subject.asset_list("Items")? {
            if item.int("Enabled")? != 0 {
                items.push(item.asset("ControllerAsset")?);
            }
        }
        let (ballistic, seq) = match items.as_slice() {
            [b] => (*b, None),
            [b, s] => (*b, Some(*s)),
            _ => {
                bail!("{}: {} enabled modular items, expected the ballistic and its subject", exit.name(), items.len())
            }
        };
        ensure!(ballistic.class() == "BallisticControllerAsset", "{}'s root is a {}", exit.name(), ballistic.class());
        let Some(seq) = seq else { return Ok(None) };
        ensure!(seq.class() == "SequenceContainerAsset", "{}'s ballistic subject is a {}", exit.name(), seq.class());
        let Some(actor) = seq.asset_list("ActorAssets")?.first().copied() else { return Ok(None) };
        let schema = actor.bundle.schema();
        let field = |v: &'a Value, n: &str| ssx_assets::anim_db::field(schema, v, n);
        let num = |v: &'a Value, n: &str| field(v, n).and_then(|x| x.as_f32().or_else(|| x.as_i64().map(|i| i as f32)));
        let Value::Array(tracks) = actor.value("Tracks")? else { bail!("{}: Tracks", actor.name()) };
        let target = |t: &'a Value| {
            let g = field(t, "Target")?.as_guid()?;
            actor.bundle.object_by_guid(g)?.name(schema)
        };
        let Some(track) = tracks.iter().find(|t| target(t) == Some("AnimRig")).or(tracks.first()) else {
            return Ok(None);
        };
        let anims = match field(track, "Anims") {
            Some(Value::Array(a)) => a.as_slice(),
            _ => &[],
        };
        let element = match anims {
            [] => return Ok(None),
            [e] => e,
            _ => bail!("{}: {} elements; the port's retract plays one", actor.name(), anims.len()),
        };
        let guid = field(element, "Asset").and_then(|g| g.as_guid()).filter(|g| !g.is_null());
        let Some(clip) = guid.and_then(|g| actor.bundle.object_by_guid(g)) else { return Ok(None) };
        let clip = AssetView { bundle: actor.bundle, object: clip };
        let Some(root) = crate::rootmotion::RootClip::of_clip_controller(&clip)? else { return Ok(None) };
        let length = actor.f32("Length")?;
        let start_tick = actor.value("DefaultInitParam").ok().and_then(|v| num(v, "StartTick")).unwrap_or(0.0);
        let (start_time, start_in) =
            (num(element, "StartTime").unwrap_or(0.0), num(element, "StartInTime").unwrap_or(0.0));
        let scale = num(element, "Scale").filter(|&x| x != 0.0).unwrap_or(1.0);
        let flags = field(element, "Flags").and_then(|x| x.as_i64()).unwrap_or(0);
        let all = [length, start_tick, start_time, start_in, scale];
        ensure!(all.iter().all(|x| x.is_finite() && x.abs() <= 1.0e5), "{}: timing {all:?}", actor.name());
        Ok(Some(Self {
            clip_name: clip.name().to_string(),
            clip: root,
            start_tick,
            length,
            start_time,
            begin: start_time + start_in,
            scale,
            mirror: flags & 1 != 0,
        }))
    }
}

#[derive(Clone, Debug)]
pub struct WingsuitFlow {
    pub deploy_end: f32,
    pub retract_end: f32,
    pub airborne_to_enter: FlowTransition,
    pub jump_to_enter: FlowTransition,
    pub airborne_to_fly: FlowTransition,
    pub enter_to_fly: FlowTransition,
    pub fly_to_exit: FlowTransition,
    pub fly_to_exit_board_particle: FlowTransition,
    pub exit_to_airborne: FlowTransition,
    pub handle_deploy: HandleTag,
    pub handle_retract: HandleTag,
    pub retract_rotation: Option<RetractRotation>,
}

const TRIGGER: &str = "BLN_trigger_uber_crash";
const BOARD_PARTICLE: &str = "BLN_EnableBoardParticle";
const HANDLE_WEIGHT: &str = "FLT_Wingsuit_BlendWeight";

fn names(a: &AssetView<'_>, field: &str) -> Result<Vec<String>> {
    Ok(a.asset_list(field)?.iter().map(|c| c.name().to_string()).collect())
}

fn transition<'a>(from: &AssetView<'a>, to: &str, particle: Option<bool>) -> Result<AssetView<'a>> {
    let mut found = Vec::new();
    for t in from.asset_list("Transitions")? {
        if t.asset("To")?.name() != to {
            continue;
        }
        let wanted = match particle {
            None => true,
            Some(p) => names(&t, if p { "ConditionsRequiredTrue" } else { "ConditionsRequiredFalse" })?
                .iter()
                .any(|n| n == BOARD_PARTICLE),
        };
        if wanted {
            found.push(t);
        }
    }
    match found.as_slice() {
        [t] => Ok(*t),
        _ => bail!("{} → {to} (board particle {particle:?}): {} transitions, expected one", from.name(), found.len()),
    }
}

fn window<'a>(node: &AssetView<'a>, kind: &str) -> Result<AssetView<'a>> {
    let subject = node.asset("SubjectController")?;
    ensure!(subject.class() == "ModularControllerAsset", "{}'s subject is a {}", node.name(), subject.class());
    let mut found = Vec::new();
    for item in subject.asset_list("Items")? {
        let c = item.asset("ControllerAsset")?;
        if c.class() != "SequenceContainerAsset" {
            continue;
        }
        for actor in c.asset_list("ActorAssets")? {
            for w in actor.asset_list("BranchTable")? {
                if w.asset_opt("BranchTypeAsset")?.is_some_and(|b| b.name() == kind) {
                    found.push(w);
                }
            }
        }
    }
    let [w] = found.as_slice() else { bail!("{}: {} {kind} windows, expected one", node.name(), found.len()) };
    for f in ["StartTuningOffset", "EndTuningOffset"] {
        ensure!(w.f32(f)? == 0.0, "{}'s {kind} window: {f} {}", node.name(), w.f32(f)?);
    }
    let time = w.f32("Time")?;
    ensure!(time.is_finite() && time >= 0.0, "{}'s {kind} window: Time {time}", node.name());
    Ok(*w)
}

fn tags<'a>(owner: &AssetView<'a>) -> Result<Vec<AssetView<'a>>> {
    let Some(set) = owner.asset_opt("TagCollectionSet")? else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for collection in set.asset_list("TagCollectionKeys")? {
        out.extend(collection.asset_list("TagKeys")?);
    }
    Ok(out)
}

fn flow_transition(from: &AssetView<'_>, t: &AssetView<'_>) -> Result<(FlowTransition, Option<f32>)> {
    let name = t.name();
    let window = match t.asset_opt("BranchWindowTypeAsset")? {
        Some(kind) => Some(window(from, kind.name())?),
        None => None,
    };
    let blend = match (t.asset_opt("OverrideBlend")?, window.map(|w| w.asset_opt("BlendAsset")).transpose()?.flatten())
    {
        (Some(b), _) | (None, Some(b)) => BlendCurve::of(&b).with_context(|| format!("{name}'s blend"))?,
        (None, None) => BlendCurve::default_transition(),
    };
    let mut uber_trigger = None;
    for tag in tags(t)? {
        let ok = tag.class() == "BoolGameStateTag" && tag.asset("ValueAsset")?.name() == TRIGGER;
        ensure!(ok, "{name}: a {} tag {:?} (the port writes only {TRIGGER})", tag.class(), tag.name());
        let (time, duration, mode) = (tag.int("Time")?, tag.int("Duration")?, tag.int("Mode")?);
        ensure!(
            (time, duration, mode) == (0, 0, 1),
            "{name}: {TRIGGER} tag Time {time} Duration {duration} Mode {mode}"
        );
        ensure!(uber_trigger.is_none(), "{name}: two {TRIGGER} tags");
        uber_trigger = Some(tag.int("Value")? != 0);
    }
    Ok((FlowTransition { blend, uber_trigger }, window.map(|w| w.f32("Time")).transpose()?))
}

fn handle_tag(node: &AssetView<'_>, bundle: &Bundle) -> Result<HandleTag> {
    let subject = node.asset("SubjectController")?;
    let mut found = Vec::new();
    for item in subject.asset_list("Items")? {
        let c = item.asset("ControllerAsset")?;
        if c.class() != "SequenceContainerAsset" {
            continue;
        }
        for actor in c.asset_list("ActorAssets")? {
            for tag in tags(&actor)? {
                if tag.class() == "FloatGameStateTag" && tag.asset("ValueAsset")?.name() == HANDLE_WEIGHT {
                    found.push(tag);
                }
            }
        }
    }
    let [tag] = found.as_slice() else { bail!("{}: {} {HANDLE_WEIGHT} tags, expected one", node.name(), found.len()) };
    let (time, duration) = (tag.int("Time")?, tag.int("Duration")?);
    ensure!((0..=32767).contains(&time) && (0..=32767).contains(&duration), "{}: the handle tag's window", node.name());
    let mode = u32::try_from(tag.int("Mode")?).with_context(|| format!("{}: the handle tag's Mode", node.name()))?;
    let value = tag.f32("Value")?;
    let Value::Array(items) = tag.value("Spline")? else { bail!("{}: Spline", node.name()) };
    let spline = items.iter().map(|v| v.as_f32().filter(|x| x.is_finite())).collect::<Option<Vec<f32>>>();
    let spline = spline.with_context(|| format!("{}: the handle tag's Spline", node.name()))?;
    ensure!(value.is_finite() && spline.len() <= 64, "{}: the handle tag's value or spline", node.name());
    let reset = if tag.int("ResetOnLeave")? != 0 {
        Some(bundle.view(HANDLE_WEIGHT, "FloatAsset")?.f32("Default")?)
    } else {
        None
    };
    let always_active = tag.int("AlwaysActive")? != 0;
    Ok(HandleTag { window: TagWindow::new(time as f32, duration as f32, always_active), value, mode, reset, spline })
}

impl WingsuitFlow {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let sf = bundle.view("SF_Ride", "StateFlowControllerAsset")?;
        let nodes = sf.asset_list("StateFlowNodeAssets")?;
        let node = |name: &str| {
            nodes.iter().copied().find(|n| n.name() == name).with_context(|| format!("SF_Ride has no node {name}"))
        };
        let (airborne, jump) = (node("state_airborne")?, node("state_airborne_jump")?);
        let (enter, fly, exit) =
            (node("state_wingsuit_enter")?, node("state_wingsuit_fly")?, node("state_wingsuit_exit")?);
        let take = |from: &AssetView<'_>, to: &str, particle: Option<bool>| {
            flow_transition(from, &transition(from, to, particle)?)
        };
        let (airborne_to_enter, _) = take(&airborne, "state_wingsuit_enter", None)?;
        let (jump_to_enter, _) = take(&jump, "state_wingsuit_enter", None)?;
        let (airborne_to_fly, _) = take(&airborne, "state_wingsuit_fly", None)?;
        let (enter_to_fly, deploy_end) = take(&enter, "state_wingsuit_fly", None)?;
        let (fly_to_exit, _) = take(&fly, "state_wingsuit_exit", Some(false))?;
        let (fly_to_exit_board_particle, _) = take(&fly, "state_wingsuit_exit", Some(true))?;
        let (exit_to_airborne, retract_end) = take(&exit, "state_airborne", None)?;
        let deploy_end = deploy_end.context("state_wingsuit_enter → state_wingsuit_fly has no window")?;
        let retract_end = retract_end.context("state_wingsuit_exit → state_airborne has no window")?;
        Ok(Self {
            deploy_end,
            retract_end,
            airborne_to_enter,
            jump_to_enter,
            airborne_to_fly,
            enter_to_fly,
            fly_to_exit,
            fly_to_exit_board_particle,
            exit_to_airborne,
            handle_deploy: handle_tag(&enter, bundle)?,
            handle_retract: handle_tag(&exit, bundle)?,
            retract_rotation: RetractRotation::load(&exit)?,
        })
    }
}
