use bevy_replicon::bytes::Bytes;
use bevy_replicon::postcard_utils::{from_buf, to_extend_mut};
use ssx_net_proto::{GhostStyle, Mode, Rewinds};

use super::*;

fn encode<T: Serialize>(v: &T) -> Bytes {
    let mut out = Vec::new();
    to_extend_mut(v, &mut out).expect("serializes");
    Bytes::from(out)
}

fn decode<T: for<'de> Deserialize<'de>>(bytes: &Bytes) -> Result<T, String> {
    let mut b = bytes.clone();
    from_buf::<T, _>(&mut b).map_err(|e| e.to_string())
}

fn raw_name(name: &[u8]) -> Bytes {
    let mut out = Vec::new();
    to_extend_mut(&(name.len() as u64), &mut out).expect("varint");
    out.extend_from_slice(name);
    Bytes::from(out)
}

#[test]
fn names_keep_the_rule_or_are_refused() {
    let ok = PlayerName::new("Elise Riggs").expect("a name");
    assert_eq!(decode::<PlayerName>(&encode(&ok)), Ok(ok));
    for bad in [
        "a\u{7}b".as_bytes().to_vec(),
        "a\nb".as_bytes().to_vec(),
        "日本".as_bytes().to_vec(),
        b" padded".to_vec(),
        b"two  spaces".to_vec(),
        Vec::new(),
        vec![b'x'; 21],
        vec![b'x'; 10_000],
        vec![0xFF, 0xFE],
    ] {
        assert!(decode::<PlayerName>(&raw_name(&bad)).is_err(), "{:?}", String::from_utf8_lossy(&bad));
    }
    let mut huge = Vec::new();
    to_extend_mut(&(u32::MAX as u64), &mut huge).unwrap();
    assert!(decode::<PlayerName>(&Bytes::from(huge)).is_err());
}

#[test]
fn appearances_and_instances_read_strictly() {
    let a = PlayerAppearance(Appearance::new(0x405D_C1D5, 3).unwrap());
    assert_eq!(decode::<PlayerAppearance>(&encode(&a)), Ok(a));
    let spec = InstanceSpec::new("exp_alps_vorassay_drop_3", Mode::Freeride, Rewinds::Off, true, GhostStyle::Character)
        .unwrap();
    let i = InstanceInfo { id: 7, spec, state: InstanceState::Running, round: 0 };
    assert_eq!(decode::<InstanceInfo>(&encode(&i)), Ok(i.clone()));
    let race =
        InstanceSpec::new("exp_alps_vorassay_drop_3", Mode::Race, Rewinds::On, false, GhostStyle::Character).unwrap();
    for state in InstanceState::ALL {
        let r = InstanceInfo { id: 2, spec: race.clone(), state, round: 513 };
        assert_eq!(decode::<InstanceInfo>(&encode(&r)), Ok(r));
        let f = InstanceInfo { id: 2, spec: i.spec.clone(), state, round: 0 };
        assert_eq!(decode::<InstanceInfo>(&encode(&f)).is_ok(), state == InstanceState::Running, "freeride {state:?}");
    }
    let f = InstanceInfo { round: 1, ..i.clone() };
    assert!(decode::<InstanceInfo>(&encode(&f)).is_err(), "freeride in round 1");
    let good = encode(&i).to_vec();
    for n in 0..good.len() {
        assert!(decode::<InstanceInfo>(&Bytes::from(good[..n].to_vec())).is_err(), "{n} bytes");
    }
    for byte in 0..good.len() {
        for bit in 0..8 {
            let mut b = good.clone();
            b[byte] ^= 1 << bit;
            if let Ok(read) = decode::<InstanceInfo>(&Bytes::from(b)) {
                assert!(read.spec.drop().is_some_and(|d| ssx_net_proto::instance::drop_name(d).is_ok()));
            }
        }
    }
    let id = format!("a:map/{}", "b".repeat(ssx_net_proto::instance::MAX_MAP_ID - 6));
    let g = format!("a:gamemode/{}", "c".repeat(ssx_net_proto::instance::MAX_MAP_ID - 11));
    let level = ssx_net_proto::instance::Level::map_under(&id, 6, Some(&g)).unwrap();
    let spec = InstanceSpec::on_level(level, Mode::Race, Rewinds::On, true, GhostStyle::Hologram).unwrap();
    let m = InstanceInfo { id: 3, spec, state: InstanceState::Countdown, round: 7 };
    assert_eq!(m.spec.encode().len(), ssx_net_proto::instance::MAX_SPEC_SIZE);
    assert_eq!(decode::<InstanceInfo>(&encode(&m)), Ok(m.clone()));
    let good = encode(&m).to_vec();
    for n in 0..good.len() {
        assert!(decode::<InstanceInfo>(&Bytes::from(good[..n].to_vec())).is_err(), "{n} bytes");
    }
    for gear in [4u16, 0x8000] {
        let mut b = Vec::new();
        to_extend_mut(&serde_bytes_of(&[0, 0, 0, 1, (gear >> 8) as u8, gear as u8]), &mut b).unwrap();
        assert!(decode::<PlayerAppearance>(&Bytes::from(b)).is_err(), "gear {gear:#06x}");
    }
}

