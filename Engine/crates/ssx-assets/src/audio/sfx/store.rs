use anyhow::{Result, bail, ensure};

use super::settings::{AudioSettings, ENTRIES};

pub const MAGIC: u32 = 0x4155_5358;
pub const VERSION: u32 = 1;
pub const IN_GAME_MUSIC: usize = 50;
pub const CUSTOM_MUSIC: usize = 20;
pub const CUSTOM_MUSIC_BYTES: usize = 0xB0;
pub const SPEECH_REPETITION_BYTES: usize = 0x2000;
pub const MUSIC_REPETITION_BYTES: usize = 0x400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioStore {
    pub version: u32,
    pub fe: Vec<i32>,
    pub in_game_music: Vec<(u32, u32)>,
    pub custom_music: Vec<Vec<u8>>,
    pub speech_repetition: Vec<u8>,
    pub music_repetition: Vec<u8>,
}

fn be(d: &[u8], o: usize) -> Result<u32> {
    match d.get(o..o + 4) {
        Some(b) => Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        None => bail!("audio store: past the end at {o:#x}"),
    }
}

impl AudioStore {
    pub fn size_for(fe: usize, in_game_music: usize, custom_music: usize) -> usize {
        0xC + (fe + 2) * 4
            + (in_game_music + 1) * 8
            + 8
            + custom_music * CUSTOM_MUSIC_BYTES
            + 4
            + SPEECH_REPETITION_BYTES
            + 4
            + MUSIC_REPETITION_BYTES
    }

    pub const fn game_size() -> usize {
        0xC + (ENTRIES + 2) * 4
            + (IN_GAME_MUSIC + 1) * 8
            + 8
            + CUSTOM_MUSIC * CUSTOM_MUSIC_BYTES
            + 4
            + SPEECH_REPETITION_BYTES
            + 4
            + MUSIC_REPETITION_BYTES
    }

    pub fn new(fe: &AudioSettings, songs: &[(u32, u32)]) -> Self {
        let mut in_game_music = vec![(0, 0); IN_GAME_MUSIC];
        for (slot, s) in in_game_music.iter_mut().zip(songs) {
            *slot = *s;
        }
        Self {
            version: VERSION,
            fe: fe.0.to_vec(),
            in_game_music,
            custom_music: vec![vec![0; CUSTOM_MUSIC_BYTES]; CUSTOM_MUSIC],
            speech_repetition: vec![0; SPEECH_REPETITION_BYTES],
            music_repetition: vec![0; MUSIC_REPETITION_BYTES],
        }
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(be(data, 0)? == MAGIC, "audio store: no 'AUSX'");
        let version = be(data, 4)?;
        ensure!(be(data, 8)? as usize == data.len(), "audio store: total {} for {} bytes", be(data, 8)?, data.len());
        let mut o = 0xC;
        let mut section = |want: &dyn Fn(usize) -> usize, counted: bool| -> Result<(usize, usize, usize)> {
            let size = be(data, o)? as usize;
            let n = if counted { be(data, o + 4)? as usize } else { 0 };
            ensure!(size == want(n) && o + size <= data.len(), "audio store: section at {o:#x} of size {size}");
            let at = o + if counted { 8 } else { 4 };
            o += size;
            Ok((at, n, size))
        };
        let (at, n, _) = section(&|n| (n + 2) * 4, true)?;
        let fe = (0..n).map(|i| be(data, at + 4 * i).map(|v| v as i32)).collect::<Result<_>>()?;
        let (at, n, _) = section(&|n| (n + 1) * 8, true)?;
        let in_game_music =
            (0..n).map(|i| Ok((be(data, at + 8 * i)?, be(data, at + 8 * i + 4)?))).collect::<Result<_>>()?;
        let (at, n, _) = section(&|n| 8 + n * CUSTOM_MUSIC_BYTES, true)?;
        let custom_music =
            (0..n).map(|i| data[at + i * CUSTOM_MUSIC_BYTES..at + (i + 1) * CUSTOM_MUSIC_BYTES].to_vec()).collect();
        let (at, _, size) = section(&|_| 4 + SPEECH_REPETITION_BYTES, false)?;
        let speech_repetition = data[at..at + size - 4].to_vec();
        let (at, _, size) = section(&|_| 4 + MUSIC_REPETITION_BYTES, false)?;
        let music_repetition = data[at..at + size - 4].to_vec();
        ensure!(o == data.len(), "audio store: {} bytes after the sections", data.len() - o);
        Ok(Self { version, fe, in_game_music, custom_music, speech_repetition, music_repetition })
    }

    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::game_size());
        let put = |out: &mut Vec<u8>, v: u32| out.extend(v.to_be_bytes());
        put(&mut out, MAGIC);
        put(&mut out, self.version);
        put(&mut out, 0);
        put(&mut out, ((self.fe.len() + 2) * 4) as u32);
        put(&mut out, self.fe.len() as u32);
        self.fe.iter().for_each(|&v| put(&mut out, v as u32));
        put(&mut out, ((self.in_game_music.len() + 1) * 8) as u32);
        put(&mut out, self.in_game_music.len() as u32);
        for &(a, b) in &self.in_game_music {
            put(&mut out, a);
            put(&mut out, b);
        }
        put(&mut out, (8 + self.custom_music.len() * CUSTOM_MUSIC_BYTES) as u32);
        put(&mut out, self.custom_music.len() as u32);
        for c in &self.custom_music {
            let mut r = c.clone();
            r.resize(CUSTOM_MUSIC_BYTES, 0);
            out.extend(r);
        }
        put(&mut out, (4 + self.speech_repetition.len()) as u32);
        out.extend(&self.speech_repetition);
        put(&mut out, (4 + self.music_repetition.len()) as u32);
        out.extend(&self.music_repetition);
        let total = out.len() as u32;
        out[8..12].copy_from_slice(&total.to_be_bytes());
        out
    }

    pub fn settings(&self) -> Option<AudioSettings> {
        if self.version != VERSION || self.fe.len() != ENTRIES {
            return None;
        }
        Some(AudioSettings(std::array::from_fn(|i| self.fe[i])))
    }

    pub fn set_settings(&mut self, s: &AudioSettings) {
        self.fe = s.0.to_vec();
    }
}

