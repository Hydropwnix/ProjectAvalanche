use std::collections::BTreeMap;

use crate::id::{BASE_NAMESPACE, ContentId, Kind};
use crate::profile_state::{ModState, StateValue};

use super::music::MusicDef;

pub const FIELD: &str = "on";

pub fn song_keys(id: &ContentId, def: &MusicDef) -> Result<Vec<ContentId>, String> {
    if !added(id) {
        return Ok(Vec::new());
    }
    match def {
        MusicDef::Playlist { songs, .. } => (1..=songs.len()).map(|n| song_key(id, n)).collect(),
        MusicDef::Song(_) => Ok(vec![id.clone()]),
        MusicDef::Track { .. } | MusicDef::Interactive(_) => Ok(Vec::new()),
    }
}

pub fn added(id: &ContentId) -> bool {
    id.kind() == Kind::Music && id.namespace() != BASE_NAMESPACE
}

pub fn song_key(id: &ContentId, n: usize) -> Result<ContentId, String> {
    ContentId::new(id.namespace(), Kind::Music, &format!("{}/{n}", id.path()))
        .map_err(|e| format!("{id}: its song {n}'s switch id {id}/{n} is not an id ({e}); a playlist's id is shorter"))
}

pub fn playlist_song_of(id: &ContentId, playlists: &BTreeMap<ContentId, usize>) -> Option<(ContentId, usize)> {
    let (parent, last) = id.path().rsplit_once('/')?;
    let canonical = !last.is_empty() && last.bytes().all(|b| b.is_ascii_digit()) && !last.starts_with('0');
    let n: usize = last.parse().ok().filter(|_| canonical)?;
    let owner = ContentId::new(id.namespace(), id.kind(), parent).ok()?;
    playlists.get(&owner).filter(|&&songs| n <= songs).map(|_| (owner, n))
}

pub fn read(state: &ModState, key: &ContentId) -> Result<bool, String> {
    match state.get(key, FIELD) {
        None => Ok(true),
        Some(StateValue::Bool(on)) => Ok(*on),
        Some(StateValue::Int(_)) => Err(format!("{key}.{FIELD}: an integer, not true or false; the song stays on")),
        Some(StateValue::Text(_)) => Err(format!("{key}.{FIELD}: a text, not true or false; the song stays on")),
    }
}

pub fn write(state: &mut ModState, key: &ContentId, on: bool) -> bool {
    if !on {
        return state.set(key, FIELD, StateValue::Bool(false));
    }
    if let Some(fields) = state.entries.get_mut(key) {
        fields.remove(FIELD);
        if fields.is_empty() {
            state.entries.remove(key);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::music::parse_toml;

    fn id(s: &str) -> ContentId {
        s.parse().unwrap()
    }

    fn playlist(n: usize) -> MusicDef {
        let mut t = String::from("type = \"playlist\"\n");
        for i in 0..n {
            t.push_str(&format!("[[song]]\nfile = \"music/s{i}.ogg\"\n"));
        }
        parse_toml(t.as_bytes(), None).unwrap()
    }

    #[test]
    fn the_switch_ids_follow_the_records() {
        let keys = song_keys(&id("tunes:music/list"), &playlist(3)).unwrap();
        assert_eq!(keys, [id("tunes:music/list/1"), id("tunes:music/list/2"), id("tunes:music/list/3")]);
        let song = parse_toml(b"type = \"song\"\nfile = \"a.ogg\"\n", None).unwrap();
        assert_eq!(song_keys(&id("tunes:music/one"), &song).unwrap(), [id("tunes:music/one")]);
        assert!(song_keys(&id("ssx:music/licensed/145"), &song).unwrap().is_empty());
    }

    #[test]
    fn a_playlist_id_too_long_for_its_switches_is_an_error() {
        let path = "p".repeat(crate::id::MAX_ID_LEN - "tunes:music/".len());
        let long = id(&format!("tunes:music/{path}"));
        let e = song_keys(&long, &playlist(1)).unwrap_err();
        assert!(e.contains("song 1's switch id"), "{e}");
        let fits = id(&format!("tunes:music/{}", &path[..path.len() - "/1024".len()]));
        assert_eq!(song_keys(&fits, &playlist(crate::audio::music::MAX_SONGS)).unwrap().len(), 1024);
    }

    #[test]
    fn an_added_song_named_like_a_playlists_song_is_found() {
        let mut lists = BTreeMap::new();
        lists.insert(id("tunes:music/a"), 3);
        assert_eq!(playlist_song_of(&id("tunes:music/a/2"), &lists), Some((id("tunes:music/a"), 2)));
        for free in ["tunes:music/a/4", "tunes:music/a/02", "tunes:music/a/0", "tunes:music/a/x", "tunes:music/b/1"] {
            assert_eq!(playlist_song_of(&id(free), &lists), None, "{free}");
        }
        assert_eq!(playlist_song_of(&id("tunes:music/one"), &lists), None);
    }

    #[test]
    fn switches_are_written_as_deviations_from_on() {
        let key = id("tunes:music/list/2");
        let mut st = ModState::default();
        assert_eq!(read(&st, &key), Ok(true));
        assert!(write(&mut st, &key, false));
        assert_eq!(read(&st, &key), Ok(false));
        assert!(st.write().contains("[\"tunes:music/list/2\"]\non = false"), "{}", st.write());
        assert!(write(&mut st, &key, true));
        assert!(st.entries.is_empty());
        st.set(&key, "note", StateValue::Int(3));
        assert!(write(&mut st, &key, false) && write(&mut st, &key, true));
        assert_eq!(st.entries[&key].len(), 1, "the other field stays");
        st.set(&key, FIELD, StateValue::Int(3));
        assert!(read(&st, &key).unwrap_err().contains("an integer"));
        st.set(&key, FIELD, StateValue::Text("no".into()));
        assert!(read(&st, &key).unwrap_err().contains("a text"));
    }
}
