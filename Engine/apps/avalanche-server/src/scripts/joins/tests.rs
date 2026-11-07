use std::time::Duration;

use ssx_net::server::TicketRequest;
use ssx_net_proto::join::RefusalCode;
use ssx_net_proto::keys::PlayerSecret;
use tokio::sync::{mpsc, oneshot};

use super::{DEFER_BOUND, FIRST_WORD, HeldJoins, next_join};
use crate::bridge::{JoinQueue, JoinTask, JoinVerdict};

fn task(name: &str) -> (JoinTask, oneshot::Receiver<JoinVerdict>) {
    let ticket = TicketRequest {
        player: PlayerSecret::generate().public(),
        name: name.into(),
        address: "203.0.113.9".parse().unwrap(),
        public_addr: "198.51.100.1:27450".parse().unwrap(),
        manifest: None,
    };
    let (reply, rx) = oneshot::channel();
    (JoinTask { ticket, reply }, rx)
}

fn asked(h: &mut HeldJoins, names: &[&str], now: Duration) -> Vec<oneshot::Receiver<JoinVerdict>> {
    let (tx, rx) = mpsc::channel(64);
    let mut queue = JoinQueue(rx);
    let ends = names
        .iter()
        .map(|n| {
            let (t, end) = task(n);
            assert!(tx.try_send(t).is_ok());
            end
        })
        .collect();
    assert!(next_join(&mut queue, Some(h), now).is_none(), "every join is put to the scripts");
    ends
}

#[test]
fn a_held_join_is_answered_held_followed_and_given_up_when_its_client_goes() {
    let mut h = HeldJoins { asking: true, ..Default::default() };
    let now = Duration::from_secs(10);
    let mut ends = asked(&mut h, &["Elise", "Mac", "Kaori"], now);
    let ids: Vec<u64> = h.to_ask.iter().map(|(id, ..)| *id).collect();
    for id in &ids {
        h.hold(*id, "the board looks you up\u{200B}", now);
    }
    assert_eq!(h.held_answered, 3);
    let mut held: Vec<_> = ends
        .iter_mut()
        .map(|e| match e.try_recv() {
            Ok(JoinVerdict::Held(hold)) => hold,
            _ => panic!("answered held"),
        })
        .collect();
    assert_eq!(*held[0].text.borrow(), "the board looks you up", "made safe");
    h.hold(ids[0], "almost there", now);
    assert!(held[0].text.has_changed().unwrap());
    assert_eq!(*held[0].text.borrow_and_update(), "almost there");
    h.hold(ids[0], "almost there", now);
    assert!(!held[0].text.has_changed().unwrap(), "the same text is not sent again");
    h.decide(ids[0], Ok(()));
    let (tx, rx) = mpsc::channel(1);
    drop(tx);
    let ready = next_join(&mut JoinQueue(rx), Some(&mut h), now).expect("let through to the slots");
    assert_eq!(ready.ticket.name, "Elise");
    assert!(ready.reply.send(JoinVerdict::Refused(RefusalCode::Full, "full".into())).is_ok());
    assert!(matches!(held[0].verdict.try_recv(), Ok(JoinVerdict::Refused(RefusalCode::Full, _))), "its own end");
    h.decide(ids[1], Err("not on the list".into()));
    assert!(
        matches!(held[1].verdict.try_recv(), Ok(JoinVerdict::Refused(RefusalCode::Scripts, t)) if t == "not on the list")
    );
    let kaori = held.pop().unwrap();
    drop(kaori);
    h.expire(now);
    assert_eq!((h.given_up, h.held(), h.forgotten.as_slice()), (1, 0, &ids[2..]));
}

#[test]
fn late_words_are_refused_as_busy_and_a_gone_request_is_given_up() {
    let mut h = HeldJoins { asking: true, ..Default::default() };
    let now = Duration::from_secs(10);
    let mut ends = asked(&mut h, &["Elise", "Mac", "Kaori"], now);
    let ids: Vec<u64> = h.to_ask.iter().map(|(id, ..)| *id).collect();
    h.hold(ids[1], "", now);
    let Ok(JoinVerdict::Held(mut mac)) = ends[1].try_recv() else { panic!("held") };
    drop(ends.pop());
    h.hold(ids[2], "", now);
    assert_eq!((h.given_up, h.forgotten.as_slice()), (1, &ids[2..]));
    h.expire(now + FIRST_WORD + Duration::from_millis(1));
    assert!(matches!(ends[0].try_recv(), Ok(JoinVerdict::Refused(RefusalCode::Busy, _))), "no first word");
    assert_eq!(h.held(), 1, "the held join waits on");
    h.expire(now + DEFER_BOUND + Duration::from_secs(2) + Duration::from_millis(1));
    assert!(matches!(mac.verdict.try_recv(), Ok(JoinVerdict::Refused(RefusalCode::Busy, _))), "not decided");
    assert_eq!((h.held(), h.refused), (0, 2));
    assert_eq!(h.forgotten, vec![ids[2], ids[0], ids[1]]);
}
