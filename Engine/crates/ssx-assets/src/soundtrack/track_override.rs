use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TrackOverride {
    pub pcm: Arc<Vec<f32>>,
    pub region: (usize, usize),
    pub sections: Option<BTreeMap<u16, (usize, usize)>>,
    pub gain: f32,
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct OverrideCursor {
    pub pos: f64,
    pub region: Option<(usize, usize)>,
    pub section: Option<u16>,
    pub started: bool,
}

impl TrackOverride {
    pub fn enter(&self, c: &mut OverrideCursor, section: u16) -> Option<String> {
        match &self.sections {
            None => {
                if !c.started {
                    (c.pos, c.region, c.started) = (self.region.0 as f64, Some(self.region), true);
                    return Some(format!("{}: plays from {:.3} s", self.label, self.region.0 as f64 / RATE));
                }
                None
            }
            Some(map) => {
                if c.started && c.section == Some(section) {
                    return None;
                }
                c.section = Some(section);
                c.started = true;
                c.region = map.get(&section).copied();
                match c.region {
                    Some((a, _)) => {
                        c.pos = a as f64;
                        Some(format!("{}: section n{section} plays from {:.3} s", self.label, a as f64 / RATE))
                    }
                    None => Some(format!("{}: section n{section} has no region in the package: silent", self.label)),
                }
            }
        }
    }

    pub fn frame(&self, c: &mut OverrideCursor, ratio: f64) -> [f32; 2] {
        let Some((a, b)) = c.region else { return [0.0; 2] };
        let n = self.pcm.len() / 2;
        let i = c.pos as usize;
        let frac = (c.pos - i as f64) as f32;
        let at = |k: usize| {
            let k = k.min(n.saturating_sub(1));
            [self.pcm[2 * k], self.pcm[2 * k + 1]]
        };
        if n == 0 {
            return [0.0; 2];
        }
        let (x, y) = (at(i), at(if i + 1 < b { i + 1 } else { a }));
        c.pos += ratio;
        if c.pos >= b as f64 {
            c.pos = a as f64 + (c.pos - b as f64) % (b - a).max(1) as f64;
        }
        [(x[0] + (y[0] - x[0]) * frac) * self.gain, (x[1] + (y[1] - x[1]) * frac) * self.gain]
    }
}

const RATE: f64 = crate::pathfinder::SAMPLE_RATE as f64;
