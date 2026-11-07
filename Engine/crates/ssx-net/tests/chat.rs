use ssx_net::client::{ChatBody, ChatFrom, KEPT_LINES};
use ssx_net::harness::chat::{ChatCase, ChatOutcome, run_chat};
use ssx_net::server::KickReason;
use ssx_net_proto::{ChatRefusal, ClientChat, GoodbyeReason, RiderId};

fn run(case: ChatCase) -> ChatOutcome {
    let o = run_chat(case).unwrap_or_else(|e| panic!("{case:?}: {e}"));
    println!(
        "{case:?}: kicks {:?}, refusal {:?}, goodbye {:?}, refusals {:?}, inbox {:?}, notes {:?}, lines {:?}",
        o.kicks, o.refusal, o.goodbye, o.refusals, o.inbox, o.notes, o.lines
    );
    o
}

fn kicked(case: ChatCase, reason: KickReason) {
    let o = run(case);
    assert_eq!(o.kicks, vec![reason], "{case:?}");
    assert!(!o.rider0_connected && o.rider1_connected, "{case:?}");
    assert!(o.lines[1].is_empty(), "{case:?}: nothing passed on");
}

fn left(case: ChatCase, why: &str) {
    let o = run(case);
    assert!(o.refusal.clone().unwrap_or_default().contains(why), "{case:?}: the reason names {why:?}");
    assert!(!o.rider0_connected && o.rider1_connected, "{case:?}");
    assert!(o.kicks.is_empty(), "{case:?}");
}

#[test]
fn a_conversation_reaches_everyone_in_the_servers_order_and_is_logged() {
    let o = run(ChatCase::Conversation);
    assert!(o.rider0_connected && o.rider1_connected && o.kicks.is_empty());
    for c in 0..2 {
        let lines: Vec<_> = o.lines[c].iter().map(|(f, b)| (*f, b.clone())).collect();
        assert_eq!(lines.len(), 3, "rider {c}: {lines:?}");
        assert!(matches!(lines[0], (ChatFrom::Rider(_), ChatBody::Text(ref t)) if t == "hello <b>&</b> all"));
        assert!(matches!(lines[1], (ChatFrom::Rider(_), ChatBody::Quick(3))));
        assert!(matches!(lines[2], (ChatFrom::Server, ChatBody::Text(ref t)) if t == "the server speaks"));
        assert_eq!(o.chat_flags[c], Some(true), "the welcome says the chat is on");
    }
    let sender = |c: usize| match o.lines[c][0].0 {
        ChatFrom::Rider(r) => r,
        ChatFrom::Server => RiderId(u16::MAX),
    };
    assert_eq!(sender(0), sender(1));
    assert_eq!(o.log.len(), 2, "the operator's log: the two passed lines");
    assert!(o.log.iter().all(|l| l.rider == sender(0).0 && l.key_id != [0; 8]));
    assert_eq!(o.log[0].line, ClientChat::Text("hello <b>&</b> all".into()));
    assert_eq!(o.log[1].line, ClientChat::Quick(3));
    assert!(o.refusals.is_empty());
}

#[test]
fn a_short_flood_is_dropped_with_one_notice() {
    let o = run(ChatCase::ShortFlood);
    assert!(o.rider0_connected && o.rider1_connected && o.kicks.is_empty(), "no one is sent away");
    assert_eq!(o.lines[1].len(), 5, "the burst passes: {:?}", o.lines[1]);
    assert_eq!(o.stats.chat_dropped, 3);
    assert_eq!(o.refusals.len(), 1, "one notice for the three: {:?}", o.refusals);
    assert!(matches!(o.refusals[0], ChatRefusal::Flood { wait_ms } if (900..=1000).contains(&wait_ms)));
}

#[test]
fn a_sustained_flood_sends_its_rider_away_with_the_reason() {
    let o = run(ChatCase::SustainedFlood);
    assert_eq!(o.lines[1].len(), 5, "the burst passes");
    assert_eq!(o.stats.chat_dropped, 10, "ten dropped before the flood allowance ends");
    assert_eq!(o.stats.chat_floods, 1);
    assert_eq!(o.kicks, vec![KickReason::ChatFlood]);
    assert_eq!(o.goodbye, Some(GoodbyeReason::ChatFlood), "the player is told");
    assert!(!o.rider0_connected && o.rider1_connected);
}

#[test]
fn a_closed_chat_is_said_in_the_welcome_and_refuses_every_line() {
    let o = run(ChatCase::Disabled);
    assert_eq!(o.chat_flags, [Some(false), Some(false)]);
    assert!(o.lines[0].is_empty() && o.lines[1].is_empty(), "passed to nobody");
    assert_eq!(o.refusals, vec![ChatRefusal::Disabled]);
    assert!(o.rider0_connected && o.kicks.is_empty() && o.log.is_empty());
}

#[test]
fn lines_a_correct_client_cannot_send_are_kicked() {
    kicked(ChatCase::InvalidUtf8, KickReason::MalformedChat);
    kicked(ChatCase::ControlCharacter, KickReason::MalformedChat);
    kicked(ChatCase::OverCharacters, KickReason::MalformedChat);
    kicked(ChatCase::OverWire, KickReason::MalformedChat);
    kicked(ChatCase::UnknownQuickLine, KickReason::MalformedChat);
    kicked(ChatCase::Huge, KickReason::MalformedPacket);
    kicked(ChatCase::WrongDirection, KickReason::WrongDirection);
}

#[test]
fn a_hostile_servers_chat_makes_the_client_leave_without_a_crash() {
    left(ChatCase::ServerMalformed, "malformed chat message");
    left(ChatCase::ServerControlCharacters, "malformed chat message");
    left(ChatCase::ServerOversized, "malformed chat message");
    left(ChatCase::ServerWrongDirection, "client-to-server chat line");
}

#[test]
fn a_line_of_an_unknown_rider_is_dropped_and_counted() {
    let o = run(ChatCase::ServerUnknownRider);
    assert!(o.rider0_connected && o.refusal.is_none());
    assert_eq!(o.inbox.1, 1, "one line of an unknown rider");
    assert!(o.lines[0].is_empty());
}

#[test]
fn ten_thousand_lines_a_second_leave_the_client_connected_with_a_bounded_inbox() {
    let o = run(ChatCase::ServerFlood);
    assert!(o.rider0_connected && o.rider1_connected && o.refusal.is_none(), "{:?}", o.refusal);
    let (received, _, overflowed, kept) = o.inbox;
    assert!(received >= 25_000, "the lines arrived: {received}");
    assert_eq!(kept, KEPT_LINES, "the newest kept, no more");
    assert_eq!(overflowed, received - KEPT_LINES as u64);
}
