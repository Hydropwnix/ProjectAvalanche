use super::*;

fn every_kind() -> Vec<ScriptItem> {
    let p = |s: &str| Some(s.to_string());
    let marker = WireMarker {
        id: "ava:marker/ring".into(),
        position: [1.0, -2.0, 3.0],
        heading: 90.0,
        size: 4.0,
        colour: 7,
        visible: true,
        checkpoint: Some(3),
        instance: Some(9),
    };
    let bodies = vec![
        ScriptBody::Event { name: "score".into(), source: 7, args: vec![1, 2, 3] },
        ScriptBody::LatentStart { transfer: 9, name: "big".into(), source: 1, total: 100_000 },
        ScriptBody::LatentChunk { transfer: 9, offset: 16_384, bytes: vec![5; 300] },
        ScriptBody::LatentCancel { transfer: 9 },
        ScriptBody::DataWrite { element: 12, key: "ready".into(), value: Some(vec![1]) },
        ScriptBody::Reset,
        ScriptBody::Created { element: 12, kind: "rider".into(), name: "Kaori".into(), parent: 1 },
        ScriptBody::Destroyed { element: 12 },
        ScriptBody::Moved { element: 12, parent: 3 },
        ScriptBody::Renamed { element: 12, name: "Elise".into() },
        ScriptBody::Data { element: 12, key: "lap".into(), value: None },
        ScriptBody::Hello { player: 4 },
        ScriptBody::Rider { element: 5, player: 4, number: 2 },
        ScriptBody::Marker { element: 6, marker },
        ScriptBody::Notice { title: "Round".into(), text: "über".into(), seconds: 2.5 },
        ScriptBody::Sound { id: "ssx:sound/hud/gate".into(), position: Some([0.0, 1.0, 2.0]) },
        ScriptBody::Started,
    ];
    bodies
        .into_iter()
        .enumerate()
        .map(|(k, body)| ScriptItem { package: if k % 3 == 0 { None } else { p("scoreboard") }, body })
        .collect()
}

#[test]
fn every_kind_round_trips_in_its_direction() {
    for item in every_kind() {
        let b = item.encode().unwrap();
        let (ok, other) = if item.body.from_client() && !item.body.from_server() {
            (Direction::ToServer, Some(Direction::ToClient))
        } else if item.body.from_client() {
            (Direction::ToServer, None)
        } else {
            (Direction::ToClient, Some(Direction::ToServer))
        };
        assert_eq!(ScriptItem::decode(&b, ok).unwrap(), item);
        if let Some(d) = other {
            assert!(ScriptItem::decode(&b, d).is_err(), "{item:?} travels {d:?}");
        }
    }
}

#[test]
fn cut_and_random_bytes_are_refused() {
    for item in every_kind() {
        let b = item.encode().unwrap();
        for n in 0..b.len() {
            for d in [Direction::ToServer, Direction::ToClient] {
                assert!(ScriptItem::decode(&b[..n], d).is_err());
            }
        }
        let mut long = b.clone();
        long.push(0);
        assert!(ScriptItem::decode(&long, Direction::ToClient).is_err());
    }
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for len in 0..2000 {
        let bytes: Vec<u8> = (0..len % 97)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect();
        let _ = ScriptItem::decode(&bytes, Direction::ToServer);
        let _ = ScriptItem::decode(&bytes, Direction::ToClient);
        let _ = Assembler::default().take(&bytes);
    }
}

#[test]
fn fields_out_of_range_are_refused() {
    let item = |body| ScriptItem { package: None, body };
    let long = "a".repeat(MAX_NAME + 1);
    assert!(item(ScriptBody::Event { name: long, source: 1, args: vec![] }).encode().is_err());
    assert!(item(ScriptBody::Event { name: "a".into(), source: 1, args: vec![0; MAX_ARGS + 1] }).encode().is_err());
    for bad in [0, 1 << 53] {
        let b = item(ScriptBody::Hello { player: bad }).encode().unwrap();
        assert!(ScriptItem::decode(&b, Direction::ToClient).is_err(), "element {bad}");
    }
    let b = item(ScriptBody::Notice { title: "t".into(), text: "x".into(), seconds: f32::NAN }).encode().unwrap();
    assert!(ScriptItem::decode(&b, Direction::ToClient).is_err(), "NaN");
    let b = item(ScriptBody::Notice { title: "t\u{1b}[2J".into(), text: "x".into(), seconds: 1.0 }).encode().unwrap();
    assert!(ScriptItem::decode(&b, Direction::ToClient).is_err(), "a control character");
    let b = item(ScriptBody::LatentStart { transfer: 1, name: "a".into(), source: 1, total: MAX_LATENT + 1 })
        .encode()
        .unwrap();
    assert!(ScriptItem::decode(&b, Direction::ToServer).is_err(), "a latent transfer beyond the cap");
    let pkg = ScriptItem { package: Some("a b".into()), body: ScriptBody::LatentCancel { transfer: 1 } };
    assert!(ScriptItem::decode(&pkg.encode().unwrap(), Direction::ToServer).is_err(), "a package name with a space");
    for (name, ok) in [("my.board-2_x", true), ("My", false), ("..", false), (".", false), ("a/b", false)] {
        let item = ScriptItem { package: Some(name.into()), body: ScriptBody::Started };
        assert_eq!(ScriptItem::decode(&item.encode().unwrap(), Direction::ToServer).is_ok(), ok, "{name}");
    }
    let started = ScriptItem { package: Some("p".into()), body: ScriptBody::Started }.encode().unwrap();
    assert!(ScriptItem::decode(&started, Direction::ToClient).is_err(), "an acknowledgement is a client's");
}

