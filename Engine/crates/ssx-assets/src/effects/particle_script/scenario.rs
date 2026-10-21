use std::sync::Arc;

use ssx_lua::game::Binder;
use ssx_lua::game::controllers::{StateSets, Track};
use ssx_lua::game::graph::run_batch;

pub fn state_sets(src: &str) -> Arc<StateSets> {
    Arc::new(StateSets::from_graph(&run_batch(
        Binder::StateSets,
        &[("controller".into(), src.as_bytes().to_vec())],
        None,
    )))
}

pub fn describe(sets: &StateSets) -> Vec<String> {
    let Some(set) = sets.sets.first() else { return Vec::new() };
    let one = |t: usize| -> Option<String> {
        Some(match &sets.kinds[t] {
            Track::Particle { target, on_start, on_end } => format!("particles {target:?} {on_start:?}/{on_end:?}"),
            Track::Light { name, group, on_start, update, on_end, .. } => {
                format!("lights {name:?}{} {on_start:?}/{update:?}/{on_end:?}", if *group { " group" } else { "" })
            }
            _ => return None,
        })
    };
    set.states
        .iter()
        .filter_map(|st| {
            let mut v: Vec<String> = st.enter.iter().filter_map(|&t| one(t).map(|s| format!("enter {s}"))).collect();
            v.extend(st.update.iter().filter_map(|&t| one(t)));
            (!v.is_empty()).then(|| format!("{}: {}", st.name, v.join("; ")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_the_states_tracks() {
        let src = "local s = CreateStateSet( \"X\" )\n local st = s:AddState( \"Idle_GAME\" )\n\
            local t = LightTrack( \"\" )\n t:ActivateOnStart()\n t:BlinkOnUpdate(0.4, 0.5)\n st:AddUpdateTrack( t )\n";
        let d = super::describe(&super::state_sets(src));
        assert_eq!(d.len(), 1);
        assert!(d[0].starts_with("Idle_GAME: lights \"\" Activate/Blink"), "{d:?}");
    }
}
