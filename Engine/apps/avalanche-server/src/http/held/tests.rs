use std::time::{Duration, Instant};

use ssx_net::address::AddressKey;
use ssx_net_proto::join::HeldAsk;
use ssx_net_proto::keys::ReplySecret;
use tokio::sync::{oneshot, watch};

use super::{HeldTickets, MAX_HELD_PER_ADDRESS, MAX_TICKETS, Refused, TICKET_WINDOW, Waiting};
use crate::bridge::{HeldJoin, JoinVerdict};

fn waiting(text: &str) -> (Waiting, watch::Sender<String>, oneshot::Sender<JoinVerdict>) {
    let (text_tx, text) = watch::channel(text.to_string());
    let (verdict_tx, verdict) = oneshot::channel();
    (
        Waiting { hold: HeldJoin { text, verdict }, since: Instant::now(), who: "\"Mac\" (abcd)".into() },
        text_tx,
        verdict_tx,
    )
}

const HOME: AddressKey = AddressKey::V4([203, 0, 113, 9]);
const ELSEWHERE: AddressKey = AddressKey::V4([198, 51, 100, 7]);

#[test]
fn a_ticket_is_good_once_from_its_address_with_its_key_in_its_window() {
    let tickets = HeldTickets::default();
    let reply = ReplySecret::generate().public();
    let now = Instant::now();
    let (w, _text, _verdict) = waiting("the board looks you up\u{202E}");
    let Ok(hold) = tickets.issue(HOME, reply, w, now) else { panic!("issued") };
    assert_eq!(hold.text, "the board looks you up", "hidden characters dropped");
    assert_eq!(hold.ask_after_ms, super::ASK_AFTER_MS);
    let ask = HeldAsk { ticket: hold.ticket, reply };
    assert!(tickets.take(&ask, ELSEWHERE, now).is_none(), "another address");
    let other = HeldAsk { ticket: hold.ticket, reply: ReplySecret::generate().public() };
    assert!(tickets.take(&other, HOME, now).is_none(), "another reply key");
    let mut forged = hold.ticket;
    forged[31] ^= 1;
    assert!(tickets.take(&HeldAsk { ticket: forged, reply }, HOME, now).is_none(), "a ticket's last byte changed");
    assert!(tickets.take(&ask, HOME, now + TICKET_WINDOW + Duration::from_millis(1)).is_none(), "late");
    assert!(tickets.take(&ask, HOME, now + TICKET_WINDOW).is_some(), "on time, from home, with its key");
    assert!(tickets.take(&ask, HOME, now).is_none(), "used");
    assert_eq!(tickets.out_now(), 0);
}

#[test]
fn tickets_are_bounded_per_address_and_in_all_and_a_late_one_gives_its_hold_up() {
    let tickets = HeldTickets::default();
    let reply = ReplySecret::generate().public();
    let now = Instant::now();
    let mut ends = Vec::new();
    for _ in 0..MAX_HELD_PER_ADDRESS {
        let (w, text, verdict) = waiting("");
        assert!(tickets.issue(HOME, reply, w, now).is_ok());
        ends.push((text, verdict));
    }
    let (w, _t, _v) = waiting("");
    let Err((why, _back)) = tickets.issue(HOME, reply, w, now) else { panic!("refused") };
    assert_eq!(why, Refused::Address);
    for i in MAX_HELD_PER_ADDRESS..MAX_TICKETS {
        let (w, text, verdict) = waiting("");
        let address = AddressKey::V4([10, 0, (i >> 8) as u8, i as u8]);
        assert!(tickets.issue(address, reply, w, now).is_ok());
        ends.push((text, verdict));
    }
    let (w, _t, _v) = waiting("");
    assert!(matches!(tickets.issue(ELSEWHERE, reply, w, now), Err((Refused::Full, _))));
    assert!(ends.iter().all(|(_, v)| !v.is_closed()), "the holds are kept while their tickets are out");
    tickets.sweep(now + TICKET_WINDOW + Duration::from_millis(1));
    assert_eq!(tickets.out_now(), 0);
    assert!(ends.iter().all(|(t, v)| v.is_closed() && t.is_closed()), "the world sees every hold given up");
}
