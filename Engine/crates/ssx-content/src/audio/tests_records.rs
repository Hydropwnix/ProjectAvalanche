use super::music::{self, MusicDef, TrackAudio};
use super::music_score::When;
use super::sounds::{self, Looping, Others, PlayerRef, Route, SoundClass, Variation};
use crate::id::ContentId;

fn id(s: &str) -> ContentId {
    s.parse().unwrap()
}

fn problems(r: Result<impl std::fmt::Debug, Vec<crate::material::Problem>>) -> Vec<(usize, String)> {
    match r {
        Ok(v) => panic!("read as {v:?}"),
        Err(ps) => ps.into_iter().map(|(line, _, m)| (line, m)).collect(),
    }
}

fn has(ps: &[(usize, String)], line: usize, text: &str) -> bool {
    ps.iter().any(|(l, m)| *l == line && m.contains(text))
}

#[test]
fn an_effect_record_reads_as_written() {
    let text = br#"files = ["sounds/a.ogg", "sounds/b.flac"]
variation = "random"
volume = 0.5
pitch = [0.9, 1.1]
loop = true
attenuation = "boarder_board_center"
players = [{ bank = "board_packed", module = 3, player = 5 }, { bank = "board_ice", player = 1 }]
others = "mute"
"#;
    let d = sounds::parse_toml(text, SoundClass::Effect).unwrap();
    assert_eq!(d.files, ["sounds/a.ogg", "sounds/b.flac"]);
    assert_eq!((d.variation, d.volume, d.pitch, d.looping), (Variation::Random, 0.5, (0.9, 1.1), Looping::On));
    assert_eq!(d.route, Route::Emitter("boarder_board_center".into()));
    let p = |bank: &str, module, player| PlayerRef { bank: bank.into(), module, player };
    assert_eq!(d.players, Some(vec![p("board_packed", Some(3), 5), p("board_ice", None, 1)]));
    assert_eq!(d.others, Others::Mute);
    let d = sounds::parse_toml(b"files = [\"a.wav\"]\nbus = \"Board_BUS\"\nvoices = [\"V_1\"]\n", SoundClass::Effect);
    let d = d.unwrap();
    assert_eq!(
        (d.route, d.voices, d.looping),
        (Route::Bus("Board_BUS".into()), Some(vec!["V_1".into()]), Looping::Disc)
    );
}

#[test]
fn an_effect_record_refuses_each_bad_field_at_its_line() {
    let text = br#"files = ["../up.ogg", "x.mp3"]
variation = "shuffle"
volume = 9.0
pitch = [2.0, 1.0]
loop = "sometimes"
bus = "Board BUS"
attenuation = "x"
players = [{ bank = "board_packed", module = 300, player = -1 }]
voices = ["V"]
others = "drop"
"#;
    let ps = problems(sounds::parse_toml(text, SoundClass::Effect));
    for (line, text) in [
        (1, "x.mp3"),
        (1, ".."),
        (2, "random or sequential"),
        (3, "volume: from 0 to 4"),
        (4, "pitch: [low, high]"),
        (5, "loop: true, false"),
        (7, "not both"),
        (8, "module: an index"),
        (8, "player: an index"),
        (9, "not both"),
        (10, "keep or mute"),
    ] {
        assert!(has(&ps, line, text), "line {line} {text:?} in {ps:?}");
    }
    let ps = problems(sounds::parse_toml(b"files = []\nothers = \"keep\"\n", SoundClass::Effect));
    assert!(has(&ps, 1, "files: 1 to 256") && has(&ps, 2, "there is neither"), "{ps:?}");
    let ps = problems(sounds::parse_toml(b"files = [\"a.wav\"]\nspeed = 2\n", SoundClass::Effect));
    assert!(has(&ps, 2, "unknown field"), "{ps:?}");
}

#[test]
fn speech_and_added_sounds_take_no_voice_fields() {
    let speech = id("ssx:sound/speech/eng/pilot_and_rider_eventdef_eng/vox_character_tumble");
    assert_eq!(SoundClass::of(&speech), SoundClass::Speech);
    assert_eq!(SoundClass::of(&id("mymod:sound/horn")), SoundClass::Added);
    assert_eq!(SoundClass::of(&id("ssx:sound/sfx/boardsfx/sfx_board")), SoundClass::Effect);
    let text = b"files = [\"a.ogg\"]\nbus = \"SFX_BUS\"\nvoices = [\"V\"]\nloop = false\n";
    let ps = problems(sounds::parse_toml(text, SoundClass::Speech));
    for line in [2, 3, 4] {
        assert!(has(&ps, line, "only for the game's effects; a speech line"), "{line} {ps:?}");
    }
    let ps = problems(sounds::parse_toml(b"files = [\"a.ogg\"]\nbus = \"SFX_BUS\"\n", SoundClass::Added));
    assert!(has(&ps, 2, "phase M7"), "{ps:?}");
    let ok =
        sounds::parse_toml(b"files = [\"a.ogg\", \"b.ogg\"]\nvolume = 2.0\npitch = [1, 1.2]\n", SoundClass::Speech);
    assert_eq!(ok.unwrap().volume, 2.0);
}

