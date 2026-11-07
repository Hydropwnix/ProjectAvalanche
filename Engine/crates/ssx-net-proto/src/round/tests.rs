use super::*;
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};

fn block() -> PoseBlock {
    let mut b = [0u8; POSE_BLOCK_SIZE];
    for (i, v) in [0.0f32, 0.0, 0.0, 1.0, 120.5, -37.25, 900.0].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    PoseBlock::parse(&b).expect("240 bytes parse")
}

fn report(kind: ReportKind, split: Option<u8>) -> ProgressReport {
    ProgressReport {
        instance: 2,
        round: 7,
        kind,
        split,
        clock_ms: 61_234,
        points: 88_000,
        descent_mm: 1_234_567,
        rewinds: 1,
        loops: 0,
        sample: NetPose::at(5_400.5, crate::PoseFlags::default(), block()).expect("a sample"),
    }
}

struct Rng(u64);

impl Rng {
    fn byte(&mut self) -> u8 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 56) as u8
    }
}

#[test]
fn reports_round_trip_and_keep_their_rules() {
    let good = [
        report(ReportKind::Split, Some(0)),
        report(ReportKind::Split, Some(SPLITS - 1)),
        report(ReportKind::Points, None),
        report(ReportKind::Finish, None),
        report(ReportKind::Failed, None),
    ];
    for r in &good {
        let bytes = r.encode();
        assert_eq!(ProgressReport::decode(&bytes).as_ref(), Ok(r));
        assert_eq!(ProgressReport::new(r.clone()).as_ref(), Ok(r));
    }
    let bytes = good[0].encode();
    for n in 0..=REPORT_SIZE + 4 {
        let mut b = bytes.to_vec();
        b.resize(n, 0);
        assert_eq!(ProgressReport::decode(&b).is_ok(), n == REPORT_SIZE, "{n} bytes");
    }
    for v in 0..=255u8 {
        let mut b = bytes;
        b[4] = v;
        assert_eq!(ProgressReport::decode(&b).is_ok(), v == 1, "kind {v}");
        let mut b = bytes;
        b[5] = v;
        assert_eq!(ProgressReport::decode(&b).is_ok(), v < SPLITS, "split {v}");
        let mut b = good[3].encode();
        b[5] = v;
        assert_eq!(ProgressReport::decode(&b).is_ok(), v == 0xFF, "a finish's split {v}");
    }
    for (clock, ok) in [(MAX_CLOCK_MS, true), (MAX_CLOCK_MS + 1, false), (u32::MAX, false)] {
        let mut r = good[3].clone();
        r.clock_ms = clock;
        assert_eq!(ProgressReport::decode(&r.encode()).is_ok(), ok, "clock {clock}");
        assert_eq!(ProgressReport::new(r).is_ok(), ok);
    }
    for (points, ok) in [(MAX_POINTS, true), (MAX_POINTS + 1, false)] {
        let mut r = good[2].clone();
        r.points = points;
        assert_eq!(ProgressReport::decode(&r.encode()).is_ok(), ok, "points {points}");
    }
    let mut b = bytes;
    b[22 + 4..22 + 8].copy_from_slice(&0x0000_0010u32.to_be_bytes());
    assert!(ProgressReport::decode(&b).is_err(), "a reserved flag bit of the sample");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..20_000 {
        let mut b = bytes;
        for _ in 0..4 {
            let k = usize::from(rng.byte()) % 22;
            b[k] = rng.byte();
        }
        if let Ok(r) = ProgressReport::decode(&b) {
            assert_eq!(r.encode(), b);
        }
    }
}

#[test]
fn standings_rows_round_trip_and_keep_their_rules() {
    let row = StandingRow {
        rider: 513,
        state: StandingState::Racing,
        split: Some(4),
        clock_ms: 40_001,
        points: 12,
        descent_mm: 99,
        tick: 0xDEAD_BEEF,
        fraction: 128,
        invalid: None,
    };
    assert_eq!(StandingRow::decode(&row.encode()), Ok(row));
    for check in Check::ALL {
        let invalid = StandingRow { state: StandingState::Invalidated, invalid: Some(check), ..row };
        assert_eq!(StandingRow::decode(&invalid.encode()), Ok(invalid));
        assert_eq!(Check::from_code(check.code()), Some(check));
        assert_eq!(Check::ALL[check.index()], check);
    }
    assert_eq!(row.time(), f64::from(0xDEAD_BEEFu32) + 0.5);
    for n in 0..=STANDING_SIZE + 2 {
        let mut b = row.encode().to_vec();
        b.resize(n, 0);
        assert_eq!(StandingRow::decode(&b).is_ok(), n == STANDING_SIZE, "{n} bytes");
    }
    for v in 0..=255u8 {
        let mut b = row.encode();
        b[2] = v;
        assert_eq!(StandingRow::decode(&b).is_ok(), (1..=4).contains(&v), "state {v}: no check, not invalidated");
        let mut b = row.encode();
        b[21] = v;
        assert_eq!(StandingRow::decode(&b).is_ok(), v == 0, "check {v} on a racing row");
        let mut b = StandingRow { state: StandingState::Invalidated, invalid: Some(Check::Speed), ..row }.encode();
        b[21] = v;
        assert_eq!(StandingRow::decode(&b).is_ok(), (1..=6).contains(&v), "check {v} on an invalidated row");
        let mut b = row.encode();
        b[3] = v;
        assert_eq!(StandingRow::decode(&b).is_ok(), v < SPLITS || v == 0xFF, "split {v}");
    }
    let mut b = row.encode();
    b[4..8].copy_from_slice(&(MAX_CLOCK_MS + 1).to_be_bytes());
    assert!(StandingRow::decode(&b).is_err());
    let mut b = row.encode();
    b[8..12].copy_from_slice(&(MAX_POINTS + 1).to_be_bytes());
    assert!(StandingRow::decode(&b).is_err());
}
