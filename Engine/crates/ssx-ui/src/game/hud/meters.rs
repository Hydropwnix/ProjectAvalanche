use super::{Component, HudEnv};
use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::clip::{Clip, DI_ROTATION, DI_VISIBLE, DI_X, DI_Y, DisplayInfo};
use crate::game::values::id;

const BAR: f32 = 116.0;
const LOW: f32 = 0.25;
const BATTERY_STEP: f32 = 0.2;
const OXYGEN_LOW: f32 = 0.3;

#[derive(Default)]
struct Bar {
    meter: Option<Clip>,
    label: Option<Clip>,
    fill: Option<Clip>,
    dimmer: Option<Clip>,
    backer: Option<Clip>,
    kind: i32,
    last: f32,
    last_max: f32,
    labelled: bool,
    shown: bool,
    assigned: bool,
}

#[derive(Default)]
struct Battery {
    meter: Option<Clip>,
    label: Option<Clip>,
    housing: Option<Clip>,
    notches: [Option<Clip>; 5],
    kind: i32,
    last: f32,
    labelled: bool,
    shown: bool,
    assigned: bool,
}

pub struct HudMeters {
    container: Option<Clip>,
    bars: [Bar; 4],
    batteries: [Battery; 2],
    oxygen: Option<Clip>,
    needle: Option<Clip>,
    button: Option<Clip>,
    oxygen_last: f32,
    oxygen_ready: bool,
    oxygen_shown: bool,
    oxygen_equipped: bool,
    bound: bool,
    assigned: bool,
    container_shown: bool,
    updated: bool,
    armour_mode: bool,
    last_armour: f32,
    last_health: f32,
    last_health_max: f32,
}

impl Default for HudMeters {
    fn default() -> Self {
        let bar = || Bar { kind: -1, last: -1.0, last_max: -1.0, ..Default::default() };
        let battery = || Battery { kind: -1, last: -1.0, ..Default::default() };
        HudMeters {
            container: None,
            bars: [bar(), bar(), bar(), bar()],
            batteries: [battery(), battery()],
            oxygen: None,
            needle: None,
            button: None,
            oxygen_last: -1.0,
            oxygen_ready: false,
            oxygen_shown: false,
            oxygen_equipped: false,
            bound: false,
            assigned: false,
            container_shown: false,
            updated: false,
            armour_mode: false,
            last_armour: -1.0,
            last_health: -1.0,
            last_health_max: -1.0,
        }
    }
}

const BINDINGS: &[&str] = &[
    "db_hud_meters",
    "db_hud_generic_meter1",
    "db_hud_generic_meter1_label",
    "db_hud_generic_meter1_dimmer",
    "db_hud_generic_meter1_fill",
    "db_hud_generic_meter1_backer",
    "db_hud_generic_meter2",
    "db_hud_generic_meter2_label",
    "db_hud_generic_meter2_dimmer",
    "db_hud_generic_meter2_fill",
    "db_hud_generic_meter2_backer",
    "db_hud_generic_meter3",
    "db_hud_generic_meter3_label",
    "db_hud_generic_meter3_dimmer",
    "db_hud_generic_meter3_fill",
    "db_hud_generic_meter3_backer",
    "db_hud_generic_meter4",
    "db_hud_generic_meter4_label",
    "db_hud_generic_meter4_dimmer",
    "db_hud_generic_meter4_fill",
    "db_hud_generic_meter4_backer",
    "db_hud_battery_meter1",
    "db_hud_battery_meter1_label",
    "db_hud_battery_meter1_housing",
    "db_hud_battery_meter1_notch0",
    "db_hud_battery_meter1_notch1",
    "db_hud_battery_meter1_notch2",
    "db_hud_battery_meter1_notch3",
    "db_hud_battery_meter1_notch4",
    "db_hud_battery_meter2",
    "db_hud_battery_meter2_label",
    "db_hud_battery_meter2_housing",
    "db_hud_battery_meter2_notch0",
    "db_hud_battery_meter2_notch1",
    "db_hud_battery_meter2_notch2",
    "db_hud_battery_meter2_notch3",
    "db_hud_battery_meter2_notch4",
    "db_hud_oxygen_meter",
    "db_hud_oxygen_meter_needle",
    "db_hud_oxygen_meter_button",
];

