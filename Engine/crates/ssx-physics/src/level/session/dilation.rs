use anyhow::{Result, bail};

use crate::curve::Curve;
use crate::tuning::Bundle;

#[derive(Clone, Debug, PartialEq)]
pub struct TimeDilationTuning {
    pub heli_trick: Option<(Curve, f32)>,
}

impl Default for TimeDilationTuning {
    fn default() -> Self {
        let points = [
            [0.0, 1.0],
            [6.677_471, 0.722_837_8],
            [26.580_225, 0.341_463_1],
            [40.129_654, 0.133_037_33],
            [53.938_404, 0.283_813_48],
            [77.8606, 0.736_141_56],
            [80.0, 1.0],
        ];
        let curve = Curve { name: "curve_time_dilation_vs_time_heli_trick".into(), points: points.to_vec() };
        Self { heli_trick: Some((curve, 80.0)) }
    }
}

impl TimeDilationTuning {
    pub fn load(bundle: &Bundle) -> Result<Self> {
        let a = bundle.view("GameLoopSceneOp", "GameLoopSceneOpAsset")?;
        let curves = match a.asset_opt("TimeDilationHeliTrickCurvesAsset")? {
            Some(list) => list.asset_list("AssetListItems")?,
            None => Vec::new(),
        };
        let chance = a.f32("TimeDilationHeliTrickChance")?;
        if curves.len() > 1 || chance < 0.99 {
            bail!("GameLoopSceneOp: {} heli-trick curves, chance {chance}: the draws would decide", curves.len());
        }
        let heli_trick = match curves.first() {
            Some(c) => Some((c.as_curve()?, c.f32("MaxX")?)),
            None => None,
        };
        Ok(Self { heli_trick })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DilationState {
    #[default]
    None,
    HeliTrick,
    Rewind,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimeDilation {
    pub state: DilationState,
    pub timer: f32,
    pub prev_heli: bool,
}

impl TimeDilation {
    pub fn update(&mut self, t: &TimeDilationTuning, heli: bool, rewind: Option<f32>, raw_dt: f32) -> f32 {
        let rising = heli && !self.prev_heli;
        match self.state {
            DilationState::None if rising => self.set(DilationState::HeliTrick),
            DilationState::None if rewind.is_some() => self.set(DilationState::Rewind),
            DilationState::HeliTrick => match &t.heli_trick {
                Some((_, max_x)) if self.timer < *max_x => self.timer += raw_dt,
                _ => self.set(DilationState::None),
            },
            DilationState::Rewind if rewind.is_none() => self.set(DilationState::None),
            _ => {}
        }
        self.prev_heli = heli;
        match self.state {
            DilationState::HeliTrick => t.heli_trick.as_ref().map_or(1.0, |(c, _)| c.eval(self.timer)),
            DilationState::Rewind => rewind.unwrap_or(1.0),
            DilationState::None => 1.0,
        }
    }

    fn set(&mut self, to: DilationState) {
        if to != self.state {
            (self.state, self.timer) = (to, 0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heli_trick_runs_the_curve_for_80_raw_ticks() {
        let t = TimeDilationTuning::default();
        let mut d = TimeDilation::default();
        assert_eq!(d.update(&t, false, None, 2.0), 1.0);
        assert_eq!(d.update(&t, true, None, 2.0), 1.0);
        assert_eq!(d.state, DilationState::HeliTrick);
        let mut factors = Vec::new();
        for _ in 0..40 {
            factors.push(d.update(&t, false, Some(0.3), 2.0));
        }
        assert!((factors[19] - t.heli_trick.as_ref().unwrap().0.eval(40.0)).abs() < 1e-6);
        assert!(factors.iter().take(39).all(|&f| f < 1.0) && factors[39] == 1.0);
        assert_eq!(d.update(&t, false, Some(0.3), 2.0), 1.0);
        assert_eq!(d.state, DilationState::None);
        assert_eq!(d.update(&t, false, Some(0.3), 2.0), 0.3);
        assert_eq!(d.state, DilationState::Rewind);
        assert_eq!(d.update(&t, true, Some(0.3), 2.0), 0.3);
        assert_eq!(d.update(&t, true, None, 2.0), 1.0);
        assert_eq!(d.state, DilationState::None);
        assert_eq!(d.update(&t, true, None, 2.0), 1.0, "a held pulse is no rising edge");
        assert_eq!(d.state, DilationState::None);
    }
}
