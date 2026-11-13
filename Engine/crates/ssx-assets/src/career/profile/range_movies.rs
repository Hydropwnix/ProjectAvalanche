use ssx_content::ContentId;
use ssx_content::profile_state::StateValue;

use super::Profile;
use crate::port_options::RangeMovies;

pub const FIELD: &str = "range_movie_played";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeMovieOutcome {
    EveryTime,
    FirstTime,
    PlayedBefore,
    NotRecorded(String),
}

impl RangeMovieOutcome {
    pub fn plays(&self) -> bool {
        !matches!(self, RangeMovieOutcome::PlayedBefore)
    }
}

fn entry(movie: &str) -> Result<ContentId, String> {
    crate::content::fmv::movie_id(movie)
        .ok_or_else(|| format!("{} is not a movie id", ssx_core::text::quoted_max(movie, 64)))
}

impl Profile {
    pub fn range_movie_played(&self, movie: &str) -> Result<bool, String> {
        let id = entry(movie)?;
        match self.mod_state.get(&id, FIELD) {
            None => Ok(false),
            Some(StateValue::Bool(b)) => Ok(*b),
            Some(_) => Err(format!("{id}.{FIELD} is not a boolean")),
        }
    }

    pub fn mark_range_movie_played(&mut self, movie: &str) -> Result<(), String> {
        let id = entry(movie)?;
        if self.mod_state.set(&id, FIELD, StateValue::Bool(true)) {
            Ok(())
        } else {
            Err(format!("{id}.{FIELD} is not kept: the profile's state beside the save is full"))
        }
    }