fn slot<'a>(m: &'a mut HudMeters, name: &str) -> Option<&'a mut Option<Clip>> {
    if name == "db_hud_meters" {
        return Some(&mut m.container);
    }
    if let Some(rest) = name.strip_prefix("db_hud_generic_meter") {
        let (k, part) = rest.split_at(1);
        let b = m.bars.get_mut(k.parse::<usize>().ok()?.checked_sub(1)?)?;
        return match part {
            "" => Some(&mut b.meter),
            "_label" => Some(&mut b.label),
            "_dimmer" => Some(&mut b.dimmer),
            "_fill" => Some(&mut b.fill),
            "_backer" => Some(&mut b.backer),
            _ => None,
        };
    }
    if let Some(rest) = name.strip_prefix("db_hud_battery_meter") {
        let (k, part) = rest.split_at(1);
        let b = m.batteries.get_mut(k.parse::<usize>().ok()?.checked_sub(1)?)?;
        return match part {
            "" => Some(&mut b.meter),
            "_label" => Some(&mut b.label),
            "_housing" => Some(&mut b.housing),
            n => b.notches.get_mut(n.strip_prefix("_notch")?.parse::<usize>().ok()?),
        };
    }
    match name {
        "db_hud_oxygen_meter" => Some(&mut m.oxygen),
        "db_hud_oxygen_meter_needle" => Some(&mut m.needle),
        "db_hud_oxygen_meter_button" => Some(&mut m.button),
        _ => None,
    }
}

impl Component for HudMeters {
    fn bindings(&self) -> &'static [&'static str] {
        BINDINGS
    }

    fn bind(&mut self, _cx: &mut Ctx, _env: &mut HudEnv, name: &str, clip: Clip) {
        if let Some(s) = slot(self, name) {
            *s = Some(clip);
        }
        self.bound = true;
    }

    fn unbind(&mut self, name: &str) {
        self.bound = false;
        if let Some(s) = slot(self, name) {
            *s = None;
        }
    }

    fn update(&mut self, cx: &mut Ctx, env: &mut HudEnv, _dt: f64) {
        if !env.values.bool(id::METERS_ACTIVE) {
            if self.container_shown
                && let Some(c) = self.container.as_mut()
            {
                c.set_visible(cx, false);
                self.container_shown = false;
            }
            for b in &mut self.bars {
                if let Some(m) = b.meter.as_mut()
                    && b.shown
                {
                    m.set_visible(cx, false);
                    b.shown = false;
                }
            }
            for b in &mut self.batteries {
                if let Some(m) = b.meter.as_mut()
                    && b.shown
                {
                    m.set_visible(cx, false);
                    b.shown = false;
                }
            }
            if self.oxygen_shown
                && let Some(o) = self.oxygen.as_mut()
            {
                o.set_visible(cx, false);
                self.oxygen_shown = false;
            }
            return;
        }
        if !self.assigned
            && let Some(c) = self.container.as_mut()
        {
            c.set_visible(cx, false);
            self.assigned = true;
            self.container_shown = false;
            self.assign(env);
        }
        if self.bound {
            self.updated = false;
            self.update_bars(cx, env);
            self.update_batteries(cx, env);
            self.update_oxygen(cx, env);
            if !self.container_shown
                && self.updated
                && let Some(c) = self.container.as_mut()
            {
                c.set_visible(cx, true);
                self.container_shown = true;
            }
        }
    }
}

impl HudMeters {
    fn assign(&mut self, env: &mut HudEnv) {
        // TODO(guess): the game object's +0xE0 query is taken as "the `GearItem` slot is equipped" (`DropSurvival::slots`); settle: the object passed to 0x82596848 and its vtable +0xE0.
        let has = |n: usize| env.gear.get(n).copied().unwrap_or(false);
        let mut k = 0;
        if has(2) {
            (self.bars[0].assigned, self.bars[0].kind) = (true, 2);
            k = 1;
        }
        let mut next = k;
        if has(4) || env.values.bool(id::IS_THERMAL_REQUIRED) {
            (self.bars[k].assigned, self.bars[k].kind) = (true, 4);
            next = k + 1;
        }
        if has(3) || env.values.bool(id::IS_DEATH_PERMANENT) {
            (self.bars[next].assigned, self.bars[next].kind) = (true, 3);
            self.armour_mode = true;
        }
        let lamp = has(6);
        if lamp {
            (self.batteries[0].assigned, self.batteries[0].kind) = (true, 6);
        }
        if has(7) {
            let b = &mut self.batteries[usize::from(lamp)];
            (b.assigned, b.kind) = (true, 7);
        }
        if has(5) {
            self.oxygen_equipped = true;
        }
    }

