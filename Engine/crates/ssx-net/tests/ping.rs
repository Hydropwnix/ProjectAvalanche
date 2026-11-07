#[test]
fn pings_are_the_servers_round_trips() {
    use core::time::Duration;
    let o =
        ssx_net::harness::ping::run_pings(Duration::from_millis(50), Duration::from_secs(3), Duration::from_secs(5))
            .unwrap_or_else(|e| panic!("{e}"));
    println!("{o:?}");
    for k in 0..2 {
        let first = o.first[k].unwrap_or_else(|| panic!("rider {k}'s ping never came: {o:?}"));
        assert!(first >= 100, "rider {k}'s first ping {first} ms is below the round trip: {o:?}");
        let (lo, hi) = o.settled[k].unwrap_or_else(|| panic!("rider {k}: no settled ping: {o:?}"));
        assert!(lo >= 100 && hi <= 100 + 4 * 17, "rider {k}'s pings {lo}…{hi} ms against a 100 ms round trip: {o:?}");
    }
    assert_eq!(o.lost, 0, "a ping taken away: {o:?}");
}