    pub fn range_movie(&mut self, option: RangeMovies, movie: &str) -> RangeMovieOutcome {
        if option == RangeMovies::EveryTime {
            return RangeMovieOutcome::EveryTime;
        }
        match self.range_movie_played(movie) {
            Ok(true) => RangeMovieOutcome::PlayedBefore,
            Ok(false) => match self.mark_range_movie_played(movie) {
                Ok(()) => RangeMovieOutcome::FirstTime,
                Err(why) => RangeMovieOutcome::NotRecorded(why),
            },
            Err(why) => {
                let kept = self.mark_range_movie_played(movie).err().map_or(String::new(), |e| format!("; {e}"));
                RangeMovieOutcome::NotRecorded(format!("{why}{kept}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ssx_content::profile_state::{MAX_ENTRIES, MAX_FIELDS, ModState};

    use super::*;

    fn id(s: &str) -> ContentId {
        s.parse().unwrap()
    }

    #[test]
    fn a_played_movie_is_kept_beside_the_save() {
        let mut p = Profile::new();
        assert_eq!(p.range_movie_played("rockies"), Ok(false));
        p.mark_range_movie_played("rockies").unwrap();
        assert_eq!(p.range_movie_played("rockies"), Ok(true));
        assert_eq!(p.range_movie_played("alps"), Ok(false), "another range's movie");
        let text = p.state_to_write().write();
        assert!(text.contains("[\"ssx:movie/rockies\"]\nrange_movie_played = true"), "{text}");
        let back = Profile { mod_state: ModState::parse(text.as_bytes()).unwrap(), ..Profile::new() };
        assert_eq!((back.range_movie_played("rockies"), back.range_movie_played("alps")), (Ok(true), Ok(false)));
        assert!(p.disc_view().mod_state.entries.is_empty(), "the console layout never holds it");
        p.mark_range_movie_played("pack:movie/range/japan").unwrap();
        assert_eq!(p.mod_state.bool(&id("pack:movie/range/japan"), FIELD), Some(true));
    }

    #[test]
    fn a_hostile_record_is_reported_and_not_taken() {
        let text = b"format = 1\n[\"ssx:movie/rockies\"]\nrange_movie_played = 1\nunknown = \"kept\"\n\
                     [\"ssx:movie/alps\"]\nrange_movie_played = \"yes\"\n\
                     [\"ssx:movie/alaska\"]\nrange_movie_played = false\n";
        let mut p = Profile { mod_state: ModState::parse(text).unwrap(), ..Profile::new() };
        assert!(p.range_movie_played("rockies").unwrap_err().contains("not a boolean"));
        assert!(p.range_movie_played("alps").is_err());
        assert_eq!(p.range_movie_played("alaska"), Ok(false));
        p.mark_range_movie_played("rockies").unwrap();
        assert_eq!(p.range_movie_played("rockies"), Ok(true));
        assert_eq!(p.mod_state.text(&id("ssx:movie/rockies"), "unknown"), Some("kept"));
        for bad in ["", "a:b", "Rockies Movie", "pack:texture/x"] {
            assert!(p.range_movie_played(bad).is_err() && p.mark_range_movie_played(bad).is_err(), "{bad:?}");
        }
        let mut full = Profile::new();
        for i in 0..MAX_ENTRIES {
            assert!(full.mod_state.set(&id(&format!("ssx:movie/m{i}")), "x", StateValue::Int(0)));
        }
        assert!(full.mark_range_movie_played("rockies").unwrap_err().contains("full"));
        assert_eq!(full.range_movie_played("rockies"), Ok(false));
        let mut fields = Profile::new();
        for i in 0..MAX_FIELDS {
            assert!(fields.mod_state.set(&id("ssx:movie/rockies"), &format!("f{i}"), StateValue::Int(0)));
        }
        assert!(fields.mark_range_movie_played("rockies").is_err());
    }

    #[test]
    fn first_time_only_posts_each_ranges_movie_once_per_profile() {
        let mut p = Profile::new();
        let posts = |p: &mut Profile, option, movie| (0..3).filter(|_| p.range_movie(option, movie).plays()).count();
        assert_eq!(posts(&mut p, RangeMovies::EveryTime, "rockies"), 3);
        assert!(p.mod_state.entries.is_empty(), "the game's behaviour records nothing");
        assert_eq!(p.range_movie(RangeMovies::FirstTimeOnly, "rockies"), RangeMovieOutcome::FirstTime);
        assert_eq!(p.range_movie(RangeMovies::FirstTimeOnly, "rockies"), RangeMovieOutcome::PlayedBefore);
        assert_eq!(posts(&mut p, RangeMovies::FirstTimeOnly, "rockies"), 0);
        assert_eq!(posts(&mut p, RangeMovies::FirstTimeOnly, "alps"), 1, "another range: its own first time");
        let back = ModState::parse(p.state_to_write().write().as_bytes()).unwrap();
        let mut again = Profile { mod_state: back, ..Profile::new() };
        assert_eq!(posts(&mut again, RangeMovies::FirstTimeOnly, "rockies"), 0);
        assert_eq!(posts(&mut again, RangeMovies::FirstTimeOnly, "alaska"), 1);
        assert_eq!(posts(&mut again, RangeMovies::EveryTime, "rockies"), 3, "switched back: the game's again");
        assert_eq!(posts(&mut Profile::new(), RangeMovies::FirstTimeOnly, "rockies"), 1);
        let text = b"format = 1\n[\"ssx:movie/rockies\"]\nrange_movie_played = 7\n";
        let mut t = Profile { mod_state: ModState::parse(text).unwrap(), ..Profile::new() };
        let first = t.range_movie(RangeMovies::FirstTimeOnly, "rockies");
        assert!(matches!(&first, RangeMovieOutcome::NotRecorded(w) if w.contains("not a boolean")) && first.plays());
        assert_eq!(t.range_movie(RangeMovies::FirstTimeOnly, "rockies"), RangeMovieOutcome::PlayedBefore);
        let mut n = Profile::new();
        assert_eq!(posts(&mut n, RangeMovies::FirstTimeOnly, "Not A Movie"), 3);
        assert!(n.mod_state.entries.is_empty());
    }
}
