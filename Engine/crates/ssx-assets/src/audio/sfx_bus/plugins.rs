use super::*;

pub(super) type Attr = (f64, f64, f64);
const BIG: f64 = 100_000.0;
const HUGE: f64 = 3.402_823_466e38;

pub fn descriptor(fourcc: &[u8; 4]) -> (usize, &'static [Attr]) {
    match fourcc {
        b"Gai0" | b"Sen0" => (0, &[(1.0, -BIG, BIG)]),
        b"GaF0" => (0, &[(1.0, 0.0, BIG)]),
        b"Rou0" => (0, &[(1.0, -BIG, BIG), (0.0, 0.0, 63.0), (0.0, 0.0, 7.0), (0.0, 0.0, 8.0)]),
        b"Rsp0" => (0, &[(1.0, 0.0, 32.0)]),
        b"HI20" => (0, &[(0.0, 0.0, 96_000.0)]),
        b"LI20" | b"LF60" => (0, &[(96_000.0, 0.0, 96_000.0)]),
        b"LS20" => (0, &[(0.0, 0.0, 96_000.0), (1.0, 0.05, 20.0)]),
        b"HS20" => (0, &[(96_000.0, 0.0, 96_000.0), (1.0, 0.05, 20.0)]),
        b"PI20" => (0, &[(96_000.0, 0.0, 96_000.0), (1.0, 0.05, 20.0), (3.0, 0.2, 20.0)]),
        b"Co10" => (0, &[(20.0, -500.0, 20.0), (1.0, 1.0, 100.0), (0.1, 0.0, 10.0), (0.5, 0.0, 30.0), (1.0, 0.0, 1.0)]),
        b"Li10" => (0, &[(20.0, -500.0, 20.0), (0.1, 0.0, 10.0), (1.0, 0.0, 1.0)]),
        b"DCl0" => (0, &[(100.0, 0.0, 1000.0)]),
        b"MtP0" => (0, &[(1.0, -2.0, 2.0); 8]),
        b"Del0" => (1, &[(0.0, 0.0, 20.0), (0.0, -0.99, 0.99)]),
        b"Pau0" => (0, &[(0.0, 0.0, 1.0)]),
        b"Pn21" => (
            4,
            &[
                (0.0, -HUGE, HUGE),
                (1.0, -HUGE, HUGE),
                (1.0, 0.0, HUGE),
                (0.0, -HUGE, HUGE),
                (1.0, 0.0, 1.0),
                (1.0, 0.0, 1.0),
                (0.0, 0.0, 1.0),
                (30.0, 1.0, 90.0),
                (110.0, 90.0, 180.0),
                (150.0, 90.0, 180.0),
                (1.0, 0.0, 1.0),
            ],
        ),
        b"Pn30" => (5, &[(1.0, -BIG, BIG)]),
        b"RIR1" => (
            2,
            &[
                (1.0, -BIG, BIG),
                (0.0, -BIG, BIG),
                (0.0, 0.0, 2.0),
                (1.0, 0.0, 10.0),
                (1.0, 0.0, 100.0),
                (0.0, 0.0, 2.0),
                (1.0, 0.0, 100.0),
                (1.0, 0.0, 100.0),
            ],
        ),
        b"BF60" => (0, &[(0.0, 0.0, 96_000.0), (192_000.0, 0.0, 192_000.0)]),
        b"Cho0" => (2, &[(0.0, 0.0, 8.0), (0.0, 0.0, 10.0), (0.0, 0.0, 1.0), (0.0, 0.0, 5.0)]),
        b"Vib0" => (1, &[(0.0, 0.0, 1.0), (0.0, 0.0, 5.0)]),
        b"FSS0" => (1, &[(0.0, -96_000.0, 96_000.0)]),
        b"RM10" => (1, &[(0.0, 0.0, 100.0), (15.0, 2.0, 80.0), (1.0, 0.0, 100.0)]),
        b"Bea0" => (
            3,
            &[
                (120.0, 0.0, 1200.0),
                (0.5, 0.0, 10.0),
                (0.0, 0.0, 1.0),
                (2.0, 2.0, 3.0),
                (0.0, 0.0, 1.0),
                (4.0, 0.75, 10.0),
                (0.03, 0.0, 10.0),
            ],
        ),
        b"Sin0" => (0, &[(440.0, 0.0, 96_000.0)]),
        b"Sub0" => (1, &[]),
        b"SnP1" => (1, &[]),
        _ => (0, &[]),
    }
}

