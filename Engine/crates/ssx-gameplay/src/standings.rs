use core::cmp::Ordering;

use ssx_net_proto::{Mode, StandingRow, StandingState};

fn measure(mode: Mode, a: &StandingRow, b: &StandingRow) -> Ordering {
    match mode {
        Mode::Race | Mode::Freeride => match (a.split, b.split) {
            (Some(x), Some(y)) => y.cmp(&x).then(a.clock_ms.cmp(&b.clock_ms)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
        Mode::Trick => b.points.cmp(&a.points),
        Mode::Survive => b.descent_mm.cmp(&a.descent_mm),
    }
}

fn group(mode: Mode, row: &StandingRow, last: bool) -> u8 {
    match row.state {
        StandingState::Finished if mode == Mode::Race || last => 0,
        StandingState::Finished | StandingState::Racing => 1,
        StandingState::Failed => 2,
        StandingState::Left => 3,
        StandingState::Invalidated => 4,
    }
}

pub fn compare(mode: Mode, a: &StandingRow, b: &StandingRow, last: bool) -> Ordering {
    let ga = group(mode, a, last);
    let by_measure = if mode == Mode::Race && ga == 0 { a.clock_ms.cmp(&b.clock_ms) } else { measure(mode, a, b) };
    ga.cmp(&group(mode, b, last)).then(by_measure).then(a.time().total_cmp(&b.time())).then(a.rider.cmp(&b.rider))
}

pub fn order(mode: Mode, rows: &mut [StandingRow], last: bool) {
    rows.sort_by(|a, b| compare(mode, a, b, last));
}

pub fn medal(rows: &[StandingRow], place: usize) -> Option<u8> {
    let row = rows.get(place)?;
    if row.state != StandingState::Finished {
        return None;
    }
    match place {
        0 => Some(2),
        1 => Some(1),
        2 => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rider: u16, state: StandingState, split: Option<u8>, clock_ms: u32, tick: u32) -> StandingRow {
        StandingRow { rider, state, split, clock_ms, points: 0, descent_mm: 0, tick, fraction: 0, invalid: None }
    }

    fn riders(rows: &[StandingRow]) -> Vec<u16> {
        rows.iter().map(|r| r.rider).collect()
    }

    #[test]
    fn a_race_orders_by_the_finish_clock_then_the_furthest_split() {
        use StandingState::*;
        let mut rows = vec![
            row(1, Racing, Some(3), 30_000, 900),
            row(2, Finished, None, 61_000, 2_000),
            row(3, Racing, Some(5), 52_000, 1_600),
            row(4, Finished, None, 59_500, 2_100),
            row(5, Failed, Some(8), 70_000, 2_200),
            row(6, Racing, None, 0, 0),
            row(7, Left, Some(8), 1_000, 10),
            row(8, Racing, Some(5), 50_000, 1_700),
            row(9, Finished, None, 59_500, 2_105),
        ];
        order(Mode::Race, &mut rows, false);
        assert_eq!(riders(&rows), [4, 9, 2, 8, 3, 1, 6, 5, 7]);
        let mut two = vec![row(2, Finished, None, 61_000, 1_000), row(4, Finished, None, 59_500, 5_000)];
        order(Mode::Race, &mut two, true);
        assert_eq!(riders(&two), [4, 2]);
        assert_eq!(
            (medal(&rows, 0), medal(&rows, 1), medal(&rows, 2), medal(&rows, 3)),
            (Some(2), Some(1), Some(0), None)
        );
    }

    #[test]
    fn tricks_by_points_and_survives_by_descent_with_the_finished_first_at_the_end() {
        use StandingState::*;
        let mut rows = vec![
            StandingRow { points: 50_000, ..row(1, Finished, None, 90_000, 3_000) },
            StandingRow { points: 80_000, ..row(2, Racing, Some(7), 80_000, 2_900) },
            StandingRow { points: 10_000, ..row(3, Failed, Some(2), 20_000, 800) },
            StandingRow { points: 50_000, ..row(4, Finished, None, 95_000, 3_100) },
        ];
        order(Mode::Trick, &mut rows, false);
        assert_eq!(riders(&rows), [2, 1, 4, 3], "live: by the points");
        order(Mode::Trick, &mut rows, true);
        assert_eq!(riders(&rows), [1, 4, 2, 3], "final: the finished first, ties to the earlier time");
        assert_eq!(medal(&rows, 2), None, "a run without its finish has no medal");
        let mut rows = vec![
            StandingRow { descent_mm: 1_500_000, ..row(1, Finished, None, 0, 10) },
            StandingRow { descent_mm: 2_500_000, ..row(2, Finished, None, 0, 20) },
            StandingRow { descent_mm: 2_000_000, ..row(3, Racing, Some(4), 0, 30) },
        ];
        order(Mode::Survive, &mut rows, false);
        assert_eq!(riders(&rows), [2, 3, 1]);
        order(Mode::Survive, &mut rows, true);
        assert_eq!(riders(&rows), [2, 1, 3]);
    }
}