pub fn in_game_music(songs: &[crate::musicbox::Song]) -> Vec<(u32, u32)> {
    let disc = |s: &&crate::musicbox::Song| s.id < crate::musicbox::FIRST_PACKAGE_SONG;
    let mut v: Vec<&crate::musicbox::Song> = songs.iter().filter(disc).filter(|s| s.get("Index").is_some()).collect();
    v.sort_by(|a, b| a.get("Index").cmp(&b.get("Index")));
    v.iter().take(IN_GAME_MUSIC).map(|s| (s.id, u32::from(s.unk[2] != 0))).collect()
}

pub fn load_in_game_music(big_root: &std::path::Path) -> Result<Vec<(u32, u32)>> {
    use anyhow::Context;
    let rel = "data/xenon/audio/music/music_musicboxsonginfo.bin";
    let path = crate::music::find_file(big_root, rel).with_context(|| format!("{rel} not found"))?;
    Ok(in_game_music(&crate::musicbox::parse_song_info(&std::fs::read(path)?)?))
}

impl AudioStore {
    // TODO(guess): `SpeechRepetition` / `MusicRepetition` (refreshed before a save, 0x830C1CE8 → 0x830B4E88 vt+0x18) are kept as loaded — the port does not serialize its repetition pools; settle: the two blobs' writers (0x830B4E88's vt+0x18, the music pools' owner).
    pub fn with_session(&self, fe: &AudioSettings, songs: &[(u32, u32)]) -> Self {
        let mut st = self.clone();
        st.set_settings(fe);
        let mut igm = vec![(0, 0); self.in_game_music.len().max(IN_GAME_MUSIC)];
        for (slot, &(id, on)) in igm.iter_mut().zip(songs) {
            let kept = self.in_game_music.iter().find(|e| e.0 == id && id != 0).map_or(on, |e| e.1);
            *slot = (id, kept);
        }
        st.in_game_music = igm;
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trip_and_size() {
        let s = AudioSettings([1, 10, 10, 10, 10, 10, 10, 0x0032_0050, 0, 1, 1]);
        let mut st = AudioStore::new(&s, &[(0xA9, 1), (0x12, 1)]);
        st.speech_repetition[5] = 7;
        let w = st.write();
        assert_eq!(w.len(), AudioStore::game_size());
        assert_eq!(w.len(), 13_224);
        assert_eq!(&w[..4], b"AUSX");
        assert_eq!(u32::from_be_bytes(w[8..12].try_into().unwrap()) as usize, w.len());
        let back = AudioStore::parse(&w).unwrap();
        assert_eq!(back, st);
        assert_eq!(back.write(), w);
        assert_eq!(back.settings(), Some(s));
        let mut v2 = back.clone();
        v2.version = 2;
        assert_eq!(v2.settings(), None);
        assert!(AudioStore::parse(&w[..w.len() - 1]).is_err());
    }

    #[test]
    fn session_keeps_the_song_flags_by_key() {
        let s = AudioSettings([1; ENTRIES]);
        let mut st = AudioStore::new(&s, &[(5, 1), (9, 1)]);
        st.in_game_music[1].1 = 0;
        let next = st.with_session(&AudioSettings([2; ENTRIES]), &[(9, 1), (5, 1), (7, 1)]);
        assert_eq!(&next.in_game_music[..4], &[(9, 0), (5, 1), (7, 1), (0, 0)]);
        assert_eq!(next.fe, vec![2; ENTRIES]);
    }
}
