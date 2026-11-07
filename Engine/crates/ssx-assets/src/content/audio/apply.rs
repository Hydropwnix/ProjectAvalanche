use std::collections::BTreeMap;
use std::sync::Arc;

use ssx_content::audio::music::TrackAudio;
use ssx_content::audio::{Limits, Pcm, decode_file};

use super::decide::{MusicFile, MusicPlan, TakenScore, TakenTrack};
use crate::music::PathfinderMusic;
use crate::score::{Decoded, ScorePlayer};
use crate::track_override::TrackOverride;

const RATE: u32 = crate::pathfinder::SAMPLE_RATE;

pub fn stereo44(pcm: Pcm) -> Vec<f32> {
    let mono = pcm.channels == 1;
    let frames = super::convert::to_rate(pcm, RATE);
    if mono { frames.iter().flat_map(|&x| [x, x]).collect() } else { frames }
}

fn decode(file: &MusicFile) -> Result<Vec<f32>, String> {
    let bytes = file.read()?;
    let pcm = decode_file(&file.rel, bytes, &Limits::music()).map_err(|e| e.to_string())?;
    super::decide::as_measured(&file.rel, Some(file.info), &pcm)?;
    Ok(stereo44(pcm))
}

fn frames(seconds: f64) -> usize {
    (seconds * f64::from(RATE)).round() as usize
}

pub fn track_override(t: &TakenTrack) -> Result<TrackOverride, String> {
    let pcm = Arc::new(decode(&t.file)?);
    let n = pcm.len() / 2;
    let label = format!("{} ({})", t.from.id, t.from.package);
    let clip = |a: f64, b: f64, what: &str| -> Result<(usize, usize), String> {
        let (a, b) = (frames(a).min(n), frames(b).min(n));
        if a >= b { Err(format!("{label}: {what} lies outside the file's {n} frames")) } else { Ok((a, b)) }
    };
    match &t.audio {
        TrackAudio::Looped { region, .. } => {
            let region = match region {
                Some(r) => clip(r.start, r.end, "the loop")?,
                None => (0, n),
            };
            Ok(TrackOverride { pcm, region, sections: None, gain: t.gain, label })
        }
        TrackAudio::Sectioned { sections, .. } => {
            let mut map = BTreeMap::new();
            for (node, r) in sections {
                let node = u16::try_from(*node).map_err(|_| format!("{label}: node {node} out of range"))?;
                map.insert(node, clip(r.start, r.end, &format!("the section of node {node}"))?);
            }
            Ok(TrackOverride { pcm, region: (0, n), sections: Some(map), gain: t.gain, label })
        }
    }
}

pub fn score(t: &TakenScore) -> Result<ScorePlayer, String> {
    let mut decoded: Decoded = BTreeMap::new();
    for (rel, file) in &t.files {
        decoded.insert(rel.clone(), Arc::new(decode(file)?));
    }
    ScorePlayer::new(t.score.clone(), &decoded).map_err(|e| format!("{} ({}): {e}", t.from.id, t.from.package))
}

pub fn apply_project(music: &mut PathfinderMusic, project: &str, plan: &MusicPlan, log: &mut Vec<String>) {
    if let Some(t) = plan.scores.get(project) {
        match score(t) {
            Ok(s) => {
                music.set_score(s, &format!("{} ({})", t.from.id, t.from.package));
                log.push(format!("music: {project}: the score of {} ({})", t.from.id, t.from.package));
                return;
            }
            Err(e) => log.push(format!("content: {} not used (the disc's score is): {e}", t.from.id)),
        }
    }
    for ((p, track), t) in &plan.tracks {
        if p != project {
            continue;
        }
        match track_override(t) {
            Ok(o) => {
                log.push(format!("music: {project} track {track}: {}", o.label));
                music.set_track_override(*track, o);
            }
            Err(e) => log.push(format!("content: {} not used (the disc's track is): {e}", t.from.id)),
        }
    }
}
