use crate::events::EventType;

pub const GHOST_ROWS: usize = 3;

pub const PADDING_CHARACTER: u32 = 0x3E7E_1256;

pub fn has_ghost_rows(t: Option<EventType>) -> bool {
    use EventType::*;
    matches!(t, Some(TrickScore | TrickBots | TrickGhosts | SurviveToDeath | RaceClock | RaceGhosts | RaceBots))
}

pub fn padding_step(t: Option<EventType>) -> u32 {
    use EventType::*;
    match t {
        Some(SurviveToBottom | SurviveToDeath) => 500_000,
        Some(RaceClock | RaceGhosts | RaceBots) => 60_000,
        _ => 25_000,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GhostEntry {
    pub value: u32,
    pub character: u32,
    pub row: Option<usize>,
}

pub fn ghost_board(rows: impl IntoIterator<Item = (u32, u32)>, t: Option<EventType>) -> Vec<GhostEntry> {
    if !has_ghost_rows(t) {
        return Vec::new();
    }
    let mut board: Vec<GhostEntry> = rows
        .into_iter()
        .take(GHOST_ROWS)
        .enumerate()
        .map(|(i, (value, character))| GhostEntry { value, character, row: Some(i) })
        .collect();
    let step = padding_step(t);
    for k in board.len()..GHOST_ROWS {
        board.push(GhostEntry { value: (k as u32 + 2) * step, character: PADDING_CHARACTER, row: None });
    }
    board_order(&mut board, t.is_some_and(EventType::ranks_ascending));
    board
}

pub fn board_order(board: &mut [GhostEntry], ascending: bool) {
    board.sort_by(|a, b| {
        use std::cmp::Ordering;
        match (a.value, b.value) {
            (x, y) if x == y => Ordering::Equal,
            (u32::MAX, _) => Ordering::Greater,
            (_, u32::MAX) => Ordering::Less,
            (x, y) if ascending => x.cmp(&y),
            (x, y) => y.cmp(&x),
        }
    });
}

pub fn explore_rank(ascending: bool, value: u32, rows: &[u32]) -> usize {
    rows.iter()
        .position(|&row| row == u32::MAX || (value != u32::MAX && if ascending { value <= row } else { row <= value }))
        .unwrap_or(rows.len())
}

pub fn beats(ascending: bool, player: u32, racer: u32) -> bool {
    player != u32::MAX && (racer == u32::MAX || if ascending { player < racer } else { racer < player })
}

pub fn bots_place(t: Option<EventType>, value: u32, racers: &[u32]) -> u32 {
    let ascending = match t {
        Some(EventType::RaceBots) => true,
        Some(EventType::TrickBots) => false,
        _ => return 0,
    };
    if racers.is_empty() {
        return 0;
    }
    racers.iter().position(|&r| beats(ascending, value, r)).map_or(u32::MAX, |k| k as u32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultsFlow {
    Explore,
    WorldTour,
}

impl ResultsFlow {
    pub fn of_drop_type(drop_type: i32) -> Option<Self> {
        match drop_type {
            0 => Some(Self::Explore),
            1 => Some(Self::WorldTour),
            _ => None,
        }
    }
}

pub fn explore_outcome(t: Option<EventType>, failed: bool, value: u32, board: &[u32]) -> (Option<usize>, bool) {
    if failed {
        return (None, false);
    }
    if t == Some(EventType::SurviveToBottom) {
        return (None, true);
    }
    let rank = explore_rank(t.is_some_and(EventType::ranks_ascending), value, board);
    (Some(rank), rank < GHOST_ROWS)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldTourRule {
    pub event_type: i64,
    pub places: Option<i64>,
    pub goal: Option<i64>,
}

impl WorldTourRule {
    pub fn won(&self, value: u32, place: u32, failed: bool, bot_scores: &[u32]) -> bool {
        if failed && self.event_type != 4 {
            return false;
        }
        let allowed = self.places.filter(|&n| n > 0).map_or(0, |n| n - 1);
        match self.event_type {
            0 => self.goal.unwrap_or(0) < i64::from(value),
            1 => bot_scores.iter().filter(|&&s| s > value).count() as i64 <= allowed,
            2..=6 => true,
            7 => i64::from(place) <= allowed,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunEnd {
    pub value: u32,
    pub code: u32,
    pub won: bool,
    pub failed: bool,
    pub gold: bool,
    pub silver: bool,
    pub bronze: bool,
}

impl RunEnd {
    pub fn explore(failed: bool, value: u32, rank: Option<usize>, won: bool) -> Self {
        if failed {
            return RunEnd { failed: true, ..Default::default() };
        }
        let medal = rank.filter(|&k| k < GHOST_ROWS);
        RunEnd {
            value,
            code: medal.map_or(4, |k| k as u32 + 1),
            won,
            failed: false,
            gold: medal == Some(0),
            silver: medal == Some(1),
            bronze: medal == Some(2),
        }
    }

    pub fn world_tour(value: u32, place: u32, won: bool, failed: bool) -> Self {
        RunEnd { value, code: place.wrapping_add(1), won: won && !failed, failed, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventType::*;

    fn values(board: &[GhostEntry]) -> Vec<u32> {
        board.iter().map(|e| e.value).collect()
    }

    #[test]
    fn the_board_takes_the_rows_in_the_types_order() {
        let race = ghost_board([(120_965, 1), (92_631, 2), (77_532, 3)], Some(RaceGhosts));
        assert_eq!(values(&race), [77_532, 92_631, 120_965]);
        assert_eq!(race.iter().map(|e| e.row).collect::<Vec<_>>(), [Some(2), Some(1), Some(0)]);
        let trick = ghost_board([(216_856, 1), (7_365_593, 2), (11_792_090, 3)], Some(TrickGhosts));
        assert_eq!(values(&trick), [11_792_090, 7_365_593, 216_856]);
    }

    #[test]
    fn the_board_takes_the_first_three_rows() {
        let board = ghost_board([(50, 1), (70, 2), (60, 3), (10, 4)], Some(RaceBots));
        assert_eq!(values(&board), [50, 60, 70]);
        assert!(board.iter().all(|e| e.row != Some(3)));
    }

    #[test]
    fn fewer_rows_are_padded() {
        let race = ghost_board([(200_000, 9)], Some(RaceGhosts));
        assert_eq!(values(&race), [180_000, 200_000, 240_000]);
        assert_eq!(race[0], GhostEntry { value: 180_000, character: PADDING_CHARACTER, row: None });
        assert_eq!(values(&ghost_board([], Some(SurviveToDeath))), [2_000_000, 1_500_000, 1_000_000]);
        assert_eq!(values(&ghost_board([], Some(TrickScore))), [100_000, 75_000, 50_000]);
        assert!(ghost_board([(5, 1)], Some(SurviveToBottom)).is_empty());
        assert!(ghost_board([(5, 1)], Some(FreeRide)).is_empty());
        assert!(ghost_board([(5, 1)], None).is_empty());
    }

    #[test]
    fn minus_one_goes_last_and_ties_keep_their_order() {
        let board = ghost_board([(u32::MAX, 1), (300, 2), (500, 3)], Some(TrickGhosts));
        assert_eq!(values(&board), [500, 300, u32::MAX]);
        let board = ghost_board([(u32::MAX, 1), (300, 2), (500, 3)], Some(RaceGhosts));
        assert_eq!(values(&board), [300, 500, u32::MAX]);
        let board = ghost_board([(300, 1), (300, 2), (300, 3)], Some(RaceGhosts));
        assert_eq!(board.iter().map(|e| e.character).collect::<Vec<_>>(), [1, 2, 3]);
    }

    #[test]
    fn the_explore_walk() {
        let rows = [77_532, 92_631, 120_965];
        assert_eq!(explore_outcome(Some(RaceGhosts), false, 101_598, &rows), (Some(2), true));
        assert_eq!(explore_outcome(Some(RaceGhosts), false, 92_631, &rows), (Some(1), true));
        assert_eq!(explore_outcome(Some(RaceGhosts), false, 130_000, &rows), (Some(3), false));
        assert_eq!(explore_outcome(Some(RaceGhosts), true, 1, &rows), (None, false));
        assert_eq!(explore_outcome(Some(SurviveToBottom), false, 1, &rows), (None, true));
        assert_eq!(explore_outcome(Some(TrickGhosts), false, 0, &[]), (Some(0), true));
        assert_eq!(explore_rank(false, 5, &[10, u32::MAX]), 1);
        assert_eq!(explore_rank(false, u32::MAX, &[10, 20]), 2);
    }

    #[test]
    fn the_bots_place() {
        assert_eq!(bots_place(Some(RaceBots), 100, &[90, 100, 110]), 2);
        assert_eq!(bots_place(Some(RaceBots), 120, &[90, 100, 110]), u32::MAX);
        assert_eq!(bots_place(Some(TrickBots), 500, &[900, 400]), 1);
        assert_eq!(bots_place(Some(RaceBots), 100, &[]), 0);
        assert_eq!(bots_place(Some(RaceGhosts), 1, &[90]), 0);
        assert_eq!(bots_place(Some(RaceBots), 100, &[u32::MAX]), 0);
    }

    #[test]
    fn the_run_end_message() {
        let e = RunEnd::explore(false, 101_598, Some(2), true);
        assert_eq!((e.code, e.won, e.bronze, e.gold), (3, true, true, false));
        assert_eq!(RunEnd::explore(false, 9, Some(3), false).code, 4);
        assert_eq!(RunEnd::explore(false, 9, None, true).code, 4);
        assert_eq!(RunEnd::explore(true, 9, Some(0), true), RunEnd { failed: true, ..Default::default() });
        let w = RunEnd::world_tour(7, 0, true, false);
        assert_eq!((w.code, w.won, w.gold), (1, true, false));
        assert_eq!(RunEnd::world_tour(7, u32::MAX, true, true).code, 0);
        assert!(!RunEnd::world_tour(7, 2, true, true).won);
    }

    #[test]
    fn the_world_tour_win() {
        let race = WorldTourRule { event_type: 7, places: Some(3), goal: None };
        assert!(race.won(1, 2, false, &[]) && !race.won(1, 3, false, &[]) && !race.won(1, u32::MAX, false, &[]));
        let trick = WorldTourRule { event_type: 1, places: None, goal: None };
        assert!(trick.won(50, 0, false, &[40, 50]) && !trick.won(50, 0, false, &[60]));
        let score = WorldTourRule { event_type: 0, places: None, goal: Some(100) };
        assert!(score.won(101, 0, false, &[]) && !score.won(100, 0, false, &[]));
        let survive = WorldTourRule { event_type: 4, ..Default::default() };
        assert!(survive.won(0, 0, true, &[]));
        assert!(!WorldTourRule { event_type: 3, ..Default::default() }.won(0, 0, true, &[]));
        assert!(!WorldTourRule { event_type: 8, ..Default::default() }.won(0, 0, false, &[]));
    }
}
