use std::path::Path;

use anyhow::Result;

use crate::content::export_audio::wav_f32;

fn manifest(name: &str, body: &str) -> String {
    format!(
        r#"# The manifest of the package "{name}" (Project Avalanche; every key: Docs/architecture/content.md §2, §16).
# Check it with `ssx-assets mod check <folder>`, pack it with `ssx-assets mod pack <folder>`.

[resource]
name = "{name}"
version = "0.1.0"
title = "{name}"
authors = []
description = ""
api = 1
type = "mod"
priority = 0
{body}"#
    )
}

const RIGHTS: &str = "Rights\n------\nEvery sound and song a package plays must be yours to share: your own recordings and \
compositions, or material whose licence lets you redistribute it. The game's own sounds and music are EA's: a package \
may name them by id, never contain them or an export of them (`mod check` and `mod pack` refuse their bytes). A server \
that streams a package's music carries the same responsibility.\n";

fn noise(n: usize, seed: u32) -> Vec<f32> {
    let mut s = seed.max(1);
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

fn whoosh() -> Vec<u8> {
    let rate = 48_000usize;
    let n = rate * 3 / 2;
    let x = noise(n, 0x5EED);
    let (mut y, mut out) = (0.0f32, Vec::with_capacity(n));
    for (i, v) in x.into_iter().enumerate() {
        let t = i as f32 / n as f32;
        let a = 0.02 + 0.10 * (std::f32::consts::PI * t).sin();
        y += a * (v - y);
        out.push(y * 0.8);
    }
    wav_f32(&out, rate as u32, 1)
}

fn song() -> Vec<u8> {
    let rate = 44_100usize;
    let chords = [[261.63, 329.63, 392.0], [220.0, 261.63, 329.63], [174.61, 220.0, 261.63], [196.0, 246.94, 293.66]];
    let n = rate * 20;
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / rate as f32;
        let c = &chords[(i / (rate * 5)).min(3)];
        let local = (i % (rate * 5)) as f32 / rate as f32;
        let env = (local / 0.05).min(1.0) * ((5.0 - local) / 0.2).min(1.0);
        let v: f32 = c.iter().map(|f| (2.0 * std::f32::consts::PI * f * t).sin()).sum::<f32>() / 3.0 * 0.3 * env;
        out.extend_from_slice(&[v, v * 0.9]);
    }
    wav_f32(&out, rate as u32, 2)
}

pub(super) fn sound(root: &Path, name: &str) -> Result<()> {
    let body = r#"
# Replace what the game's sounds play. A sound's id is a CSIS event or function of the game's audio interfaces:
# `ssx-assets content ids --kind sound` lists them, `ssx-assets content export <id>` writes what one plays from your own
# disc (its samples by bank, module, player and mixer voice, and a sounds.toml naming them) as a starting point.
# When a sound plays stays the game's: its programs start, stop, pitch and level the voices; your files are what they
# play.
[override]
"ssx:sound/sfx/boardsfx/sfx_board" = "sounds/carve.toml"
# A speech event's line (said where the game says it, on the event's voice):
# "ssx:sound/speech/eng/pilot_and_rider_eventdef_eng/vox_character_tumble" = "sounds/my_tumble.ogg"

# Sounds of your own (`<this package>:sound/<name>`) are registered and checked; scripts and markers play them from
# phase M7.
# [add]
# "sound/horn" = "sounds/horn.ogg"
"#;
    std::fs::create_dir(root.join("sounds"))?;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::write(
        root.join("sounds/carve.toml"),
        r#"# What the board's carve voices play (the board's other voices — glides, debris — stay the game's).
# files: OGG Vorbis, FLAC or WAV (1, 2, 4 or 6 channels, 8 to 192 kHz, at most 120 s and 64 MiB decoded at 48 kHz).
files = ["sounds/carve_01.wav"]
variation = "random"            # random | sequential: the file each voice start plays
volume = 1.0                    # a factor on the game's volume (0 to 4)
pitch = [0.97, 1.03]            # a factor drawn per start, on the game's pitch (0.25 to 4)
loop = "disc"                   # "disc": loops where the game's sample loops; true | false
# The voices the set replaces, by the game's mixer voice (`content export` names each player's):
voices = ["Board_Packed_Glide_Carve_Voice", "Board_Powder_Glide_Carve_Voice", "Board_Ice_Glide_Carve_Voice", "Board_Rock_Glide_Carve_Voice"]
others = "keep"                 # keep | mute: the id's other voices
# bus = "Board_BUS"             # a bus of the game's mixer instead of the voice's own
# attenuation = "boarder_board_center"   # or 3-D through one of the game's emitters
"#,
    )?;
    std::fs::write(root.join("sounds/carve_01.wav"), whoosh())?;
    std::fs::write(root.join("README.txt"), format!("{name}: a sound package (Project Avalanche).\n\n{RIGHTS}"))?;
    Ok(())
}

pub(super) fn music(root: &Path, name: &str) -> Result<()> {
    let body = r#"
# Songs added to the licensed soundtrack (drawn as the game draws its own: weighed by the race type, never twice
# in a row out of turn); `replace = true` in the playlist plays only these. RiderNet's music menu lists them after the
# disc's songs to switch on and off (a playlist's n-th song is <your package>:music/playlist/<n> in the profile's
# `.mods.toml`; an added song's id must not be one of those).
[add]
"music/playlist" = "music/playlist.toml"

# A licensed song's audio replaced (its title and race types stay the disc's unless the song's fields give them):
# [override]
# "ssx:music/licensed/145" = "music/my_song.ogg"
# A Deadly Descent's music: its track looped or in sections (`type = "track"`), or an interactive score that follows
# the game's own Pathfinder events (`type = "interactive"`, on the project's track 0):
# "ssx:music/pathfinder/ssxdd/0" = "music/my_score.toml"
# `ssx-assets content ids --kind music` lists the ids; `ssx-assets content export <id>` writes one from your disc.
"#;
    std::fs::create_dir(root.join("music"))?;
    std::fs::write(root.join(ssx_content::manifest::MANIFEST_FILE), manifest(name, body))?;
    std::fs::write(
        root.join("music/playlist.toml"),
        r#"type = "playlist"
replace = false                  # true: the disc's licensed songs are not drawn while this package is on

[[song]]
file = "music/song_01.wav"       # OGG Vorbis, FLAC or WAV (mono or stereo, at most 30 min)
Artist = "Me"
Title = "Four chords"
RaceIt = true                    # the race types it is weighed for (1000 against 1, as the game's songs)
TrickIt = true
SurviveIt = true
gain = 1.0                       # on the licensed level
# loop = [5.0, 20.0]             # repeats from 5 s to 20 s until the level ends
# drops = ["ssx:drop/exp_alps_vorassay_drop_3"]   # only on these drops
# menu = true                    # also the front end's menu music posts (after the first ride of a run)
"#,
    )?;
    std::fs::write(root.join("music/song_01.wav"), song())?;
    std::fs::write(root.join("README.txt"), format!("{name}: a music package (Project Avalanche).\n\n{RIGHTS}"))?;
    Ok(())
}
