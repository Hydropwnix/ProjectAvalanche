use super::*;

fn valid_block_bytes(seed: u64) -> [u8; POSE_BLOCK_SIZE] {
    let mut rng = Rng(seed | 1);
    let mut b = [0u8; POSE_BLOCK_SIZE];
    for (i, v) in [0.0f32, 0.0, 0.0, 1.0, 1024.5, -37.25, 9000.0].iter().enumerate() {
        b[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    for (start, end) in [(0x1C, 0x24), (0x30, 0x47), (0x4C, POSE_BLOCK_SIZE)] {
        b[start..end].iter_mut().for_each(|x| *x = rng.byte());
    }
    b[0x48..0x4C].copy_from_slice(&1.0f32.to_be_bytes());
    b
}

fn valid_sample_bytes(seed: u64) -> [u8; NET_POSE_SIZE] {
    let mut s = [0u8; NET_POSE_SIZE];
    s[..4].copy_from_slice(&123_456u32.to_be_bytes());
    let flags = PoseFlags::default().with(PoseFlag::Teleport).with(PoseFlag::ImpactLatched).with_warp_epoch(7);
    s[4..8].copy_from_slice(&flags.bits().to_be_bytes());
    s[8..12].copy_from_slice(&4_321u32.to_be_bytes());
    s[12..16].copy_from_slice(&(-1_234_567i32).to_be_bytes());
    s[BLOCK..].copy_from_slice(&valid_block_bytes(seed));
    s
}

const BLOCK: usize = 16;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 56) as u8
    }
}

#[test]
fn valid_sample_round_trips_byte_for_byte() {
    for seed in 0..64 {
        let bytes = valid_sample_bytes(seed);
        let pose = NetPose::decode(&bytes).expect("a packer-shaped sample decodes");
        assert_eq!(pose.encode(), bytes);
        assert_eq!(pose.tick(), 123_456);
        assert!(pose.flags().contains(PoseFlag::Teleport) && pose.flags().contains(PoseFlag::ImpactLatched));
        assert!(!pose.flags().contains(PoseFlag::Rewind) && !pose.flags().contains(PoseFlag::Respawn));
        assert_eq!(pose.flags().warp_epoch(), 7);
        let relayed = RelayedPose { rider: RiderId(0xBEEF), pose };
        let wire = relayed.encode();
        assert_eq!(&wire[..2], &[0xBE, 0xEF]);
        assert_eq!(RelayedPose::decode(&wire), Ok(relayed));
    }
}

#[test]
fn every_wrong_length_is_refused() {
    let big = [0u8; 2048];
    for n in (0..=big.len()).filter(|&n| n != NET_POSE_SIZE) {
        let err = NetPose::decode(&big[..n]).unwrap_err();
        assert_eq!(err, DecodeError::Size { kind: "pose sample", expected: NET_POSE_SIZE, actual: n });
    }
    for n in (0..=big.len()).filter(|&n| n != RELAYED_POSE_SIZE) {
        assert!(matches!(RelayedPose::decode(&big[..n]), Err(DecodeError::Size { actual, .. }) if actual == n));
    }
}

#[test]
fn pseudo_random_samples_never_panic() {
    let mut rng = Rng(0x5EED_5EED_5EED_5EED);
    let (mut ok, mut refused) = (0u32, 0u32);
    for _ in 0..200_000 {
        let mut bytes = [0u8; RELAYED_POSE_SIZE];
        bytes.iter_mut().for_each(|b| *b = rng.byte());
        if rng.next() & 1 == 0 {
            bytes[2 + BLOCK..].copy_from_slice(&valid_block_bytes(rng.next()));
            let flags = (rng.next() as u32) & PoseFlags::DEFINED;
            bytes[6..10].copy_from_slice(&flags.to_be_bytes());
            let execution =
                (rng.next() as u32) & if rng.next().is_multiple_of(8) { u32::MAX } else { crate::MAX_EXECUTION };
            bytes[10..14].copy_from_slice(&execution.to_be_bytes());
        }
        match (NetPose::decode(&bytes[2..]), RelayedPose::decode(&bytes)) {
            (Ok(p), Ok(r)) => {
                assert_eq!(p, r.pose);
                assert_eq!(p.encode()[..], bytes[2..]);
                ok += 1;
            }
            (Err(a), Err(b)) => {
                assert_eq!(a, b);
                refused += 1;
            }
            (a, b) => panic!("the two decoders disagree: {a:?} / {b:?}"),
        }
    }
    assert!(ok > 0 && refused > 0, "both outcomes reached: {ok} accepted, {refused} refused");
}