    fn gap(&mut self, cx: &mut Ctx, with_batteries: bool) -> f64 {
        let mut gap = 0.0;
        if !self.oxygen_equipped
            && let Some(o) = self.oxygen.as_mut()
        {
            gap += o.get_number(cx, "_height");
        }
        if with_batteries {
            for b in &mut self.batteries {
                if !b.assigned
                    && let Some(m) = b.meter.as_mut()
                {
                    gap += m.get_number(cx, "_height");
                }
            }
        }
        gap
    }

    fn update_bars(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        for k in 0..self.bars.len() {
            if self.bars[k].meter.is_none() {
                continue;
            }
            if !self.bars[k].labelled {
                hide(cx, self.bars[k].meter.as_mut());
                if self.bars[k].assigned {
                    let key = match self.bars[k].kind {
                        1 => "TXT_METER_WINGSUIT",
                        2 => "TXT_METER_ICEAXE",
                        3 => "TXT_METER_ARMOR",
                        4 => "TXT_METER_THERMAL",
                        _ => "",
                    };
                    self.label_bar(cx, env, k, key);
                }
                continue;
            }
            if !self.bars[k].assigned {
                continue;
            }
            let v = &*env.values;
            match self.bars[k].kind {
                1 => fill_bar(cx, &mut self.bars[k], v.float(id::WINGSUIT_EFFECTIVENESS), 1.0),
                2 => fill_bar(cx, &mut self.bars[k], v.float(id::ICE_AXE_EFFECTIVENESS), 1.0),
                4 => fill_bar(cx, &mut self.bars[k], v.float(id::TEMPERATURE), v.float(id::TEMPERATURE_MAX)),
                _ => self.armour_bar(cx, env, k),
            }
            self.updated = true;
        }
    }

    fn label_bar(&mut self, cx: &mut Ctx, env: &mut HudEnv, k: usize, key: &str) {
        let gap = self.gap(cx, true);
        let text = if key.is_empty() { String::new() } else { env.loc.text(key) };
        let b = &mut self.bars[k];
        if let Some(m) = b.meter.as_mut() {
            let mut i = m.info(cx);
            i.y += gap;
            i.visible = false;
            m.set_info(cx, &i, DI_Y | DI_VISIBLE);
        }
        if let Some(c) = b.backer.as_mut() {
            c.set(cx, "_width", Value::Number(148.0));
            shift_x(cx, c, -100.0);
            c.goto_label(cx, "normal", true);
        }
        if let Some(c) = b.fill.as_mut() {
            c.set(cx, "_width", Value::Number(0.0));
            shift_x(cx, c, -100.0);
            c.goto_label(cx, "white", true);
        }
        if let Some(c) = b.label.as_mut() {
            c.set_text(cx, &text);
            shift_x(cx, c, -100.0);
        }
        if let Some(c) = b.dimmer.as_mut() {
            c.set(cx, "_width", Value::Number(0.0));
            shift_x(cx, c, -100.0);
        }
        b.shown = false;
        b.labelled = true;
    }

