use bevy::prelude::*;
use ssx_physics::graph::LandingRumble;
use ssx_physics::surface::{ICE, POWDER, ROCK, SNOW_COVERAGE};

use crate::particles::spray::SprayInputs;
use crate::ride::Ride;

#[derive(Default)]
pub struct Trackers {
    rumble: LandingRumble,
    landing_impact: f32,
}

impl Trackers {
    pub fn step(&mut self, ride: &Ride) {
        self.landing_impact = self.rumble.step(&ride.state);
    }

    pub fn inputs(&self, ride: &Ride) -> SprayInputs {
        let s = &ride.state;
        let w = &s.weights;
        SprayInputs {
            is_awesome: s.score.is_awesome,
            velocity: s.traj_velocity,
            spray_velocity: s.spray.velocity,
            ice_spray: s.spray.ice,
            snow_spray: s.spray.carving,
            landing_impact: self.landing_impact,
            ground_contact: s.contact.is_some(),
            in_air: s.in_air(),
            depth_below_snow: s.depth_below_snow,
            grinding: s.grind.is_grinding(),
            powder: w[POWDER],
            packed: [w[0], w[1], w[2], w[3], w[4]],
            ice: w[ICE],
            rock: w[ROCK],
            backwards_rewind: ride.rewind_backwards,
            ground_normal: s.ground_normal,
            snow_coverage: w[SNOW_COVERAGE],
            ao: ride.session.controllers.as_ref().map_or(1.0, |c| c.fx.world_ao),
            is_super_awesome: s.score.is_super_awesome,
            track_started: s.track_started,
            wipeout: s.landing.tumbling,
            ice_axe_active: s.ice_axe.frame.active,
            ice_axe_blend: s.ice_axe.blend_weight,
            lean: s.kin.lean_norm,
            downwash_visual_strength: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Fx {
    Float(f32),
    UInt(u32),
    Int(i32),
    Bool(bool),
    Var(VarValue),
}

#[derive(Clone, Copy, Debug)]
enum VarValue {
    Bool(bool),
    Int(i32),
    Float(f32),
}

impl Fx {
    fn float(self) -> f32 {
        match self {
            Fx::Float(x) => x,
            Fx::UInt(x) => x as f32,
            Fx::Int(x) => x as f32,
            Fx::Bool(b) | Fx::Var(VarValue::Bool(b)) => b as u8 as f32,
            Fx::Var(VarValue::Int(x)) => x as f32,
            Fx::Var(VarValue::Float(x)) => x,
        }
    }
    fn int(self) -> i64 {
        match self {
            Fx::Float(x) | Fx::Var(VarValue::Float(x)) => x as i64,
            Fx::UInt(x) => x as i64,
            Fx::Int(x) | Fx::Var(VarValue::Int(x)) => x as i64,
            Fx::Bool(b) | Fx::Var(VarValue::Bool(b)) => b as i64,
        }
    }
}

const FX_OPS: [(&str, u8); 12] = [
    ("&&", 4),
    ("||", 4),
    ("==", 3),
    ("!=", 3),
    ("<=", 2),
    (">=", 2),
    ("<", 2),
    (">", 2),
    ("+", 1),
    ("-", 1),
    ("*", 0),
    ("/", 0),
];

fn fx_eval(text: &str, var: &dyn Fn(&str) -> Option<VarValue>) -> Fx {
    let s: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    fx_parse(&s, var)
}

fn fx_parse(s: &str, var: &dyn Fn(&str) -> Option<VarValue>) -> Fx {
    let b = s.as_bytes();
    let (mut depth, mut skip, mut best): (i32, bool, Option<(u8, usize, &str)>) = (0, true, None);
    for i in 0..b.len() {
        match b[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        if skip {
            skip = false;
            continue;
        }
        if depth != 0 {
            continue;
        }
        if let Some(&(op, prio)) = FX_OPS.iter().find(|(op, _)| s[i..].starts_with(op)) {
            if best.is_none_or(|(p, _, _)| prio > p) {
                best = Some((prio, i, op));
            }
            skip = true;
        }
    }
    if let Some((_, i, op)) = best {
        let (l, r) = (fx_parse(&s[..i], var), fx_parse(&s[i + op.len()..], var));
        return fx_binary(op, l, r);
    }
    if let Some(rest) = s.strip_prefix('!') {
        return Fx::Int((fx_parse(rest, var).int() == 0) as i32);
    }
    if let Some(rest) = s.strip_prefix('-') {
        return match fx_parse(rest, var) {
            Fx::Int(x) => Fx::Int(-x),
            Fx::Float(x) => Fx::Float(-x),
            other => other,
        };
    }
    if let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return fx_parse(inner, var);
    }
    match s {
        "true" => Fx::Bool(true),
        "false" => Fx::Bool(false),
        _ if s.contains('.') => Fx::Float(s.parse().unwrap_or(0.0)),
        _ => match var(s) {
            Some(v) => Fx::Var(v),
            None => match s.strip_prefix("0x") {
                Some(h) => Fx::UInt(u32::from_str_radix(h, 16).unwrap_or(0)),
                None => Fx::Int(s.parse().unwrap_or(0)),
            },
        },
    }
}

fn fx_binary(op: &str, l: Fx, r: Fx) -> Fx {
    match op {
        "&&" => return Fx::Bool(l.int() != 0 && r.int() != 0),
        "||" => return Fx::Bool(l.int() != 0 || r.int() != 0),
        _ => {}
    }
    enum Mode {
        Float,
        UInt,
        Int,
        Zero,
    }
    let is_var = |x: &Fx| matches!(x, Fx::Var(_));
    let int_like = |x: &Fx| matches!(x, Fx::Int(_) | Fx::UInt(_) | Fx::Var(_));
    let mode = if matches!(l, Fx::Float(_)) || matches!(r, Fx::Float(_)) {
        Mode::Float
    } else if matches!((l, r), (Fx::UInt(_), Fx::UInt(_))) {
        Mode::UInt
    } else if (matches!(l, Fx::Int(_)) && int_like(&r)) || (matches!(r, Fx::Int(_)) && int_like(&l)) {
        Mode::Int
    } else if is_var(&l) || is_var(&r) {
        Mode::Float
    } else {
        Mode::Zero
    };
    let cmp = |o: std::cmp::Ordering| match op {
        "==" => o.is_eq(),
        "!=" => o.is_ne(),
        "<" => o.is_lt(),
        ">" => o.is_gt(),
        "<=" => o.is_le(),
        _ => o.is_ge(),
    };
    let arith = matches!(op, "+" | "-" | "*" | "/");
    match mode {
        Mode::Float => {
            let (a, b) = (l.float(), r.float());
            match op {
                "+" => Fx::Float(a + b),
                "-" => Fx::Float(a - b),
                "*" => Fx::Float(a * b),
                "/" => Fx::Float(a / b),
                _ => Fx::Bool(a.partial_cmp(&b).is_some_and(cmp)),
            }
        }
        Mode::UInt | Mode::Int => {
            let (a, b) = (l.int(), r.int());
            if !arith {
                return Fx::Bool(cmp(a.cmp(&b)));
            }
            let v = match op {
                "+" => a.wrapping_add(b),
                "-" => a.wrapping_sub(b),
                "*" => a.wrapping_mul(b),
                _ => a.checked_div(b).unwrap_or(0),
            };
            if matches!(mode, Mode::UInt) { Fx::UInt(v as u32) } else { Fx::Int(v as i32) }
        }
        Mode::Zero => {
            if arith {
                Fx::Int(0)
            } else {
                Fx::Bool(false)
            }
        }
    }
}

fn variable(v: &ssx_formats::lynx::Variable, inputs: &SprayInputs) -> VarValue {
    let source = v.source.as_deref().and_then(|s| s.strip_prefix("gamestate:")).map(|name| match name {
        "VEC_SnowSprayVelocity" => inputs.spray_velocity.length(),
        "VEC_Velocity" => inputs.velocity.length(),
        "BLN_InAir" => inputs.in_air as u8 as f32,
        "BLN_IsAwesome" => inputs.is_awesome as u8 as f32,
        "BLN_IsSuperAwesome" => inputs.is_super_awesome as u8 as f32,
        "BLN_IsTrackStarted" => inputs.track_started as u8 as f32,
        "BLN_State_Wipeout" => inputs.wipeout as u8 as f32,
        "BLN_IceAxeEffectsActive" => inputs.ice_axe_active as u8 as f32,
        "FLT_IceAxeBlendWeight" => inputs.ice_axe_blend,
        "FLT_Lean" => inputs.lean,
        "FLT_SurfaceWeightIce" => inputs.ice,
        "FLT_SurfaceWeightPowder" => inputs.powder,
        "FLT_SurfaceWeightRock" => inputs.rock,
        "FLT_DownwashVisualStrength" => inputs.downwash_visual_strength,
        _ => 0.0,
    });
    let default = v.default.as_deref().unwrap_or("");
    match v.ty.to_ascii_uppercase().as_str() {
        "BOOL" => VarValue::Bool(source.map_or(default.eq_ignore_ascii_case("true") || default == "1", |x| x != 0.0)),
        "INT" => VarValue::Int(source.map_or_else(|| default.trim().parse().unwrap_or(0), |x| x as i32)),
        _ => VarValue::Float(source.unwrap_or_else(|| default.trim().parse().unwrap_or(0.0))),
    }
}

pub fn effect_age_ms(clock: f32, start: f32) -> i32 {
    if start == -1.0 { -1 } else { ((clock - start) * 1000.0) as i32 }
}

pub fn trigger_type(b: &ssx_formats::lynx::Behaviour, inputs: &SprayInputs, age_ms: i32) -> i32 {
    let var = |name: &str| match name {
        "EFFECT_AGE_MS" => Some(VarValue::Int(age_ms)),
        _ => b.variable(name).map(|v| variable(v, inputs)),
    };
    let mut best = -1;
    for t in &b.triggers {
        let kind = match t.kind.as_str() {
            "TURN_ON" => 0,
            "TURN_OFF" => 1,
            "TURN_OFF_INSTANTLY" => 2,
            _ => continue,
        };
        if kind > best && t.conditions.iter().any(|c| fx_eval(&c.text, &var).float() != 0.0) {
            best = kind;
        }
    }
    best
}

pub fn apply_triggers(
    b: &ssx_formats::lynx::Behaviour,
    inputs: &SprayInputs,
    clock: f32,
    slot: (&mut f32, &mut f32, &mut bool),
) {
    let (start, stop, instant) = slot;
    match (trigger_type(b, inputs, effect_age_ms(clock, *start)), *start != -1.0) {
        (1, true) => (*start, *stop) = (-1.0, clock),
        (2, true) => (*stop, *instant) = (clock, true),
        (0, false) => (*start, *stop, *instant) = (clock, -1.0, false),
        _ => {}
    }
}

pub fn precipitation_override() -> Option<f32> {
    std::env::var("SSX_PRECIPITATION").ok().and_then(|v| v.parse::<f32>().ok()).map(|x| x.clamp(0.0, 1.0))
}

pub fn default_blend(
    behaviour: Option<&ssx_formats::lynx::Behaviour>,
    inputs: &SprayInputs,
    precipitation: f32,
) -> Vec<(u8, f32)> {
    let Some(b) = behaviour else {
        return vec![(1, 1.0 - precipitation), (2, precipitation)];
    };
    let Some(state) = b.blend_states.first() else { return Vec::new() };
    let var = |name: &str| b.variable(name).map(|v| variable(v, inputs));
    let axes: Vec<f32> = state
        .axes
        .iter()
        .map(|a| {
            let x = fx_eval(&a.weight.text, &var).float();
            let t = ((x - a.from[0]) / (a.from[1] - a.from[0])).clamp(0.0, 1.0);
            a.to[0] + (a.to[1] - a.to[0]) * t
        })
        .collect();
    match axes[..] {
        [s] => {
            let i = s.trunc();
            let f = s - i;
            vec![(i as u8, 1.0 - f), (i as u8 + 1, f)]
        }
        [s0, s1] => vec![(1, (1.0 - s0) * (1.0 - s1)), (2, s0 * (1.0 - s1)), (3, (1.0 - s0) * s1), (4, s0 * s1)],
        _ => Vec::new(),
    }
}

pub fn player_shadow(q: Option<&crate::particles::shadow_query::PlayerShadowQuery>) -> f32 {
    q.map_or(1.0, |q| q.value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str) -> Fx {
        let var = |n: &str| match n {
            "V" => Some(VarValue::Float(0.35)),
            "T" => Some(VarValue::Float(0.1)),
            "M" => Some(VarValue::Float(3.5)),
            "B" => Some(VarValue::Bool(true)),
            _ => None,
        };
        fx_eval(s, &var)
    }

    #[test]
    fn behaviour_expressions_follow_the_game() {
        assert!((eval("(V - T) * M").float() - 0.875).abs() < 1e-6);
        assert_eq!(eval("10-4-3").int(), 9);
        assert_eq!(eval("V*4").int(), 0);
        assert!((eval("V*4.0").float() - 1.4).abs() < 1e-6);
        assert!(matches!(eval("B && !false"), Fx::Bool(true)));
        assert!(matches!(eval("V > 0.5"), Fx::Bool(false)));
    }

    fn behaviour() -> ssx_formats::lynx::Behaviour {
        let xml = r#"<particleBehaviour name="t">
            <variable name="AIR" type="BOOL" source="gamestate:BLN_InAir"/>
            <trigger type="TURN_ON"><condition expression="AIR"/></trigger>
            <trigger type="TURN_OFF"><condition expression="!AIR"/></trigger>
            <trigger type="TURN_OFF_INSTANTLY"><condition expression="EFFECT_AGE_MS &gt; 500"/></trigger>
        </particleBehaviour>"#;
        ssx_formats::lynx::Behaviour::parse(xml.as_bytes()).unwrap()
    }

    #[test]
    fn triggers_switch_a_slot_as_the_game() {
        let b = behaviour();
        let mut inputs = SprayInputs { in_air: true, ..Default::default() };
        let (mut start, mut stop, mut instant) = (-1.0, -1.0, false);
        apply_triggers(&b, &inputs, 1.0, (&mut start, &mut stop, &mut instant));
        assert_eq!((start, stop, instant), (1.0, -1.0, false));
        apply_triggers(&b, &inputs, 1.2, (&mut start, &mut stop, &mut instant));
        assert_eq!((start, stop), (1.0, -1.0));
        apply_triggers(&b, &inputs, 1.6, (&mut start, &mut stop, &mut instant));
        assert_eq!((start, stop, instant), (1.0, 1.6, true));
        let (mut start, mut stop, mut instant) = (2.0, -1.0, false);
        inputs.in_air = false;
        apply_triggers(&b, &inputs, 2.1, (&mut start, &mut stop, &mut instant));
        assert_eq!((start, stop, instant), (-1.0, 2.1, false));
        assert_eq!(effect_age_ms(2.1, -1.0), -1);
    }
}
