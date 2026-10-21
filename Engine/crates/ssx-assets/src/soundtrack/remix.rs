use std::path::Path;

use anyhow::{Context, Result};
use ssx_formats::ptch::{Patch, Ptch};

pub const PATCHES: [(&str, &str); 22] = [
    ("HarmonyMusicBuses", "music_buses.ptc"),
    ("GameInputPatch", "game_input_patch.ptc"),
    ("JumpFilterEnvelopePatch", "jump_filter_envelope.ptc"),
    ("LandStutterLfoPatch", "land_stutter_lfo.ptc"),
    ("CarveToFilterFreqPatch", "carve_to_filter_freq.ptc"),
    ("MainDspPatch", "main_dsp.ptc"),
    ("WingsuitSHPatch", "wingsuit_s_h.ptc"),
    ("BoostPatch", "boost_remix_patch.ptc"),
    ("TimeDilationPatch", "time_dilation_remix_patch.ptc"),
    ("TumblePatch", "tumble_remix_patch.ptc"),
    ("AwesomeModePatch", "awesome_mode_remix_patch.ptc"),
    ("AwesomeLandPatch", "awesome_land_remix_patch.ptc"),
    ("GrindingPatch", "grinding_remix_patch.ptc"),
    ("TunnelPatch", "tunnel_remix_patch.ptc"),
    ("DeathPatch", "death_remix_patch.ptc"),
    ("PitchPatch", "pitch_remix_patch.ptc"),
    ("OxygenPatch", "oxygen_remix_patch.ptc"),
    ("GrindingSHPatch", "grinding_s_h_patch.ptc"),
    ("IcePatch", "ice_remix_patch.ptc"),
    ("RewindPatch", "rewind_patch.ptc"),
    ("Debug_TriggerPatch", "debug_triggers_patch.ptc"),
    ("SuperAwesomeModePatch", "super_awesome_mode_remix_patch.ptc"),
];

pub const TICK: f32 = 1.0 / 30.0;
pub const BLOCK: usize = 256;
pub const RAMP: usize = 64;
pub const GAME_RATE: f32 = 48_000.0;

pub const INPUTS: [(&str, &str); 23] = [
    ("UpdateCarve", "Carve Amount"),
    ("Jump", "Jump"),
    ("Land", "Land"),
    ("WingsuitDeployed", "Wingsuit Open"),
    ("WingsuitRetracted", "Wingsuit Close"),
    ("TimeDilationStart", "Time Dilation Start"),
    ("TimeDilationStop", "Time Dilation Stop"),
    ("BoostStart", "Boost Start"),
    ("BoostStop", "Boost Stop"),
    ("TumbleStart", "Tumble Start"),
    ("TumbleStop", "Tumble Stop"),
    ("AwesomeModeStart", "Awesome Mode Start"),
    ("AwesomeModeStop", "Awesome Mode Stop"),
    ("SuperAwesomeModeStart", "Super Awesome Mode Start"),
    ("SuperAwesomeModeStop", "Super Awesome Mode Stop"),
    ("AwesomeLandStart", "Awesome Land Start"),
    ("AwesomeLandStop", "Awesome Land Stop"),
    ("GrindingStart", "Grinding Start"),
    ("GrindingStop", "Grinding Stop"),
    ("TunnelStart", "Tunnel Start"),
    ("TunnelStop", "Tunnel Stop"),
    ("UpdatePitch", "Pitch Input"),
    ("PlayerDeath", "Player Death Start"),
];

pub struct RemixPatches {
    pub patches: Vec<(&'static str, Patch)>,
}

impl RemixPatches {
    pub fn load(big_root: &Path) -> Result<Self> {
        let mut patches = Vec::new();
        for (name, file) in PATCHES {
            let rel = format!("data/xenon/audio/eaaudiocontroller/{file}");
            let path = crate::music::find_file(big_root, &rel).with_context(|| rel.clone())?;
            match Ptch::parse(&std::fs::read(&path)?).with_context(|| rel.clone())? {
                Ptch::V4(p) => patches.push((name, p)),
                Ptch::V1(_) => anyhow::bail!("{rel}: a v1 patch where the controller expects v4"),
            }
        }
        Ok(Self { patches })
    }

    pub fn unconnected_inputs(&self) -> Vec<String> {
        let mut outs = Vec::new();
        for (_, p) in &self.patches {
            for port in p.ports.iter().filter(|q| !q.input) {
                outs.push((port.name.clone(), p.interfaces[port.interface as usize].hash));
            }
        }
        let mut v = Vec::new();
        for (name, p) in &self.patches {
            for port in p.ports.iter().filter(|q| q.input) {
                let key = (port.name.clone(), p.interfaces[port.interface as usize].hash);
                if !outs.contains(&key) {
                    v.push(format!("{name}: {}", port.name));
                }
            }
        }
        v
    }
}

pub fn convert_unit(v: f64, from: u8, to: u8) -> Option<f64> {
    if from == to || from == 0 || to == 0 {
        return Some(v);
    }
    Some(match (from, to) {
        (1, 4) => 20.0 * v.log10(),
        (4, 1) => 10f64.powf(v * 0.05),
        (5, 6) => v * 1000.0,
        (6, 5) => v * 0.001,
        (9, 10) => v * 0.017_453_292_5,
        (10, 9) => v * 57.295_779_5,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_convert_only_between_pairs() {
        assert!((convert_unit(0.0, 4, 1).unwrap() - 1.0).abs() < 1e-12);
        assert!((convert_unit(20.0, 4, 1).unwrap() - 10.0).abs() < 1e-9);
        assert!((convert_unit(10.0, 1, 4).unwrap() - 20.0).abs() < 1e-12);
        assert_eq!(convert_unit(2.0, 5, 6), Some(2000.0));
        assert_eq!(convert_unit(2.0, 5, 9), None);
        assert_eq!(convert_unit(3.0, 0, 9), Some(3.0));
    }
}
