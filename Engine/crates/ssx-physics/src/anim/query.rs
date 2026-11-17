pub mod actor_type {
    pub const SNOWBOARDER: i32 = 0;
    pub const CAMERA: i32 = 2;
    pub const HELICOPTER: i32 = 4;
    pub const GHOST: i32 = 5;
}

pub const FIND_HELI: &str = "QueryTask_FindHeli";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryTask {
    FindHeli,
}

impl QueryTask {
    pub fn from_name(task: &str) -> Option<Self> {
        (task == FIND_HELI).then_some(Self::FindHeli)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Animatable {
    Player,
    Helicopter,
    Ghost(usize),
}

pub fn find_heli(me: Animatable, candidates: impl IntoIterator<Item = (Animatable, i32)>) -> Option<Animatable> {
    candidates.into_iter().find(|&(a, ty)| a != me && ty == actor_type::HELICOPTER).map(|(a, _)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_heli_takes_the_first_other_helicopter() {
        let world = [
            (Animatable::Player, actor_type::SNOWBOARDER),
            (Animatable::Ghost(0), actor_type::GHOST),
            (Animatable::Helicopter, actor_type::HELICOPTER),
            (Animatable::Ghost(1), actor_type::HELICOPTER),
        ];
        assert_eq!(find_heli(Animatable::Player, world), Some(Animatable::Helicopter));
        assert_eq!(find_heli(Animatable::Helicopter, world), Some(Animatable::Ghost(1)));
    }

    #[test]
    fn find_heli_without_a_helicopter_is_none() {
        let world = [(Animatable::Player, actor_type::SNOWBOARDER), (Animatable::Ghost(0), actor_type::GHOST)];
        assert_eq!(find_heli(Animatable::Player, world), None);
        assert_eq!(QueryTask::from_name("QueryTask_Player_NearHeliGrabHandle"), None);
        assert_eq!(QueryTask::from_name(FIND_HELI), Some(QueryTask::FindHeli));
    }
}
