use bevy::prelude::*;
use ssx_physics::weather::{Channel as Stack, WeatherStacks};

use crate::particles::sim::Weather;

#[derive(Clone, Debug)]
struct Channel<T> {
    target: T,
    value: T,
    elapsed: f32,
}

impl<T: Copy> Channel<T> {
    fn new(initial: T) -> Self {
        Channel { target: initial, value: initial, elapsed: 0.0 }
    }

    fn retarget(&mut self, target: T) {
        (self.target, self.elapsed) = (target, 0.0);
    }

    fn step(&mut self, dt: f32) -> f32 {
        self.elapsed = (self.elapsed + dt).min(1.0);
        self.elapsed
    }
}

#[derive(Clone, Debug)]
pub struct WeatherManager {
    precipitation: Channel<f32>,
    direction: Channel<Vec3>,
    speed: Channel<f32>,
    seen: [u32; 3],
}

impl Default for WeatherManager {
    fn default() -> Self {
        let stacks = WeatherStacks::default();
        let (p, d, s) = stacks.targets();
        WeatherManager {
            precipitation: Channel::new(p),
            direction: Channel::new(d),
            speed: Channel::new(s),
            seen: stacks.changes,
        }
    }
}

impl WeatherManager {
    pub fn follow(&mut self, stacks: &WeatherStacks) {
        let (p, d, s) = stacks.targets();
        let changed = |ch: Stack, seen: &mut [u32; 3]| {
            let i = ch as usize;
            let c = stacks.changes[i] != seen[i];
            seen[i] = stacks.changes[i];
            c
        };
        if changed(Stack::Precipitation, &mut self.seen) {
            self.precipitation.retarget(p);
        }
        if changed(Stack::Direction, &mut self.seen) {
            self.direction.retarget(d);
        }
        if changed(Stack::Speed, &mut self.seen) {
            self.speed.retarget(s);
        }
    }

    pub fn update(&mut self, dt: f32) {
        let t = self.precipitation.step(dt);
        let p = &mut self.precipitation;
        p.value = p.target + (p.value - p.target) * t;
        let td = self.direction.step(dt);
        let mut len = 1.0;
        if td < 1.0 {
            let d = &mut self.direction;
            let v = d.target + (d.value - d.target) * td;
            len = v.length();
            d.value = if len > 0.0 { v / len } else { Vec3::ZERO };
        }
        let ts = self.speed.step(dt);
        let s = &mut self.speed;
        s.value = (s.target + (s.value - s.target) * ts) * len;
    }

    pub fn weather(&self) -> Weather {
        Weather { wind: self.direction.value * self.speed.value, precipitation: self.precipitation.value }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_pushes_blends_and_pops_back() {
        let (mut m, mut stacks) = (WeatherManager::default(), WeatherStacks::default());
        m.follow(&stacks);
        m.update(1.0 / 30.0);
        assert_eq!(m.weather().precipitation, 0.0);
        let id = stacks.push_precipitation(0.5);
        m.follow(&stacks);
        m.update(1.0 / 30.0);
        assert!((m.weather().precipitation - 0.5 * (1.0 - 1.0 / 30.0)).abs() < 1e-6);
        for _ in 0..40 {
            m.follow(&stacks);
            m.update(1.0 / 30.0);
        }
        assert!((m.weather().precipitation - 0.5).abs() < 1e-6);
        stacks.pop(Stack::Precipitation, id);
        for _ in 0..40 {
            m.follow(&stacks);
            m.update(1.0 / 30.0);
        }
        assert!(m.weather().precipitation.abs() < 1e-6);
    }
}
