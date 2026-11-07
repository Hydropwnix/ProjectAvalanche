use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use ssx_content::audio::sounds::SoundClass;
use ssx_content::{ContentId, Kind};

use super::audio::{DiscMusic, DiscSounds, DiscTrack};

pub fn wav_f32(samples: &[f32], rate: u32, channels: u16) -> Vec<u8> {
    let data = (samples.len() * 4) as u32;
    let align = channels * 4;
    let mut v = Vec::with_capacity(44 + samples.len() * 4);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    v.extend_from_slice(&align.to_le_bytes());
    v.extend_from_slice(&32u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

struct WavOut {
    path: std::path::PathBuf,
    file: std::io::BufWriter<std::fs::File>,
    left: u64,
}

impl WavOut {
    fn create(path: &Path, frames: u64, rate: u32, channels: u16) -> Result<Self> {
        let data = frames * u64::from(channels) * 4;
        if data + 36 > u64::from(u32::MAX) {
            bail!("{}: {data} bytes of samples, over the WAV format's 4 GiB", path.display());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut h = wav_f32(&[], rate, channels);
        h[4..8].copy_from_slice(&((36 + data) as u32).to_le_bytes());
        h[40..44].copy_from_slice(&(data as u32).to_le_bytes());
        std::io::Write::write_all(&mut file, &h)?;
        Ok(Self { path: path.to_path_buf(), file, left: data })
    }

    fn put(&mut self, samples: &[f32]) -> Result<()> {
        let n = samples.len() as u64 * 4;
        if n > self.left {
            bail!("{}: more samples than its header states", self.path.display());
        }
        self.left -= n;
        let mut b = Vec::with_capacity(samples.len() * 4);
        for x in samples {
            b.extend_from_slice(&x.to_le_bytes());
        }
        std::io::Write::write_all(&mut self.file, &b)?;
        Ok(())
    }

    fn finish(mut self) -> Result<std::path::PathBuf> {
        if self.left != 0 {
            bail!("{}: {} bytes fewer than its header states", self.path.display(), self.left);
        }
        std::io::Write::flush(&mut self.file)?;
        Ok(self.path)
    }
}

pub fn export_to(big_root: &Path, id: &ContentId, out_dir: &Path, app_dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut written = Vec::new();
    let result = match id.kind() {
        Kind::Sound => export_sound(big_root, id, out_dir, &mut written),
        Kind::Music => export_music(big_root, id, out_dir, &mut written),
        k => Err(anyhow::anyhow!("{id}: {} is not exported by this command's audio part", k.as_str())),
    };
    let mut rows = Vec::new();
    for path in &written {
        let (size, hash) = ssx_content::hash::hash_file(path).with_context(|| format!("hashing {}", path.display()))?;
        rows.push(super::export::ExportRow { hash, size, id: id.to_string(), file: path.display().to_string() });
    }
    super::export::record_exports(app_dir, &rows)?;
    result.map(|()| written)
}

fn write_text(out_dir: &Path, rel: &str, text: &str, written: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let path = out_dir.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, text)?;
    written.push(path);
    Ok(())
}

fn header(id: &ContentId, what: &str) -> String {
    format!(
        "# {id}: {what} — an export for your own use (Docs/architecture/content.md §16).\n\
         # A package may not ship these files: `mod check` and `mod pack` refuse exported bytes. Make your own and\n\
         # name them here, then add this file to your package's [override].\n\n"
    )
}

fn export_sound(big_root: &Path, id: &ContentId, out_dir: &Path, written: &mut Vec<std::path::PathBuf>) -> Result<()> {
    match SoundClass::of(id) {
        SoundClass::Effect => {}
        SoundClass::Speech | SoundClass::Trigger => bail!(
            "{id}: a speech event's lines are drawn at run time from every rider's and pilot's bank by the sentence's \
             queries; this version exports the game's effects"
        ),
        _ => bail!("{id}: not a sound of the disc's effects"),
    }
    let name = id.path().rsplit('/').next().unwrap_or(id.path());
    let disc = DiscSounds::index(big_root)?;
    let s = disc.get(id).with_context(|| format!("{id}: no such sound on the disc"))?;
    let mut aems = crate::sfx_aems::Aems::new();
    aems.load_interfaces(big_root)?;
    for p in crate::music::find_files(big_root, "data/xenon/audio/sfx", "abk") {
        let Some(stem) = p.file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        aems.add_bank(crate::sfx_aems::Bank::load(big_root, &stem)?);
    }
    let key = aems.key_in(&s.file, &s.name).with_context(|| format!("{id}: no CSIS entry {} in {}", s.name, s.file))?;
    let voices = crate::sfx_mixer::voice_patch_table(big_root)?;
    let mut toml = header(id, &format!("the samples {} plays", s.name));
    let mut first = Vec::new();
    let mut done: std::collections::BTreeSet<(usize, u16)> = std::collections::BTreeSet::new();
    let _ = writeln!(toml, "# Its players (bank, module, player: the voice it plays on), each with its samples:");
    let players = aems.players_of(key);
    if players.is_empty() {
        bail!("{id}: no module of the disc's banks plays for {}", s.name);
    }
    for (k, p) in players.iter().enumerate() {
        let bank = &aems.banks[p.bank];
        let voice =
            p.voice_id.and_then(|v| voices.get(&v)).map_or_else(|| "the default voice".to_string(), |(_, v)| v.clone());
        let _ = writeln!(toml, "#   {} module {} player {}: {voice}", bank.name, p.module, p.player);
        for e in p.entries.iter().filter(|e| !e.is_unused()) {
            let file = format!("{name}/{}_sound{}.wav", bank.name, e.sound_index);
            if done.insert((p.bank, e.sound_index)) {
                let (samples, rate, ch) = decode_entry(bank, e)?;
                let frames = samples.len() as u64 / u64::from(ch.max(1));
                let mut w = WavOut::create(&out_dir.join(&file), frames, rate, ch)?;
                w.put(&samples)?;
                written.push(w.finish()?);
                let _ = writeln!(
                    toml,
                    "#     {file} ({ch} ch, {rate} Hz, {:.2} s)",
                    frames as f64 / f64::from(rate.max(1))
                );
            } else {
                let _ = writeln!(toml, "#     {file}");
            }
            if k == 0 {
                first.push(format!("\"sounds/{file}\""));
            }
        }
    }
    let _ = write!(
        toml,
        "\n# The set your files make (in the package's sounds/ folder); `voices` or `players` limit it to some of the\n\
         # voices above (`others = \"keep\"` keeps the rest the disc's).\nfiles = [{}]\nvariation = \"random\"\n\
         volume = 1.0\npitch = [1.0, 1.0]\n# voices = [\"Board_Packed_Glide_Carve_Voice\"]\n# others = \"keep\"\n",
        first.join(", ")
    );
    write_text(out_dir, &format!("{name}.toml"), &toml, written)
}

fn decode_entry(bank: &crate::sfx_aems::Bank, e: &ssx_formats::abk::SampleEntry) -> Result<(Vec<f32>, u32, u16)> {
    use ssx_formats::eaac::{Codec, Storage, read_v0_blocks};
    let s =
        bank.abk.s10a.sounds.get(usize::from(e.sound_index)).and_then(Option::as_ref).context("an empty S10A slot")?;
    let h = &s.header;
    let blocks = match h.storage {
        Storage::Ram => read_v0_blocks(h, &bank.data, s.offset as usize + h.size)?,
        Storage::Stream => {
            let ast = bank.ast.as_ref().context("a streamed sound without its .ast")?;
            read_v0_blocks(h, ast, e.stream_offset as usize)?
        }
        Storage::Gigasample => bail!("a gigasample sound"),
    };
    let (samples, rate, ch) = match h.codec {
        Codec::EaXma => {
            let d = ssx_formats::xma_decode::decode_sound(h, &blocks)?;
            (d.samples, d.sample_rate, d.channels)
        }
        Codec::EaLayer3V1 => {
            let d = ssx_formats::ealayer3_decode::decode_sound(h, &blocks)?;
            (d.samples, d.sample_rate, d.channels)
        }
        c => bail!("codec {c:?} not on the disc"),
    };
    Ok((samples, rate, u16::from(ch)))
}

fn export_music(big_root: &Path, id: &ContentId, out_dir: &Path, written: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let disc = DiscMusic::index(big_root)?;
    match disc.get(id).with_context(|| format!("{id}: no such music on the disc"))? {
        DiscTrack::Licensed(sample) => {
            let name = format!("licensed_{sample}");
            let mb = crate::musicbox::MusicBox::load(big_root)?;
            let song = mb.song(*sample).context("no song record")?;
            let (samples, rate, ch) = mb.decode_song(*sample)?;
            let mut w = WavOut::create(
                &out_dir.join(format!("{name}.wav")),
                samples.len() as u64 / u64::from(ch.max(1)),
                rate,
                ch,
            )?;
            w.put(&samples)?;
            written.push(w.finish()?);
            let mut toml = header(id, &format!("the licensed song {}", song.label()));
            let _ = write!(toml, "type = \"song\"\nfile = \"music/{name}.wav\"\n");
            for k in ["Artist", "Title"] {
                if let Some(v) = song.get(k) {
                    let _ = writeln!(toml, "{k} = {v:?}");
                }
            }
            for k in ["RaceIt", "TrickIt", "SurviveIt"] {
                let _ = writeln!(toml, "{k} = {}", song.flag(k));
            }
            toml.push_str("gain = 1.0\n# loop = [start_s, end_s]\n");
            write_text(out_dir, &format!("{name}.toml"), &toml, written)
        }
        DiscTrack::Pathfinder { project, track, mpf } => {
            let name = format!("{project}_{track}");
            let music = crate::music::PathfinderMusic::load(big_root, project, &[*track])?;
            let mpf = ssx_formats::mpf::MpfFile::parse(&std::fs::read(mpf)?)?;
            let heads = music.track_heads(*track)?;
            let (rate, ch) = heads.first().map_or((44_100, 2), |h| (h.2, h.3));
            if let Some(h) = heads.iter().find(|h| (h.2, h.3) != (rate, ch)) {
                bail!("{id}: its sounds differ in rate or channels ({} Hz × {} against {rate} × {ch})", h.2, h.3);
            }
            let total: u64 = heads.iter().map(|h| u64::from(h.1)).sum();
            let mut w = WavOut::create(&out_dir.join(format!("{name}.wav")), total, rate, ch)?;
            let mut at: Vec<(usize, f64, f64)> = Vec::new();
            let mut frames = 0u64;
            for (k, h) in heads.iter().enumerate() {
                let samples = music.decode_track_sound(*track, k)?;
                let n = samples.len() as u64 / u64::from(ch);
                if n != u64::from(h.1) {
                    bail!("{id}: sound {k} decodes to {n} frames, its header states {}", h.1);
                }
                w.put(&samples)?;
                at.push((h.0, frames as f64 / f64::from(rate), (frames + n) as f64 / f64::from(rate)));
                frames += n;
            }
            written.push(w.finish()?);
            let mut toml = header(id, &format!("{project} track {track}: its sounds in their order"));
            let _ = write!(toml, "type = \"track\"\nfile = \"music/{name}.wav\"\ngain = 1.0\n\n");
            let _ = writeln!(
                toml,
                "# Its sections by their first node, each here the region of its first segment in {name}.wav:"
            );
            let ram = mpf.tracks.get(*track).is_some_and(|t| t.num_subbanks > 0);
            for (n, node) in mpf.nodes.iter().enumerate() {
                if node.sample != ssx_formats::mpf::SECTION_START || node.track as usize != *track {
                    continue;
                }
                let first = mpf.nodes.iter().find(|x| x.section as usize == n && x.sample > 0);
                let region = first.and_then(|x| {
                    let record = x.sample as usize - 1;
                    let key = if ram { (mpf.samples.get(record)?.location & 0xFFFF) as usize } else { record };
                    at.iter().find(|(r, ..)| *r == key)
                });
                match region {
                    Some((_, a, b)) => {
                        let _ = writeln!(
                            toml,
                            "[[section]]\nnode = {n}\nstart = {a:.6}\nend = {b:.6}  # part {}",
                            node.part
                        );
                    }
                    None => {
                        let _ =
                            writeln!(toml, "# section node {n} (part {}): its sound is not in this track", node.part);
                    }
                }
            }
            write_text(out_dir, &format!("{name}.toml"), &toml, written)
        }
    }
}