pub fn ctor_default(fourcc: &[u8; 4], i: usize) -> f64 {
    match (fourcc, i) {
        (b"Cho0", 0 | 1) | (b"Vib0", 0) => 0.03,
        (b"FSS0", 0) | (b"Bea0", 0) => 1.0,
        (b"RM10", 0) => 2.0,
        (b"Bea0", 2) => 400.0,
        (b"Pn21", 0) | (b"Pn30", 2) => 30.0,
        (b"Pn21", 1) | (b"Pn30", 3) => 110.0,
        (b"Pn21", 2) => 2.0,
        (b"Pn21", 3) | (b"Pn30", 4) => 150.0,
        (b"Pn30", 0) => 0.2,
        _ => 0.0,
    }
}

pub(super) fn make_stage(fourcc: &[u8; 4], outputs: usize, ctor: &[f64]) -> Stage {
    let c = |i: usize| ctor.get(i).copied().unwrap_or(0.0);
    match fourcc {
        b"SnP1" => Stage::Source,
        b"Gai0" => Stage::Gain { last: None },
        b"GaF0" => Stage::Fader { fade: None },
        b"Sen0" => Stage::Send { last: None, target: None, key: (String::new(), 0) },
        b"Rou0" => Stage::Route { last: None, target: None, key: (String::new(), 0), window: (0, 0, 0) },
        b"Sub0" => Stage::SubMix { input: None },
        b"HI20" => Stage::Filter(dsp::FilterKind::HighPass, dsp::Filter::default()),
        b"LI20" => Stage::Filter(dsp::FilterKind::LowPass, dsp::Filter::default()),
        b"LS20" => Stage::Filter(dsp::FilterKind::LowShelf, dsp::Filter::default()),
        b"HS20" => Stage::Filter(dsp::FilterKind::HighShelf, dsp::Filter::default()),
        b"PI20" => Stage::Filter(dsp::FilterKind::Peaking, dsp::Filter::default()),
        b"Co10" => Stage::Compressor { state: dsp::Compressor::default(), side: None },
        b"DCl0" => Stage::Clip,
        b"MtP0" => Stage::Matrix { last: [1.0; 8] },
        b"Pn21" => Stage::Pan { last: Vec::new() },
        b"Pn30" => Stage::Pan3d(Box::new(dsp::pan3d::Pan3d::new(outputs, c(0) as f32, c(2) as f32, c(3) as f32))),
        b"BF60" => Stage::BandPass(Box::new(fx::BandPassFir64::new(outputs))),
        b"FSS0" => Stage::FreqShift(Box::new(fx::FrequencyShift::new(c(0) != 0.0))),
        b"Del0" => Stage::Delay(Box::new(fx::Delay::new(outputs, c(0)))),
        b"Cho0" => Stage::Chorus(Box::new(fx::ModDelay::new(outputs, (c(0) + 2.0 * c(1)) * 1.7))),
        b"Vib0" => Stage::Vibrato(Box::new(fx::ModDelay::new(outputs, 2.0 * c(0)))),
        b"RM10" => Stage::Reverb(Box::new(fx::Reverb::new(c(0)))),
        b"Bea0" => Stage::BeatDetect(Box::new(fx::BeatDetect::new(outputs, c(0), c(1), c(2)))),
        b"Sin0" => Stage::Silent,
        b"RIR1" => Stage::ReverbIr(Box::new(dsp::reverb_ir::ReverbIr::new(c(0) as f32, c(1) as i32, outputs, outputs))),
        b"Li10" => Stage::Limiter(dsp::Compressor::default()),
        b"PPl0" => Stage::Source,
        _ => Stage::Pass,
    }
}