#[test]
fn reserved_flag_bits_are_refused_one_by_one() {
    for bit in (0..32).map(|i| 1u32 << i) {
        let mut bytes = valid_sample_bytes(1);
        bytes[4..8].copy_from_slice(&bit.to_be_bytes());
        let defined = bit & PoseFlags::DEFINED != 0;
        assert_eq!(NetPose::decode(&bytes).is_ok(), defined, "flag bit {bit:#x}");
        if !defined {
            assert_eq!(NetPose::decode(&bytes), Err(DecodeError::ReservedFlags(bit)));
        }
    }
}

#[test]
fn non_finite_floats_are_refused_in_every_float_field() {
    for field in 0..7 {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut bytes = valid_sample_bytes(2);
            bytes[BLOCK + field * 4..BLOCK + 4 + field * 4].copy_from_slice(&bad.to_be_bytes());
            assert!(matches!(NetPose::decode(&bytes), Err(DecodeError::NonFinite(_))), "field {field} = {bad}");
        }
    }
}

#[test]
fn pose_weight_must_be_exactly_one() {
    for w in [0.0f32, -1.0, 0.999_999_94, 1.000_000_1, f32::NAN, f32::INFINITY, -0.0] {
        let mut bytes = valid_sample_bytes(3);
        bytes[BLOCK + 0x48..BLOCK + 0x4C].copy_from_slice(&w.to_be_bytes());
        assert!(matches!(NetPose::decode(&bytes), Err(DecodeError::PoseWeight(_))), "weight {w}");
    }
}

#[test]
fn unwritten_bytes_must_be_zero() {
    for offset in (0x24..0x30).chain([0x47]) {
        let mut bytes = valid_sample_bytes(4);
        bytes[BLOCK + offset] = 0x01;
        assert_eq!(NetPose::decode(&bytes), Err(DecodeError::UnwrittenBytes), "block offset {offset:#x}");
    }
}

#[test]
fn constructor_applies_the_decoders_checks() {
    let block = PoseBlock::parse(&valid_block_bytes(5)).unwrap();
    let mut nan = block.clone();
    nan.trajectory_position[1] = f32::NAN;
    assert!(NetPose::new(1, PoseFlags::default(), nan).is_err());
    let mut stack = block.clone();
    stack.unk_47 = 9;
    assert_eq!(NetPose::new(1, PoseFlags::default(), stack), Err(DecodeError::UnwrittenBytes));
    assert!(NetPose::new(1, PoseFlags::default(), block).is_ok());
}

#[test]
fn flags_builder_sets_only_defined_bits() {
    let all = [PoseFlag::Teleport, PoseFlag::Rewind, PoseFlag::Respawn, PoseFlag::ImpactLatched]
        .into_iter()
        .fold(PoseFlags::default(), PoseFlags::with)
        .with_warp_epoch(0xFF)
        .with_tick_fraction(0xFF);
    assert_eq!(all.bits(), PoseFlags::DEFINED);
    assert_eq!(all.with_warp_epoch(3).warp_epoch(), 3);
    assert_eq!(all.with_tick_fraction(7).tick_fraction(), 7);
    assert_eq!(all.with_tick_fraction(7).warp_epoch(), 0xFF, "the fraction leaves the epoch");
    assert_eq!(PoseFlags::from_bits(all.bits()), Ok(all));
}