fn serde_bytes_of(b: &[u8]) -> impl Serialize + '_ {
    struct B<'a>(&'a [u8]);
    impl Serialize for B<'_> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_bytes(self.0)
        }
    }
    B(b)
}

#[test]
fn pings_and_statuses_read_within_their_rules() {
    for ms in [0u16, 1, 100, MAX_PING_MS] {
        let p = PlayerPing::from_millis(u128::from(ms));
        assert_eq!(decode::<PlayerPing>(&encode(&p)), Ok(p));
    }
    assert_eq!(PlayerPing::from_millis(1 << 70).millis(), MAX_PING_MS, "a link's huge estimate is stated as the cap");
    for ms in [MAX_PING_MS + 1, u16::MAX] {
        assert!(decode::<PlayerPing>(&encode(&ms)).is_err(), "{ms} ms");
    }
    assert!(decode::<PlayerPing>(&Bytes::from(vec![0xFF, 0xFF, 0xFF, 0xFF])).is_err(), "a varint beyond u16");
    for s in [
        PlayerStatus::Joining,
        PlayerStatus::Loading,
        PlayerStatus::Intro,
        PlayerStatus::Riding,
        PlayerStatus::Away,
        PlayerStatus::Spectating,
        PlayerStatus::Finished,
    ] {
        assert_eq!(decode::<PlayerState>(&encode(&PlayerState(s))), Ok(PlayerState(s)));
    }
    for b in [0u8, 8, 0xFF] {
        assert!(decode::<PlayerState>(&encode(&b)).is_err(), "status {b}");
    }
}

#[test]
fn standings_and_their_diffs_read_strictly() {
    use crate::protocol::{MAX_STANDINGS, Standings, StandingsDiff};
    use bevy_replicon::prelude::Diffable;
    use ssx_net_proto::{StandingRow, StandingState};
    let row = |rider: u16| StandingRow {
        rider,
        state: StandingState::Finished,
        split: None,
        clock_ms: 60_000 + u32::from(rider),
        points: 0,
        descent_mm: 0,
        tick: 9_000,
        fraction: 3,
        invalid: None,
    };
    let mut table = Standings::new(3, 12);
    for k in 0..MAX_STANDINGS as u16 {
        table.apply_diff(&StandingsDiff::Set { slot: k as u8, row: row(k) }).expect("an append");
    }
    assert!(table.apply_diff(&StandingsDiff::Set { slot: MAX_STANDINGS as u8, row: row(999) }).is_err(), "full");
    assert_eq!(decode::<Standings>(&encode(&table)), Ok(table.clone()));
    let mut short = Standings::new(3, 12);
    assert!(short.apply_diff(&StandingsDiff::Set { slot: 1, row: row(1) }).is_err(), "a gap");
    let bytes = |rows: &[StandingRow], count: u8| {
        let mut b = vec![0, 3, 0, 12, count];
        for r in rows {
            b.extend_from_slice(&r.encode());
        }
        let mut out = Vec::new();
        to_extend_mut(&serde_bytes_of(&b), &mut out).unwrap();
        Bytes::from(out)
    };
    assert!(decode::<Standings>(&bytes(&[row(1), row(2)], 2)).is_ok());
    assert!(decode::<Standings>(&bytes(&[row(1), row(2)], 3)).is_err());
    assert!(decode::<Standings>(&bytes(&[row(1), row(1)], 2)).is_err(), "a rider twice");
    let rows: Vec<StandingRow> = (0..=MAX_STANDINGS as u16).map(row).collect();
    assert!(decode::<Standings>(&bytes(&rows, rows.len() as u8)).is_err(), "one row beyond the cap");
    let mut broken = row(1).encode();
    broken[2] = 9;
    let mut b = vec![0, 3, 0, 12, 1];
    b.extend_from_slice(&broken);
    let mut out = Vec::new();
    to_extend_mut(&serde_bytes_of(&b), &mut out).unwrap();
    assert!(decode::<Standings>(&Bytes::from(out)).is_err(), "an unknown state");
    for d in [StandingsDiff::Round(4), StandingsDiff::Set { slot: 2, row: row(7) }, StandingsDiff::Truncate(1)] {
        assert_eq!(decode::<StandingsDiff>(&encode(&d)), Ok(d));
    }
    for bad in [&[][..], &[1, 0], &[2, 0], &[3], &[3, 1, 1], &[4, 0, 0], &[1, 0, 0, 0]] {
        let mut out = Vec::new();
        to_extend_mut(&serde_bytes_of(bad), &mut out).unwrap();
        assert!(decode::<StandingsDiff>(&Bytes::from(out)).is_err(), "{bad:?}");
    }
}
