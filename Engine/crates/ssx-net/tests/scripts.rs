use ssx_net::harness::scripts::{ScriptCase, ScriptOutcome, run_scripts};
use ssx_net::server::KickReason;
use ssx_net_proto::GoodbyeReason;
use ssx_net_proto::scripts::{MAX_ARGS, ScriptBody};

fn run(case: ScriptCase) -> ScriptOutcome {
    let o = run_scripts(case).unwrap_or_else(|e| panic!("{case:?}: {e}"));
    println!(
        "{case:?}: kicks {:?}, refusal {:?}, goodbye {:?}, stats {:?}, server got {}, client got {}, notes {:?}",
        o.kicks,
        o.refusal,
        o.goodbye,
        o.stats,
        o.server_got.len(),
        o.client_got.len(),
        o.notes
    );
    o
}

fn names(o: &ScriptOutcome) -> Vec<String> {
    o.client_got
        .iter()
        .map(|i| match &i.body {
            ScriptBody::Event { name, .. } => name.clone(),
            b => format!("{b:?}"),
        })
        .collect()
}

#[test]
fn items_cross_both_ways_in_order_and_whole() {
    let o = run(ScriptCase::RoundTrip);
    assert!(o.client0_connected && o.client1_connected && o.kicks.is_empty() && o.refusal.is_none());
    assert_eq!((o.server_fragment_bytes, o.client_fragment_bytes), (0, 0));
    assert_eq!(names(&o), ["x", "y", "z"]);
    let got: Vec<&ScriptBody> = o.server_got.iter().map(|(_, i)| &i.body).collect();
    assert_eq!(got.len(), 3);
    assert!(matches!(got[1], ScriptBody::Event { args, .. } if args.len() == MAX_ARGS));
    let client = o.server_got[0].0;
    assert!(o.server_got.iter().all(|(c, _)| *c == client), "one client's items under its id");
    assert_eq!(o.joined.len(), 2, "both sessions told: {:?}", o.joined);
    assert!(o.joined.iter().any(|(c, _)| *c == client));
    assert_eq!(o.stats.dropped + o.stats.malformed + o.stats.floods, 0);
}

#[test]
fn a_burst_past_the_event_bucket_is_dropped_and_counted() {
    let o = run(ScriptCase::Burst);
    assert!(o.client0_connected && o.kicks.is_empty() && o.goodbye.is_none());
    assert_eq!(o.server_got.len(), 200, "the burst passes");
    assert_eq!(o.stats.dropped, 60);
}

#[test]
fn a_flood_sends_its_client_away_with_the_reason() {
    let o = run(ScriptCase::Flood);
    assert_eq!(o.goodbye, Some(GoodbyeReason::ScriptFlood));
    assert_eq!(o.stats.floods, 1);
    assert!(!o.client0_connected && o.client1_connected);
    assert!(o.kicks.contains(&KickReason::ScriptFlood), "{:?}", o.kicks);
}

#[test]
fn what_a_correct_client_never_sends_ends_its_connection() {
    for case in [ScriptCase::Malformed, ScriptCase::ServerKind] {
        let o = run(case);
        assert_eq!(o.kicks, vec![KickReason::MalformedScript], "{case:?}");
        assert!(!o.client0_connected && o.client1_connected, "{case:?}");
        assert_eq!(o.stats.malformed, 1, "{case:?}");
        assert!(o.server_got.is_empty(), "{case:?}");
    }
}

#[test]
fn what_a_correct_server_never_sends_makes_the_client_leave() {
    for (case, why) in
        [(ScriptCase::ServerMalformed, "malformed script message"), (ScriptCase::ClientKind, "script item")]
    {
        let o = run(case);
        assert!(o.refusal.as_deref().unwrap_or_default().contains(why), "{case:?}: {:?}", o.refusal);
        assert!(!o.client0_connected && o.client1_connected, "{case:?}");
        assert!(o.kicks.is_empty(), "{case:?}");
    }
}

#[test]
fn a_backlog_arrives_in_order_paced_without_a_kick() {
    let o = run(ScriptCase::Backlog);
    assert!(o.client0_connected && o.kicks.is_empty() && o.refusal.is_none(), "{:?}: {:?}", o.notes, o.refusal);
    assert_eq!(o.client_fragment_bytes, 0);
    assert_eq!(o.client_got.len(), 64);
    let firsts: Vec<u8> = o
        .client_got
        .iter()
        .map(|i| match &i.body {
            ScriptBody::Event { args, .. } => args[0],
            _ => 0,
        })
        .collect();
    assert_eq!(firsts, (0..64u32).map(|k| (k % 251) as u8).collect::<Vec<_>>());
}
