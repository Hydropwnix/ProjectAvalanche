use std::time::Instant;

use ssx_net_proto::master::{MAX_HEARTBEAT_BODY, ServerList};

use crate::common::*;

#[test]
fn rates_per_address_and_per_key() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let flood = next_ip();
    let codes: Vec<u16> =
        (0..40).map(|_| rt.block_on(exchange(addr, flood, "GET", "/v1/challenge", vec![], &[])).0).collect();
    assert_eq!(codes.iter().filter(|&&c| c == 200).count(), 32, "{codes:?}");
    assert!(codes[32..].iter().all(|&c| c == 429));
    assert_eq!(rt.block_on(exchange(addr, next_ip(), "GET", "/v1/challenge", vec![], &[])).0, 200);
    let fake = Fake::start("Rate", Query::Honest, true);
    let got: Vec<String> = (0..6)
        .map(|_| match rt.block_on(heartbeat(addr, &fake, |_| {})) {
            Got::Answer(a) => format!("listed {}", a.listed),
            Got::Plain(s, _) => s.to_string(),
        })
        .collect();
    assert_eq!(got, ["listed true", "listed true", "listed true", "listed true", "429", "429"]);
    m.stop();
    m.join().expect("stops");
}

#[test]
fn bodies_heads_and_queries_are_bounded() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let ip = next_ip();
    let big = vec![0u8; MAX_HEARTBEAT_BODY + 1];
    assert_eq!(rt.block_on(exchange(addr, ip, "POST", "/v1/heartbeat", big, &[])).0, 413);
    assert_eq!(rt.block_on(exchange(addr, ip, "POST", "/v1/heartbeat", vec![0; 10], &[])).0, 400);
    let long = "x".repeat(9000);
    assert_eq!(
        rt.block_on(exchange(addr, next_ip(), "GET", "/v1/servers?protocol=4", vec![], &[("x-long", &long)])).0,
        431
    );
    for q in
        ["/v1/servers", "/v1/servers?protocol=04", "/v1/servers?protocol=4&x=1", "/v1/servers?protocol=99999999999"]
    {
        assert_eq!(rt.block_on(exchange(addr, next_ip(), "GET", q, vec![], &[])).0, 400, "{q}");
    }
    assert_eq!(rt.block_on(exchange(addr, next_ip(), "POST", "/v1/servers?protocol=4", vec![], &[])).0, 405);
    assert_eq!(rt.block_on(exchange(addr, next_ip(), "GET", "/v1/heartbeat", vec![], &[])).0, 405);
    assert_eq!(rt.block_on(exchange(addr, next_ip(), "GET", "/admin", vec![], &[])).0, 404);
    let t0 = Instant::now();
    let slow = {
        use std::io::{Read, Write};
        let stream = rt.block_on(async {
            let s = tokio::net::TcpSocket::new_v4().expect("socket");
            s.bind(std::net::SocketAddr::new(next_ip(), 0)).expect("bind");
            s.connect(addr).await.expect("connect").into_std().expect("std")
        });
        stream.set_nonblocking(false).expect("blocking");
        stream.set_read_timeout(Some(20 * SECOND)).expect("timeout");
        let mut stream = stream;
        stream.write_all(b"GET /v1/servers?protocol=4 HTTP/1.1\r\nHost: m\r\n").expect("write");
        let mut buf = Vec::new();
        let _ = stream.read_to_end(&mut buf);
        buf
    };
    let cut = t0.elapsed();
    assert!(cut >= 4 * SECOND && cut < 8 * SECOND, "cut after {cut:?} ({} bytes)", slow.len());
    m.stop();
    m.join().expect("stops");
}

#[test]
fn the_list_carries_an_etag_and_answers_304() {
    let m = master_with(|_| {});
    let addr = m.started.addr;
    let rt = runtime();
    let (status, etag, body) = rt.block_on(exchange(addr, next_ip(), "GET", "/v1/servers?protocol=4", vec![], &[]));
    assert_eq!(status, 200);
    assert!(ServerList::from_json(&body, 4).expect("reads").servers.is_empty());
    let etag = etag.expect("an ETag");
    let (status, again, body) =
        rt.block_on(exchange(addr, next_ip(), "GET", "/v1/servers?protocol=4", vec![], &[("if-none-match", &etag)]));
    assert_eq!((status, again.as_deref(), body.len()), (304, Some(etag.as_str()), 0));
    let (status, _, _) = rt.block_on(exchange(addr, next_ip(), "GET", "/v1/blocklist", vec![], &[]));
    assert_eq!(status, 200);
    m.stop();
    m.join().expect("stops");
}
