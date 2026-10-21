use glam::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Precipitation = 0,
    Direction = 1,
    Speed = 2,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WeatherStacks {
    pub precipitation: Vec<(f32, u32)>,
    pub direction: Vec<(Vec3, u32)>,
    pub speed: Vec<(f32, u32)>,
    pub next_id: u32,
    pub changes: [u32; 3],
}

impl Default for WeatherStacks {
    fn default() -> Self {
        let mut w = WeatherStacks {
            precipitation: Vec::new(),
            direction: Vec::new(),
            speed: Vec::new(),
            next_id: 1,
            changes: [0; 3],
        };
        w.push_precipitation(0.0);
        w.push_direction(Vec3::NEG_X);
        w.push_speed(0.0);
        w
    }
}

impl WeatherStacks {
    fn id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push_precipitation(&mut self, x: f32) -> u32 {
        let id = self.id();
        self.precipitation.push((x, id));
        self.changes[Channel::Precipitation as usize] += 1;
        id
    }

    pub fn push_direction(&mut self, d: Vec3) -> u32 {
        let id = self.id();
        self.direction.push((d, id));
        self.changes[Channel::Direction as usize] += 1;
        id
    }

    pub fn push_speed(&mut self, s: f32) -> u32 {
        let id = self.id();
        self.speed.push((s, id));
        self.changes[Channel::Speed as usize] += 1;
        id
    }

    pub fn pop(&mut self, channel: Channel, id: u32) {
        match channel {
            Channel::Precipitation => self.precipitation.retain(|e| e.1 != id),
            Channel::Direction => self.direction.retain(|e| e.1 != id),
            Channel::Speed => self.speed.retain(|e| e.1 != id),
        }
        self.changes[channel as usize] += 1;
    }

    pub fn targets(&self) -> (f32, Vec3, f32) {
        (
            self.precipitation.last().map_or(0.0, |e| e.0),
            self.direction.last().map_or(Vec3::ZERO, |e| e.0),
            self.speed.last().map_or(0.0, |e| e.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushes_pop_back_to_the_default_preset() {
        let mut w = WeatherStacks::default();
        assert_eq!(w.targets(), (0.0, Vec3::NEG_X, 0.0));
        let a = w.push_precipitation(0.5);
        let b = w.push_precipitation(1.0);
        w.pop(Channel::Precipitation, a);
        assert_eq!(w.targets().0, 1.0, "the top stays");
        w.pop(Channel::Precipitation, b);
        assert_eq!(w.targets().0, 0.0);
        assert_eq!(w.changes, [5, 1, 1]);
    }
}
