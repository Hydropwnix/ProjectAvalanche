use anyhow::Result;

use super::cache::{self, Cache};
use super::disc;
use super::image::Picture;
use super::{Loaded, Request, Slot, Source, built_in, fitted, resolve, user_file};

const MOVIE_SLOTS: [Slot; 4] = [Slot::Background, Slot::TileMods, Slot::TileSettings, Slot::TileExtras];

struct Sink<'a> {
    emit: &'a mut dyn FnMut(Loaded),
    cache: Option<Cache>,
    problems: Vec<String>,
}

impl Sink<'_> {
    fn send(&mut self, slot: Slot, source: Source, picture: Picture, note: String) {
        (self.emit)(Loaded { slot, source, picture, note });
    }

    fn decoded(&mut self, slot: Slot, result: Result<(Picture, String)>) -> bool {
        match result {
            Ok((picture, note)) => {
                let picture = fitted(slot, picture);
                if let Some(Err(e)) = self.cache.as_mut().map(|c| c.store(slot, &picture, &note)) {
                    self.problems.push(format!("caching {}: {e:#}", slot.name()));
                }
                self.send(slot, Source::Disc, picture, note);
                true
            }
            Err(e) => {
                self.problems.push(format!("{}: {e:#}", slot.name()));
                false
            }
        }
    }
}

pub fn load(request: &Request, emit: &mut dyn FnMut(Loaded)) -> Vec<String> {
    let cache = request.game_data.as_ref().map(|data| Cache::open(&request.app_dir, &cache::data_key(data)));
    let mut sink = Sink { emit, cache, problems: Vec::new() };
    let mut pending = Vec::new();
    for slot in Slot::ALL {
        let path = user_file(&request.app_dir, slot);
        match path.is_file().then(|| Picture::read_png(&path)) {
            Some(Ok(p)) => sink.send(slot, Source::User, fitted(slot, p), path.display().to_string()),
            Some(Err(e)) => {
                sink.problems.push(format!("{}: {e:#}", path.display()));
                pending.push(slot);
            }
            None => pending.push(slot),
        }
    }
    pending.retain(|&slot| match sink.cache.as_ref().and_then(|c| c.load(slot)) {
        Some((picture, note)) => {
            sink.send(slot, Source::Disc, picture, note);
            false
        }
        None => true,
    });
    let big = request.game_data.as_deref().map(ssx_assets::setup::big_root);
    if let Some(big) = &big {
        pending.retain(|&slot| match slot {
            Slot::Logo => !sink.decoded(slot, disc::logo(big)),
            Slot::Hero => !sink.decoded(slot, disc::hero(big)),
            _ => true,
        });
    }
    for &slot in &pending {
        if resolve(slot, false, false) == Some(Source::BuiltIn)
            && let Some(picture) = built_in(slot)
        {
            sink.send(slot, Source::BuiltIn, picture, "built-in scene".into());
        }
    }
    let wanted: Vec<Slot> = pending.iter().copied().filter(|s| MOVIE_SLOTS.contains(s)).collect();
    let Some(data) = request.game_data.as_deref().filter(|_| !wanted.is_empty()) else { return sink.problems };
    let movie = disc::movie_file(data);
    if !movie.is_file() {
        sink.problems.push(format!("no intro movie ({}): built-in pictures in its place", movie.display()));
        return sink.problems;
    }
    match disc::movie_shots(&movie, |s| wanted.contains(&s)) {
        Ok(shots) => shots.into_iter().for_each(|(slot, picture, note)| {
            sink.decoded(slot, Ok((picture, note)));
        }),
        Err(e) => sink.problems.push(format!("intro movie: {e:#}")),
    }
    sink.problems
}
