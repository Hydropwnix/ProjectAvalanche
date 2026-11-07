use super::*;
use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock, pack_half};
use ssx_net_proto::PoseFlags;

fn block(q: [f32; 4], p: [f32; 3], speed: f32) -> PoseBlock {
    let mut b = [0u8; POSE_BLOCK_SIZE];
    for (i, v) in q.iter().chain(&p).enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    b[0x22..0x24].copy_from_slice(&(pack_half(speed) | 0x8000).to_be_bytes());
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    PoseBlock::parse(&b).expect("240 bytes")
}

fn sample(tick: u32, epoch: u8, p: [f32; 3]) -> NetPose {
    NetPose::new(tick, PoseFlags::default().with_warp_epoch(epoch), block([0.0, 0.0, 0.0, 1.0], p, 0.5)).unwrap()
}

#[test]
fn ticks_order_and_epochs() {
    let mut t = RiderTrack::default();
    assert_eq!(check(&mut t, 1000, TickWindow::both(300), &sample(1000, 0, [0.0; 3])), Ok(Verdict::Relay));
    assert_eq!(
        check(&mut t, 1000, TickWindow::both(300), &sample(1000, 0, [0.0; 3])),
        Ok(Verdict::Stale),
        "the same tick again"
    );
    assert_eq!(
        check(&mut t, 1000, TickWindow::both(300), &sample(998, 0, [0.0; 3])),
        Ok(Verdict::Stale),
        "an older tick"
    );
    assert_eq!(check(&mut t, 1000, TickWindow::both(300), &sample(1301, 0, [0.0; 3])), Err(Violation::Tick(301)));
    assert_eq!(check(&mut t, 1000, TickWindow::both(300), &sample(699, 0, [0.0; 3])), Err(Violation::Tick(-301)));
    assert_eq!(check(&mut t, 1004, TickWindow::both(300), &sample(1004, 1, [900.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 1006, TickWindow::both(300), &sample(1006, 0, [0.0; 3])), Ok(Verdict::OlderEpoch));
    assert_eq!(t.position(), Some([900.0, 0.0, 0.0]));
    assert!(epoch_newer(0, 255) && epoch_newer(127, 0) && !epoch_newer(128, 0) && !epoch_newer(200, 0));
    assert!(!epoch_newer(5, 5));
}

#[test]
fn values_no_ride_produces_are_violations() {
    let unit = [0.0, 0.0, 0.0, 1.0];
    let mut t = RiderTrack::default();
    let far = NetPose::new(10, PoseFlags::default(), block(unit, [0.0, 20_001.0, 0.0], 0.5)).unwrap();
    assert_eq!(check(&mut t, 10, TickWindow::both(300), &far), Err(Violation::Coordinate(20_001.0)));
    let long = NetPose::new(10, PoseFlags::default(), block([0.0, 0.0, 0.0, 1.01], [0.0; 3], 0.5)).unwrap();
    assert!(matches!(check(&mut t, 10, TickWindow::both(300), &long), Err(Violation::RotationLength(_))));
    let fast = NetPose::new(10, PoseFlags::default(), block(unit, [0.0; 3], 9.0)).unwrap();
    assert_eq!(check(&mut t, 10, TickWindow::both(300), &fast), Err(Violation::Speed(9.0)));
    assert_eq!(t, RiderTrack::default(), "nothing refused becomes the newest");
}

#[test]
fn a_step_beyond_any_ride_needs_a_warp() {
    let mut t = RiderTrack::default();
    check(&mut t, 100, TickWindow::both(300), &sample(100, 3, [0.0; 3])).unwrap();
    assert_eq!(check(&mut t, 102, TickWindow::both(300), &sample(102, 3, [30.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(
        check(&mut t, 104, TickWindow::both(300), &sample(104, 3, [70.0, 0.0, 0.0])),
        Err(Violation::Step(10.0))
    );
    for flag in [PoseFlag::Teleport, PoseFlag::Respawn, PoseFlag::Rewind] {
        let mut t2 = t;
        let p = NetPose::new(
            104,
            PoseFlags::default().with_warp_epoch(3).with(flag),
            block([0.0, 0.0, 0.0, 1.0], [5_000.0, 0.0, 0.0], 0.5),
        )
        .unwrap();
        assert_eq!(check(&mut t2, 104, TickWindow::both(300), &p), Ok(Verdict::Relay), "{flag:?}");
    }
}

fn counted(tick: u32, execution: u32, epoch: u8, p: [f32; 3]) -> NetPose {
    sample(tick, epoch, p).with_execution(execution).unwrap()
}

#[test]
fn within_an_epoch_the_execution_count_orders_and_measures() {
    let mut t = RiderTrack::default();
    assert_eq!(check(&mut t, 500, TickWindow::both(300), &counted(500, 100, 2, [0.0; 3])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 500, TickWindow::both(300), &counted(499, 102, 2, [30.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 500, TickWindow::both(300), &counted(510, 101, 2, [30.0, 0.0, 0.0])), Ok(Verdict::Stale));
    assert_eq!(check(&mut t, 500, TickWindow::both(300), &counted(500, 114, 2, [90.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(
        check(&mut t, 500, TickWindow::both(300), &counted(502, 116, 2, [190.0, 0.0, 0.0])),
        Err(Violation::Step(25.0))
    );
    let mut u = RiderTrack::default();
    assert_eq!(check(&mut u, 600, TickWindow::both(300), &counted(600, 0, 0, [0.0; 3])), Ok(Verdict::Relay));
    assert_eq!(check(&mut u, 600, TickWindow::both(300), &counted(600, 0, 0, [0.0; 3])), Ok(Verdict::Stale));
    assert_eq!(check(&mut u, 602, TickWindow::both(300), &counted(602, 0, 0, [1.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 700, TickWindow::both(300), &counted(700, 0, 3, [5.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 702, TickWindow::both(300), &counted(702, 0, 3, [5.0, 0.0, 0.0])), Ok(Verdict::Relay));
    assert_eq!(
        check(&mut t, 704, TickWindow::both(300), &counted(704, 900, 2, [5.0, 0.0, 0.0])),
        Ok(Verdict::OlderEpoch)
    );
}

#[test]
fn the_window_ahead_widens_by_the_servers_lost_ticks() {
    let w = TickWindow { behind: 300, ahead: 550, view_behind: 300, view_ahead: 550 };
    let mut t = RiderTrack::default();
    assert_eq!(check(&mut t, 1000, w, &sample(1550, 0, [0.0; 3])), Ok(Verdict::Relay));
    assert_eq!(check(&mut t, 1000, w, &sample(1551, 0, [0.0; 3])), Err(Violation::Tick(551)));
    assert_eq!(check(&mut t, 1000, w, &sample(699, 0, [0.0; 3])), Err(Violation::Tick(-301)));
}

#[test]
fn the_view_time_lies_within_its_window_of_the_stamp() {
    let w = TickWindow { behind: 300, ahead: 300, view_behind: 302, view_ahead: 362 };
    for (lead, ok) in [(0.0, true), (362.0, true), (-302.0, true), (362.5, false), (-302.25, false), (61.75, true)] {
        let mut t = RiderTrack::default();
        let p = sample(1000, 0, [0.0; 3]).with_view_time(1000.0 + lead).unwrap();
        let got = check(&mut t, 1000, w, &p);
        if ok {
            assert_eq!(got, Ok(Verdict::Relay), "{lead}");
        } else {
            assert_eq!(got, Err(Violation::ViewLead(lead)), "{lead}");
            assert_eq!(t, RiderTrack::default(), "a refused sample is not the newest");
        }
    }
}

#[test]
fn the_order_follows_the_counts_through_a_jump_of_the_stamps() {
    let o = |execution, tick| SampleOrder { epoch: 4, execution, tick };
    assert_eq!(follows(o(963, 3100), o(961, 3152)), Ok(2));
    assert_eq!(follows(o(961, 3152), o(963, 3100)), Err(Verdict::Stale));
    assert_eq!(follows(o(963, 3100), o(963, 3100)), Err(Verdict::Stale));
    assert_eq!(follows(o(0, 10), o(0, 9)), Ok(1));
    assert_eq!(follows(o(0, 9), o(0, 10)), Err(Verdict::Stale));
    assert_eq!(follows(SampleOrder { epoch: 5, execution: 0, tick: 0 }, o(963, 3100)), Ok(0));
    assert_eq!(follows(SampleOrder { epoch: 3, execution: 2000, tick: 9000 }, o(963, 3100)), Err(Verdict::OlderEpoch));
}