    fn armour_bar(&mut self, cx: &mut Ctx, env: &mut HudEnv, k: usize) {
        let v = &*env.values;
        let (armour, health, max) = (v.float(id::ARMOUR), v.float(id::HEALTH), v.float(id::HEALTH_MAX));
        if armour > 0.0 {
            if !self.armour_mode {
                let text = env.loc.text("TXT_METER_ARMOR");
                let b = &mut self.bars[k];
                set_text(cx, b.label.as_mut(), &text);
                goto(cx, b.fill.as_mut(), "white");
                goto(cx, b.backer.as_mut(), "normal");
                self.armour_mode = true;
                self.last_armour = -1.0;
            }
            if armour != self.last_armour {
                let b = &mut self.bars[k];
                set_width(cx, b.fill.as_mut(), (armour * BAR).ceil());
                set_width(cx, b.dimmer.as_mut(), BAR);
                if !b.shown || self.last_armour < 0.0 {
                    goto(cx, b.fill.as_mut(), "white");
                    goto(cx, b.backer.as_mut(), "normal");
                }
                self.last_armour = armour;
            }
        } else if health != self.last_health || max != self.last_health_max {
            if self.armour_mode {
                let text = env.loc.text("TXT_METER_HEALTH");
                let b = &mut self.bars[k];
                set_text(cx, b.label.as_mut(), &text);
                let (dim, back) = if health > LOW { ("yellow", "normal") } else { ("red", "warning") };
                goto(cx, b.fill.as_mut(), dim);
                goto(cx, b.backer.as_mut(), back);
                self.armour_mode = false;
                (self.last_health, self.last_health_max) = (-1.0, -1.0);
            }
            let last = self.last_health;
            let b = &mut self.bars[k];
            set_width(cx, b.fill.as_mut(), (health * BAR).ceil());
            set_width(cx, b.dimmer.as_mut(), max * BAR);
            if (!b.shown || last > LOW) && health < LOW {
                goto(cx, b.fill.as_mut(), "red");
                goto(cx, b.backer.as_mut(), "warning");
            } else if (!b.shown || last < LOW) && health > LOW {
                goto(cx, b.fill.as_mut(), "yellow");
                goto(cx, b.backer.as_mut(), "normal");
            }
            (self.last_health, self.last_health_max) = (health, max);
        }
        let b = &mut self.bars[k];
        if !b.shown {
            show(cx, b.meter.as_mut());
            b.shown = true;
        }
    }

    fn update_batteries(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        for k in 0..self.batteries.len() {
            if self.batteries[k].meter.is_none() {
                continue;
            }
            let kind = self.batteries[k].kind;
            if !self.batteries[k].labelled {
                hide(cx, self.batteries[k].meter.as_mut());
                if self.batteries[k].assigned {
                    match kind {
                        6 => self.label_battery(cx, env, k, "TXT_METER_HEADLAMP"),
                        7 => self.label_battery(cx, env, k, "TXT_METER_PULSEGOGGLES"),
                        _ => {}
                    }
                }
                continue;
            }
            if !self.batteries[k].assigned {
                continue;
            }
            match kind {
                6 => fill_battery(cx, &mut self.batteries[k], env.values.float(id::HEAD_LAMP_LEVEL)),
                7 => fill_battery(cx, &mut self.batteries[k], env.values.float(id::PULSE_GOGGLE_LEVEL)),
                _ => {}
            }
            self.updated = true;
        }
    }

    fn label_battery(&mut self, cx: &mut Ctx, env: &mut HudEnv, k: usize, key: &str) {
        let gap = self.gap(cx, false);
        let text = env.loc.text(key);
        let b = &mut self.batteries[k];
        if let Some(m) = b.meter.as_mut() {
            let mut i = m.info(cx);
            i.y += gap;
            i.visible = false;
            m.set_info(cx, &i, DI_Y | DI_VISIBLE);
        }
        set_text(cx, b.label.as_mut(), &text);
        b.shown = false;
        b.labelled = true;
    }