#[test]
fn items_cross_in_pieces_within_the_budget() {
    let big = ScriptItem {
        package: Some("p".into()),
        body: ScriptBody::Event { name: "e".into(), source: 1, args: vec![9; MAX_ARGS] },
    };
    let small = ScriptItem { package: None, body: ScriptBody::Destroyed { element: 3 } };
    let mut out = Outgoing::new(1 << 20);
    out.push(big.encode().unwrap()).unwrap();
    out.push(small.encode().unwrap()).unwrap();
    let mut asm = Assembler::default();
    let mut got = Vec::new();
    let mut rounds = 0;
    while !out.is_empty() {
        let wires = out.take(20_000);
        assert!(wires.iter().map(Vec::len).sum::<usize>() <= 20_000);
        assert!(wires.iter().all(|w| w.len() <= MAX_WIRE));
        for w in wires {
            got.extend(asm.take(&w).unwrap());
        }
        rounds += 1;
    }
    assert_eq!(rounds, 4, "65 KiB in 20 000-byte budgets");
    assert_eq!(asm.held(), 0);
    let items: Vec<ScriptItem> = got.iter().map(|b| ScriptItem::decode(b, Direction::ToClient).unwrap()).collect();
    assert_eq!(items, vec![big, small]);
    assert!(Assembler::default().take(&[0, 0, 1, 5]).is_err());
    let mut a = Assembler::default();
    a.take(&[FIRST, 0, 1, 5]).unwrap();
    assert!(a.take(&[FIRST, 0, 1, 5]).is_err());
    assert!(Assembler::default().take(&[FIRST | LAST, 0, 0]).is_err());
    assert!(Assembler::default().take(&[4 | FIRST | LAST, 0, 1, 5]).is_err());
    let mut q = Outgoing::new(100);
    assert!(q.push(vec![1; 101]).is_err());
    q.push(vec![1; 60]).unwrap();
    assert_eq!(q.push(vec![1; 41]), Err(QueueFull { held: 60, cap: 100 }));
    let mut q = Outgoing::new(1 << 20);
    q.push(vec![7; 100]).unwrap();
    q.push(vec![8; 100]).unwrap();
    let first = q.take(50);
    q.clear();
    assert_eq!(q.held(), 53);
    let mut a = Assembler::default();
    assert!(a.take(&first[0]).unwrap().is_empty());
    let rest: Vec<Vec<u8>> = q.take(1000).iter().flat_map(|w| a.take(w).unwrap()).collect();
    assert_eq!(rest, vec![vec![7; 100]]);
}

#[test]
fn a_paced_item_waits_for_its_bytes_and_keeps_the_order() {
    let mut q = Outgoing::new(1 << 20);
    q.push(vec![1; 10]).unwrap();
    q.push_paced(vec![2; 3000], 3000).unwrap();
    q.push(vec![3; 10]).unwrap();
    let asked = std::cell::RefCell::new(Vec::new());
    let credit = std::cell::Cell::new(0usize);
    let mut pace = |n: usize| {
        asked.borrow_mut().push(n);
        let ok = credit.get() >= n;
        if ok {
            credit.set(credit.get() - n);
        }
        ok
    };
    let mut a = Assembler::default();
    let got: Vec<Vec<u8>> = q.take_paced(100_000, &mut pace).iter().flat_map(|w| a.take(w).unwrap()).collect();
    assert_eq!(got, vec![vec![1; 10]], "the paced item and the one behind it wait");
    credit.set(3000);
    let got: Vec<Vec<u8>> = q.take_paced(1000, &mut pace).iter().flat_map(|w| a.take(w).unwrap()).collect();
    assert!(got.is_empty(), "the paced item started within the budget");
    q.clear();
    let got: Vec<Vec<u8>> = q.take_paced(100_000, &mut pace).iter().flat_map(|w| a.take(w).unwrap()).collect();
    assert_eq!(got, vec![vec![2; 3000]], "its rest goes, the cleared item behind it is gone");
    assert_eq!(*asked.borrow(), vec![3000, 3000], "asked once while it waited, once when it went");
    assert!(q.is_empty());
}