#[test]
fn the_time_carries_its_fraction_and_refuses_what_is_no_tick() {
    let block = PoseBlock::parse(&valid_block_bytes(6)).unwrap();
    let flags = PoseFlags::default().with_warp_epoch(4);
    let p = NetPose::at(1234.75, flags, block.clone()).unwrap();
    assert_eq!((p.tick(), p.flags().tick_fraction(), p.flags().warp_epoch()), (1234, 192, 4));
    assert_eq!(p.time(), 1234.75);
    let back = NetPose::decode(&p.encode()).unwrap();
    assert_eq!(back.time(), 1234.75, "the fraction crosses the wire");
    assert_eq!(NetPose::at(9.0 + 0.999_999, flags, block.clone()).unwrap().flags().tick_fraction(), 255);
    assert_eq!(NetPose::at(0.0, flags, block.clone()).unwrap().time(), 0.0);
    for bad in [-0.5, f64::NAN, f64::INFINITY, f64::from(u32::MAX) + 1.0] {
        assert!(matches!(NetPose::at(bad, flags, block.clone()), Err(DecodeError::Time(_))), "{bad}");
    }
}

#[test]
fn the_execution_index_round_trips_within_24_bits_and_its_top_byte_is_refused() {
    let bytes = valid_sample_bytes(5);
    let pose = NetPose::decode(&bytes).expect("a valid sample");
    assert_eq!(pose.execution(), 4_321);
    for e in [0, 1, 22, crate::MAX_EXECUTION] {
        let p = pose.clone().with_execution(e).expect("within 24 bits");
        assert_eq!(NetPose::decode(&p.encode()).map(|q| q.execution()), Ok(e));
    }
    assert_eq!(pose.clone().with_execution(crate::MAX_EXECUTION + 1), Err(DecodeError::Execution(1 << 24)));
    for top in [1u32 << 24, 1 << 31, u32::MAX] {
        let mut b = bytes;
        b[8..12].copy_from_slice(&top.to_be_bytes());
        assert_eq!(NetPose::decode(&b), Err(DecodeError::Execution(top)), "{top:#x}");
    }
}

#[test]
fn the_view_time_rides_as_its_lead_over_the_stamp_and_refuses_what_the_wire_cannot_carry() {
    let block = PoseBlock::parse(&valid_block_bytes(7)).unwrap();
    let flags = PoseFlags::default().with_warp_epoch(2);
    let p = NetPose::at(1000.5, flags, block.clone()).unwrap();
    assert_eq!((p.view_lead(), p.view_time()), (0.0, 1000.5), "no view time: the stamp's own");
    for (view, lead) in [(1061.75, 61.25), (995.25, -5.25), (1000.5, 0.0)] {
        let q = p.clone().with_view_time(view).unwrap();
        assert_eq!((q.view_lead(), q.view_time()), (lead, view));
        let back = NetPose::decode(&q.encode()).unwrap();
        assert_eq!(back, q, "the lead crosses the wire");
        assert_eq!(back.view_time(), view);
    }
    let r = NetPose::at(10.0 + 0.3, flags, block.clone()).unwrap().with_view_time(20.0 + 0.3).unwrap();
    assert!((r.view_time() - 20.3).abs() <= 1.0 / 512.0 + 1e-12, "{}", r.view_time());
    assert_eq!(r.view_lead() * 256.0, (r.view_lead() * 256.0).round());
    let max = f64::from(i32::MAX) / 256.0;
    let min = f64::from(i32::MIN) / 256.0;
    assert_eq!(p.clone().with_view_time(1000.5 + max).unwrap().view_lead(), max);
    assert_eq!(p.clone().with_view_time(1000.5 + min).unwrap().view_lead(), min);
    for bad in [1000.5 + max + 1.0, 1000.5 + min - 1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(p.clone().with_view_time(bad), Err(DecodeError::ViewTime(_))), "{bad}");
    }
    for word in [i32::MIN, -1, 0, 1, i32::MAX] {
        let mut bytes = valid_sample_bytes(8);
        bytes[12..16].copy_from_slice(&word.to_be_bytes());
        let q = NetPose::decode(&bytes).expect("any lead");
        assert_eq!(q.view_lead(), f64::from(word) / 256.0);
        assert_eq!(q.encode(), bytes);
    }
    assert_eq!(NET_POSE_SIZE, 256);
    assert_eq!(RELAYED_POSE_SIZE, 258);
}
