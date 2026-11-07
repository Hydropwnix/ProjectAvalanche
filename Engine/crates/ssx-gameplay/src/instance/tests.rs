use super::*;
use ssx_net_proto::{GhostStyle, Rewinds};

fn instance(capacity: usize) -> Instance {
    let spec = InstanceSpec::new("exp_alps_vorassay_drop_3", Mode::Freeride, Rewinds::On, false, GhostStyle::Character)
        .expect("spec");
    Instance::freeride(1, spec, capacity)
}

fn ok(i: &mut Instance, rider: u16, m: ClientControl) -> Progress {
    i.control(rider, &m, 0).unwrap_or_else(|e| panic!("{m:?}: {e}"));
    i.progress(rider).expect("a member")
}

fn refused(i: &mut Instance, rider: u16, m: ClientControl) -> Refusal {
    i.control(rider, &m, 0).expect_err("refused").rule
}

const LOADED: ClientControl = ClientControl::Loaded { round: 0 };
fn ready() -> ClientControl {
    use ssx_formats::ghost::{POSE_BLOCK_SIZE, PoseBlock};
    let mut b = [0u8; POSE_BLOCK_SIZE];
    b[12..16].copy_from_slice(&1.0f32.to_be_bytes());
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    let block = PoseBlock::parse(&b).expect("240 bytes parse");
    let sample = ssx_net_proto::NetPose::new(0, ssx_net_proto::PoseFlags::default(), block).expect("a sample");
    let second = Box::new(sample.clone().with_execution(2).expect("an execution"));
    ClientControl::Ready { round: 0, sample: Box::new(sample), second }
}

#[test]
fn freeride_runs_from_the_start_and_takes_riders_until_full() {
    let mut i = instance(2);
    assert_eq!(i.state(), InstanceState::Running);
    assert_eq!(i.admit(4), Ok(()));
    assert_eq!(i.admit(4), Err(Admission::Present));
    assert_eq!(i.admit(9), Ok(()));
    assert_eq!(i.admit(1), Err(Admission::Full));
    assert!(i.leave(4));
    assert!(!i.leave(4));
    assert_eq!(i.admit(1), Ok(()));
    assert_eq!(i.len(), 2);
    assert_eq!(i.tick(10_000), crate::round::Changes::default(), "freeride has no cycle");
}

#[test]
fn control_messages_come_in_their_order_once() {
    let a = Appearance::new(0x405D_C1D5, 1).unwrap();
    let mut i = instance(4);
    i.admit(3).unwrap();
    assert_eq!(refused(&mut i, 3, LOADED), Refusal::Order, "loaded before the look");
    assert_eq!(refused(&mut i, 3, ready()), Refusal::Order);
    assert_eq!(ok(&mut i, 3, ClientControl::Appearance(a)), Progress::Dressed(a));
    assert_eq!(refused(&mut i, 3, ClientControl::Appearance(a)), Refusal::Order, "the look twice");
    assert_eq!(refused(&mut i, 3, ready()), Refusal::Order, "ready before loaded");
    assert_eq!(refused(&mut i, 3, ClientControl::Loaded { round: 1 }), Refusal::NoRounds, "freeride's round is 0");
    assert_eq!(ok(&mut i, 3, LOADED), Progress::Loaded(a));
    assert_eq!(refused(&mut i, 3, LOADED), Refusal::Order);
    assert_eq!(ok(&mut i, 3, ready()), Progress::Ready(a));
    for m in [ClientControl::Appearance(a), LOADED, ready()] {
        assert_eq!(refused(&mut i, 3, m.clone()), Refusal::Order, "{m:?} after ready");
    }
    assert_eq!(refused(&mut i, 3, ClientControl::StartAck { round: 0 }), Refusal::NoRounds);
    assert_eq!(refused(&mut i, 8, ClientControl::Appearance(a)), Refusal::Order, "a rider not in the instance");
    assert_eq!(i.progress(3).and_then(Progress::appearance), Some(a));
    assert_eq!(i.relay_roles(3), (true, true));
}

#[test]
fn a_level_left_takes_a_new_look_in_the_same_order() {
    let (a, b) = (Appearance::new(0x405D_C1D5, 1).unwrap(), Appearance::new(0x55BB_536F, 2).unwrap());
    let mut i = instance(4);
    i.admit(3).unwrap();
    assert_eq!(refused(&mut i, 3, ClientControl::LevelLeft), Refusal::Order, "level left before the look");
    ok(&mut i, 3, ClientControl::Appearance(a));
    assert_eq!(ok(&mut i, 3, ClientControl::LevelLeft), Progress::Away, "while the level loads");
    ok(&mut i, 3, ClientControl::Appearance(a));
    ok(&mut i, 3, LOADED);
    assert_eq!(ok(&mut i, 3, ClientControl::LevelLeft), Progress::Away, "from the intro");
    assert_eq!(i.status(3), Some(PlayerStatus::Away));
    assert_eq!(i.progress(3).and_then(Progress::appearance), None);
    for m in [ClientControl::LevelLeft, LOADED, ready()] {
        assert_eq!(refused(&mut i, 3, m.clone()), Refusal::Order, "{m:?} while away");
    }
    assert_eq!(ok(&mut i, 3, ClientControl::Appearance(b)), Progress::Dressed(b));
    assert_eq!(refused(&mut i, 3, ClientControl::Appearance(b)), Refusal::Order, "the new look twice");
    ok(&mut i, 3, LOADED);
    ok(&mut i, 3, ready());
    assert_eq!(ok(&mut i, 3, ClientControl::LevelLeft), Progress::Away, "from a ride");
    assert_eq!(ok(&mut i, 3, ClientControl::Appearance(a)), Progress::Dressed(a));
    assert_eq!(i.status(3), Some(PlayerStatus::Loading));
}