#[test]
fn a_playlist_and_a_song_read_as_written() {
    let text = br#"type = "playlist"
replace = true
[[song]]
file = "music/a.ogg"
Artist = "Me"
Title = "One"
TrickIt = true
RaceIt = false
loop = [5.0, 20.0]
drops = ["ssx:drop/exp_alps_vorassay_drop_3"]
menu = true
gain = 0.8
[[song]]
file = "music/b.flac"
"#;
    let MusicDef::Playlist { replace, songs } = music::parse_toml(text, None).unwrap() else { panic!() };
    assert!(replace);
    assert_eq!(songs.len(), 2);
    let a = &songs[0];
    assert_eq!((a.artist.as_deref(), a.title.as_deref()), (Some("Me"), Some("One")));
    assert_eq!((a.race_it, a.trick_it, a.survive_it, a.menu, a.gain), (Some(false), Some(true), None, true, 0.8));
    assert_eq!(a.looping.map(|l| (l.start, l.end)), Some((5.0, 20.0)));
    assert_eq!(a.drops, Some(vec![id("ssx:drop/exp_alps_vorassay_drop_3")]));
    assert_eq!((songs[1].file.as_str(), songs[1].gain, songs[1].looping), ("music/b.flac", 1.0, None));
    let over = id("ssx:music/licensed/145");
    let MusicDef::Song(s) = music::parse_toml(b"type = \"song\"\nfile = \"x.wav\"\n", Some(&over)).unwrap() else {
        panic!()
    };
    assert_eq!((s.file.as_str(), s.title), ("x.wav", None));
}

#[test]
fn a_music_record_is_the_type_its_id_takes() {
    let licensed = id("ssx:music/licensed/145");
    let track = id("ssx:music/pathfinder/ssxdd/0");
    let cases: [(&[u8], Option<&ContentId>, &str); 4] = [
        (b"type = \"track\"\nfile = \"a.ogg\"\n", None, "an added music entry is a playlist or a song"),
        (b"type = \"interactive\"\n", Some(&licensed), "a licensed song's override is a song"),
        (b"type = \"playlist\"\n", Some(&track), "a track or an interactive score"),
        (b"file = \"a.ogg\"\n", None, "type: playlist, song, track or interactive"),
    ];
    for (text, over, want) in cases {
        let ps = problems(music::parse_toml(text, over));
        assert!(ps.iter().any(|(_, m)| m.contains(want)), "{want:?} in {ps:?}");
    }
    let text = br#"type = "playlist"
[[song]]
file = "a.mp3"
Title = ""
gain = 7.0
loop = [9.0, 3.0]
drops = ["ssx:sound/x/y", "nonsense"]
"#;
    let ps = problems(music::parse_toml(text, None));
    for (line, text) in
        [(3, "not .ogg, .flac or .wav"), (4, "Title: 1 to 256"), (5, "gain"), (6, "loop"), (7, "a drop id")]
    {
        assert!(has(&ps, line, text), "line {line} {text:?} in {ps:?}");
    }
}

#[test]
fn a_track_is_looped_or_in_sections() {
    let track = id("ssx:music/pathfinder/ssxdd/0");
    let looped =
        music::parse_toml(b"type = \"track\"\nfile = \"t.ogg\"\nloop = [1.0, 9.0]\ngain = 0.5\n", Some(&track));
    let MusicDef::Track { audio: TrackAudio::Looped { file, region }, gain } = looped.unwrap() else { panic!() };
    assert_eq!((file.as_str(), region.map(|r| (r.start, r.end)), gain), ("t.ogg", Some((1.0, 9.0)), 0.5));
    let text = b"type = \"track\"\nfile = \"t.ogg\"\n[[section]]\nnode = 4\nstart = 0.0\nend = 8.0\n[[section]]\nnode = 9\nstart = 8.0\nend = 16.0\n";
    let MusicDef::Track { audio: TrackAudio::Sectioned { sections, .. }, .. } =
        music::parse_toml(text, Some(&track)).unwrap()
    else {
        panic!()
    };
    assert_eq!(sections.iter().map(|(n, r)| (*n, r.start, r.end)).collect::<Vec<_>>(), [(4, 0.0, 8.0), (9, 8.0, 16.0)]);
    let both =
        b"type = \"track\"\nfile = \"t.ogg\"\nloop = [0.0, 1.0]\n[[section]]\nnode = 4\nstart = 0.0\nend = 8.0\n";
    let ps = problems(music::parse_toml(both, Some(&track)));
    assert!(ps.iter().any(|(_, m)| m.contains("not both")), "{ps:?}");
    let text = b"type = \"track\"\nfile = \"t.ogg\"\n[[section]]\nnode = 4\nstart = 0.0\nend = 8.0\n[[section]]\nnode = 4\nstart = 3.0\nend = 2.0\n[[section]]\nnode = 70000\nstart = 0.0\nend = 1.0\n";
    let ps = problems(music::parse_toml(text, Some(&track)));
    for (line, text) in [(8, "given twice"), (8, "below end"), (12, "from 0 to 65535")] {
        assert!(has(&ps, line, text), "line {line} {text:?} in {ps:?}");
    }
}

