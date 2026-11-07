use std::collections::BTreeMap;

use super::{Checked, Severity};
use crate::id::ContentId;
use crate::manifest::MANIFEST_FILE;
use crate::movie::{self, Limits, Walked};
use crate::path::PackageTree;

#[derive(Default)]
pub struct MovieFiles {
    pub movies: BTreeMap<ContentId, (String, Walked)>,
}

pub(super) fn check_entry(c: &mut Checked, tree: &PackageTree, id: &ContentId, f: &str, at: (usize, usize)) {
    if !f.to_ascii_lowercase().ends_with(".webm") {
        c.push(
            Severity::Error,
            MANIFEST_FILE,
            at,
            format!("{id}: {f}: a movie is WebM (.webm: AV1 video, Vorbis audio)"),
        );
        return;
    }
    let limits = Limits::default();
    let walked = tree
        .open_read(f, limits.max_file_bytes)
        .map_err(|e| e.to_string())
        .and_then(|(file, len)| movie::walk(file, len, &limits).map_err(|e| e.to_string()));
    match walked {
        Ok(w) => {
            let i = &w.info;
            let audio = i.audio.as_ref().map_or("no sound".to_string(), |a| {
                format!("Vorbis, {} channels at {} Hz, {} packets", a.channels, a.rate, w.audio_packets)
            });
            let message = format!(
                "{id}: WebM: AV1 {} × {}, {:.3} s, {} pictures ({} key frames; the largest unit {} bytes); {audio}",
                i.video.width,
                i.video.height,
                i.duration_seconds(),
                w.pictures,
                w.keyframes,
                w.largest_video_block
            );
            c.push(Severity::Note, f, (0, 0), message);
            c.movies.movies.insert(id.clone(), (f.to_string(), w));
        }
        Err(e) => c.push(Severity::Error, MANIFEST_FILE, at, format!("{id}: {f}: {e}")),
    }
}