    fn update_oxygen(&mut self, cx: &mut Ctx, env: &mut HudEnv) {
        if self.oxygen.is_none() {
            return;
        }
        if !self.oxygen_ready {
            goto(cx, self.oxygen.as_mut(), "normal");
            goto(cx, self.needle.as_mut(), "normal");
            hide(cx, self.oxygen.as_mut());
            if let Some(n) = self.needle.as_mut() {
                n.set_info(cx, &DisplayInfo { rotation: -58.0, ..Default::default() }, DI_ROTATION);
            }
            let scheme = env.values.int(id::SETTINGS_GAMEPLAY_CONTROLLER_SCHEME);
            goto(cx, self.button.as_mut(), if scheme == 1 { "classic" } else { "xbox_rb" });
            self.oxygen_ready = true;
            self.oxygen_shown = false;
            return;
        }
        if !self.oxygen_equipped {
            return;
        }
        let level = env.values.float(id::OXYGEN_TANK_LEVEL);
        if level > 0.0 && env.values.bool(id::IS_OXYGEN_USE_REQUESTED) {
            goto(cx, self.oxygen.as_mut(), "engaged");
            goto(cx, self.needle.as_mut(), "engaged");
        } else if level.abs() <= 1.525_879e-5 {
            goto(cx, self.oxygen.as_mut(), "dead");
            goto(cx, self.needle.as_mut(), "normal");
        } else if level < OXYGEN_LOW {
            goto(cx, self.oxygen.as_mut(), "warning");
            goto(cx, self.needle.as_mut(), "normal");
        } else if level > OXYGEN_LOW {
            goto(cx, self.oxygen.as_mut(), "normal");
            goto(cx, self.needle.as_mut(), "normal");
        }
        if level != self.oxygen_last {
            let rotation = -(f64::from(1.0 - level) * 180.0 + 58.0);
            if let Some(n) = self.needle.as_mut() {
                n.set_info(cx, &DisplayInfo { rotation, ..Default::default() }, DI_ROTATION);
            }
            self.oxygen_last = level;
        }
        if !self.oxygen_shown && level > 0.0 {
            show(cx, self.oxygen.as_mut());
            self.oxygen_shown = true;
        }
        self.updated = true;
    }
}

fn fill_bar(cx: &mut Ctx, b: &mut Bar, value: f32, max: f32) {
    if value != b.last || max != b.last_max {
        set_width(cx, b.fill.as_mut(), (value * BAR).ceil());
        set_width(cx, b.dimmer.as_mut(), max * BAR);
        if (!b.shown || b.last > LOW) && value < LOW {
            goto(cx, b.fill.as_mut(), "red");
            goto(cx, b.backer.as_mut(), "warning");
        } else if (!b.shown || b.last < LOW) && value > LOW {
            goto(cx, b.fill.as_mut(), "white");
            goto(cx, b.backer.as_mut(), "normal");
        }
        (b.last, b.last_max) = (value, max);
    }
    if !b.shown {
        show(cx, b.meter.as_mut());
        b.shown = true;
    }
}

fn fill_battery(cx: &mut Ctx, b: &mut Battery, level: f32) {
    if level != b.last {
        for (k, n) in b.notches.iter_mut().enumerate() {
            if let Some(n) = n.as_mut() {
                n.set_visible(cx, (k as f32 * BATTERY_STEP) < level);
            }
        }
        if (!b.shown || b.last > BATTERY_STEP) && level < BATTERY_STEP {
            goto(cx, b.housing.as_mut(), "warning");
            for n in b.notches.iter_mut() {
                goto(cx, n.as_mut(), "red");
            }
        } else if (!b.shown || b.last < BATTERY_STEP) && level > BATTERY_STEP {
            goto(cx, b.housing.as_mut(), "normal");
            for n in b.notches.iter_mut() {
                goto(cx, n.as_mut(), "white");
            }
        }
        b.last = level;
    }
    if !b.shown {
        show(cx, b.meter.as_mut());
        b.shown = true;
    }
}

fn shift_x(cx: &mut Ctx, c: &mut Clip, dx: f64) {
    let mut i = c.info(cx);
    i.x += dx;
    c.set_info(cx, &i, DI_X);
}

fn set_width(cx: &mut Ctx, c: Option<&mut Clip>, w: f32) {
    if let Some(c) = c {
        c.set(cx, "_width", Value::Number(f64::from(w)));
    }
}

fn set_text(cx: &mut Ctx, c: Option<&mut Clip>, text: &str) {
    if let Some(c) = c {
        c.set_text(cx, text);
    }
}

fn goto(cx: &mut Ctx, c: Option<&mut Clip>, label: &str) {
    if let Some(c) = c {
        c.goto_label(cx, label, true);
    }
}

fn show(cx: &mut Ctx, c: Option<&mut Clip>) {
    if let Some(c) = c {
        c.set_visible(cx, true);
    }
}

fn hide(cx: &mut Ctx, c: Option<&mut Clip>) {
    if let Some(c) = c {
        c.set_visible(cx, false);
    }
}
