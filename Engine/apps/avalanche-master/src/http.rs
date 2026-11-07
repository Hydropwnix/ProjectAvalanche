use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{
    CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, HeaderName, HeaderValue, IF_NONE_MATCH, RETRY_AFTER,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use ssx_net_proto::master::MAX_HEARTBEAT_BODY;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;
use tracing::warn;

use crate::address::{address_key, canonical};
use crate::forwarded::{self, Source};
use crate::limits::Verdict;
use crate::state::{self, Master, Stats};

pub const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");
const LOGGED_HEADER: usize = 120;

pub struct Reply {
    status: u16,
    content_type: &'static str,
    body: Bytes,
    retry_after: Option<u64>,
    etag: Option<String>,
}

impl Reply {
    pub fn text(status: u16, text: &str) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8",
            body: Bytes::from(format!("{text}\n")),
            retry_after: None,
            etag: None,
        }
    }

    pub fn octets(body: Vec<u8>) -> Self {
        Self { status: 200, content_type: "application/octet-stream", body: body.into(), retry_after: None, etag: None }
    }

    pub fn limited(wait: Duration) -> Self {
        Self { retry_after: Some(wait.as_secs() + 1), ..Self::text(429, "too many requests") }
    }

    pub fn with_status(self, status: u16) -> Self {
        Self { status, ..self }
    }

    fn json(body: Bytes, etag: String, if_none_match: Option<&HeaderValue>, master: &Master) -> Self {
        let held = if_none_match.and_then(|v| v.to_str().ok()).is_some_and(|v| {
            v.split(',').map(str::trim).any(|t| t == "*" || t == etag || t.strip_prefix("W/") == Some(etag.as_str()))
        });
        if held {
            Stats::count(&master.stats.not_modified);
            return Self {
                status: 304,
                content_type: "application/json",
                body: Bytes::new(),
                retry_after: None,
                etag: Some(etag),
            };
        }
        Stats::count(&master.stats.lists_served);
        Self { status: 200, content_type: "application/json", body, retry_after: None, etag: Some(etag) }
    }

    fn into_response(self) -> Response<Full<Bytes>> {
        let mut response = Response::new(Full::new(self.body));
        *response.status_mut() = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(self.content_type));
        match self.etag.as_deref().and_then(|e| HeaderValue::from_str(e).ok()) {
            Some(etag) => {
                headers.insert(ETAG, etag);
                headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            None => {
                headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            }
        }
        if let Some(secs) = self.retry_after {
            headers.insert(RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

fn endpoint(method: &Method, path: &str) -> usize {
    match (method, path) {
        (&Method::GET, "/v1/challenge") => state::CHALLENGE,
        (&Method::POST, "/v1/heartbeat") => state::HEARTBEAT,
        (&Method::GET, "/v1/servers") => state::SERVERS,
        (&Method::GET, "/v1/blocklist") => state::BLOCKLIST,
        _ => state::OTHER,
    }
}

pub fn protocol_query(query: Option<&str>) -> Option<u32> {
    let n = query?.strip_prefix("protocol=")?;
    let ok =
        !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit()) && (n == "0" || !n.starts_with('0'));
    ok.then(|| n.parse().ok()).flatten()
}

fn log_refused(peer: SocketAddr, refusal: forwarded::Refusal, lines: hyper::header::GetAll<'_, HeaderValue>) {
    let (mut count, mut bytes, mut shown) = (0usize, 0usize, String::new());
    for value in lines.iter() {
        count += 1;
        bytes += value.len();
        if shown.len() < LOGGED_HEADER {
            if count > 1 {
                shown.push_str(" | ");
            }
            let part = value.as_bytes().get(..value.len().min(LOGGED_HEADER)).unwrap_or_default();
            shown.push_str(&String::from_utf8_lossy(part));
        }
    }
    warn!(
        "refused a request from the trusted proxy {}: {refusal} (X-Forwarded-For: lines {count}, bytes {bytes}, {})",
        canonical(peer.ip()),
        ssx_core::text::quoted_max(&shown, LOGGED_HEADER)
    );
}

fn health(master: &Master) -> Reply {
    if master.stopping.load(Ordering::Acquire) {
        return Reply::text(503, "stopping");
    }
    if master.registry.lock().is_err() || master.blocklists.read().is_err() {
        return Reply::text(503, "a table is unusable (a panic held its lock): restart the master");
    }
    Reply::text(200, "ok")
}

async fn handle(master: &Master, peer: SocketAddr, request: Request<Incoming>) -> Reply {
    Stats::count(&master.stats.requests);
    let now = Instant::now();
    let lines = request.headers().get_all(X_FORWARDED_FOR);
    let source = match forwarded::source(peer.ip(), &master.config.trusted, lines.iter().map(HeaderValue::as_bytes)) {
        Ok(s) => s,
        Err(forwarded::Refusal::Missing) if request.method() == Method::GET && request.uri().path() == "/v1/health" => {
            Source::Direct(peer.ip().to_canonical())
        }
        Err(refusal) => {
            Stats::count(&master.stats.forwarded_refused);
            log_refused(peer, refusal, lines);
            return Reply::text(400, &refusal.to_string());
        }
    };
    let _client_slot = match source {
        Source::Direct(_) => None,
        Source::Proxied { client, .. } => {
            Stats::count(&master.stats.proxied);
            let Some(slot) = master.limits.connections.client(address_key(client)) else {
                Stats::count(&master.stats.limited);
                return Reply {
                    retry_after: Some(1),
                    ..Reply::text(429, "too many requests at once from your address")
                };
            };
            Some(slot)
        }
    };
    let client = source.client();
    let (method, path) = (request.method().clone(), request.uri().path().to_string());
    match master.limits.endpoint[endpoint(&method, &path)].check(address_key(client), now) {
        Verdict::Allowed => {}
        Verdict::Limited(wait) => {
            Stats::count(&master.stats.limited);
            return Reply::limited(wait);
        }
        Verdict::Full => {
            Stats::count(&master.stats.limited);
            return Reply::text(503, "busy");
        }
    }
    match (method, path.as_str()) {
        (Method::GET, "/v1/challenge") => Reply::octets(master.challenges.issue(client, now).to_bytes().to_vec()),
        (Method::POST, "/v1/heartbeat") => {
            let declared = request.headers().get(CONTENT_LENGTH).and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
            if declared.is_some_and(|n| n > MAX_HEARTBEAT_BODY as u64) {
                Stats::count(&master.stats.limited);
                return Reply::text(413, "the body is too large");
            }
            match Limited::new(request.into_body(), MAX_HEARTBEAT_BODY).collect().await {
                Ok(body) => crate::heartbeat::heartbeat(master, client, &body.to_bytes()).await,
                Err(_) => {
                    Stats::count(&master.stats.limited);
                    Reply::text(413, "the body is too large or ended early")
                }
            }
        }
        (Method::GET, "/v1/servers") => {
            let Some(protocol) = protocol_query(request.uri().query()) else {
                return Reply::text(400, "the query is protocol=<number>");
            };
            let Ok(mut registry) = master.registry.lock() else { return Reply::text(503, "busy") };
            let (body, etag) = registry.body(protocol, now);
            drop(registry);
            Reply::json(body, etag, request.headers().get(IF_NONE_MATCH), master)
        }
        (Method::GET, "/v1/blocklist") => {
            let Ok(lists) = master.blocklists.read() else { return Reply::text(503, "busy") };
            let (body, etag) = (lists.published.clone(), lists.etag.clone());
            drop(lists);
            Reply::json(body, etag, request.headers().get(IF_NONE_MATCH), master)
        }
        (Method::GET, "/v1/health") => health(master),
        (_, "/v1/challenge" | "/v1/servers" | "/v1/blocklist" | "/v1/health") => Reply::text(405, "GET only"),
        (_, "/v1/heartbeat") => Reply::text(405, "POST only"),
        _ => Reply::text(404, "not found"),
    }
}

async fn serve_io<I>(master: Arc<Master>, io: I, peer: SocketAddr)
where
    I: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let service = service_fn(|request| {
        let master = Arc::clone(&master);
        async move { Ok::<_, Infallible>(handle(&master, peer, request).await.into_response()) }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(state::HEAD_TIMEOUT)
        .max_buf_size(state::MAX_HEAD_BYTES)
        .max_headers(state::MAX_HEADERS)
        .keep_alive(false);
    let _ = builder.serve_connection(io, service).await;
}

async fn serve(master: Arc<Master>, stream: tokio::net::TcpStream, peer: SocketAddr, tls: Option<TlsAcceptor>) {
    let connections = &master.limits.connections;
    let trusted = master.config.trusted.contains(peer.ip());
    let (direct, proxied) =
        if trusted { (None, connections.open_proxied()) } else { (connections.open(address_key(peer.ip())), None) };
    if direct.is_none() && proxied.is_none() {
        Stats::count(&master.stats.limited);
        if trusted {
            Stats::count(&master.stats.proxy_pool_full);
        }
        return;
    }
    let work = async {
        match tls {
            Some(acceptor) => match tokio::time::timeout(state::HEAD_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(s)) => serve_io(Arc::clone(&master), TokioIo::new(s), peer).await,
                _ => Stats::count(&master.stats.limited),
            },
            None => serve_io(Arc::clone(&master), TokioIo::new(stream), peer).await,
        }
    };
    if tokio::time::timeout(state::CONNECTION_TIMEOUT, work).await.is_err() {
        Stats::count(&master.stats.limited);
    }
}

pub async fn run(
    listener: TcpListener,
    master: Arc<Master>,
    tls: Option<Arc<crate::tls::Slot>>,
    shutdown: Arc<Notify>,
) {
    loop {
        tokio::select! {
            () = shutdown.notified() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    tokio::spawn(serve(Arc::clone(&master), stream, peer, tls.as_ref().map(|s| s.current())));
                }
                Err(e) => {
                    warn!("accept: {e}");
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            },
        }
    }
    drop(listener);
    let until = Instant::now() + Duration::from_secs(2);
    while master.limits.connections.open_now() > 0 && Instant::now() < until {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_query_is_strict() {
        assert_eq!(protocol_query(Some("protocol=4")), Some(4));
        assert_eq!(protocol_query(Some("protocol=0")), Some(0));
        assert_eq!(protocol_query(Some("protocol=4294967295")), Some(u32::MAX));
        for bad in [
            None,
            Some(""),
            Some("protocol="),
            Some("protocol=04"),
            Some("protocol=4&x=1"),
            Some("protocol=-1"),
            Some("protocol=4294967296"),
            Some("x=1&protocol=4"),
            Some("PROTOCOL=4"),
            Some("protocol=4 "),
        ] {
            assert_eq!(protocol_query(bad), None, "{bad:?}");
        }
    }
}
