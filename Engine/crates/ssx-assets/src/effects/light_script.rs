#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightBehaviour {
    Off,
    On,
    Flicker { strength: f32 },
    Blink { strength: f32, frequency: f32, gap: f32 },
}

impl LightBehaviour {
    pub fn factor(&self, t: f32, r: f32) -> f32 {
        match *self {
            LightBehaviour::Off => 0.0,
            LightBehaviour::On => 1.0,
            LightBehaviour::Flicker { strength } => 1.0 - strength * r,
            LightBehaviour::Blink { strength, frequency, gap } => {
                let on = 1.0 / frequency;
                let period = on + gap;
                let phase = t - (t / period).trunc() * period;
                if phase <= on {
                    ((phase / on - 0.5) * std::f32::consts::TAU).cos() * strength + (1.0 - strength)
                } else {
                    1.0 - strength
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_and_flicker_factors() {
        let b = LightBehaviour::Blink { strength: 0.4, frequency: 0.5, gap: 0.0 };
        assert!((b.factor(0.0, 0.0) - 0.2).abs() < 1e-6);
        assert!((b.factor(1.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((b.factor(2.5, 0.0) - b.factor(0.5, 0.0)).abs() < 1e-5);
        let f = LightBehaviour::Flicker { strength: 0.5 };
        assert_eq!(f.factor(0.0, 0.0), 1.0);
        assert_eq!(f.factor(0.0, 0.5), 0.75);
    }
}