const SCORE: &[u8] = br#"type = "interactive"
bpm = 120
beats_per_bar = 4
start = "intro"
[[layer]]
name = "drums"
level = 0.0
[[section]]
name = "intro"
file = "music/intro.wav"
[[section]]
name = "main"
file = "music/main.ogg"
layers.drums = "music/main_drums.wav"
[[section]]
name = "win"
file = "music/win.wav"
repeat = false
[[stinger]]
name = "tricky"
file = "music/tricky.wav"
[events]
Music_Start = { section = "main", when = "bar" }
Tricky_B = { section = "win", hold = true }
Music_Return = { section = "@held" }
Tricky_A = { stinger = "tricky" }
0x1234abcd = { pitch = 0.0, fade = 2.0, stop = 2.0 }
[control]
50 = { layers = { drums = 1.0 }, fade = 0.5 }
"#;

#[test]
fn an_interactive_score_reads_as_written() {
    let track = id("ssx:music/pathfinder/ssxdd/0");
    let MusicDef::Interactive(s) = music::parse_toml(SCORE, Some(&track)).unwrap() else { panic!() };
    assert_eq!((s.bpm, s.beats_per_bar, s.start.as_deref()), (Some(120.0), 4, Some("intro")));
    assert_eq!(s.sections.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["intro", "main", "win"]);
    assert_eq!(s.sections[1].layers.get("drums").map(String::as_str), Some("music/main_drums.wav"));
    assert!(s.sections[0].repeat && !s.sections[2].repeat);
    assert_eq!(s.layers.get("drums"), Some(&0.0));
    assert_eq!(s.events["Music_Start"].section, Some(("main".into(), When::Bar)));
    assert!(s.events["Tricky_B"].hold);
    assert_eq!(s.events["Tricky_A"].stinger.as_deref(), Some("tricky"));
    let stop = &s.events["0x1234abcd"];
    assert_eq!((stop.pitch, stop.fade, stop.stop), (Some(0.0), 2.0, Some(2.0)));
    assert_eq!(s.control[&50].layers.get("drums"), Some(&1.0));
    let mut files = s.files();
    files.sort_unstable();
    let want = ["music/intro.wav", "music/main.ogg", "music/main_drums.wav", "music/tricky.wav", "music/win.wav"];
    assert_eq!(files, want);
}

#[test]
fn an_interactive_score_refuses_what_it_cannot_play() {
    let track = id("ssx:music/pathfinder/ssxdd/0");
    let text = br#"type = "interactive"
start = "nowhere"
[[layer]]
name = "Drums"
[[section]]
name = "main"
file = "music/main.ogg"
layers.bass = "music/bass.ogg"
next = "main"
[events]
Music_Start = { section = "main", when = "bar" }
Bad-Name = { section = "main" }
Go = { section = "elsewhere", when = "later" }
Hit = { stinger = "none", layers = { drums = 2.0 }, stop = 99.0 }
[control]
200 = { fade = 0.5 }
"#;
    let ps = problems(music::parse_toml(text, Some(&track)));
    for (line, text) in [
        (2, "start: no section"),
        (4, "a new name of [a-z0-9_]"),
        (8, "layer \"bass\" is not declared"),
        (11, "needs the score's bpm"),
        (12, "a name of the project's events"),
        (13, "no section \"elsewhere\""),
        (13, "now, bar or end"),
        (14, "stinger: no stinger"),
        (14, "seconds from 0 to 60"),
    ] {
        assert!(has(&ps, line, text), "line {line} {text:?} in {ps:?}");
    }
    assert!(ps.iter().any(|(_, m)| m.contains("`next` is for a section with `repeat = false`")), "{ps:?}");
    assert!(ps.iter().any(|(_, m)| m.contains("a control level from 0 to 127")), "{ps:?}");
}
