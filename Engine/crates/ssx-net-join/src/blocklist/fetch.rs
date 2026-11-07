use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use ssx_content::blocklist::{Answer, from_hex_list};
use ssx_net_proto::master::{Blocklist, MAX_BLOCKLIST_BYTES, MasterUrl};

use crate::browse::masters::{LIST_TIMEOUT, Roots};
use crate::browse::resolve::Lookup;

const MAX_ETAG: usize = ssx_content::blocklist::cache::MAX_ETAG;

pub fn fetch(
    url: &MasterUrl,
    etag: Option<&str>,
    roots: &Roots,
    cancel: Option<Arc<AtomicBool>>,
    lookup: Lookup,
) -> Result<Answer, String> {
    let target = format!("{url}/v1/blocklist");
    let mut req = crate::browse::masters::agent(url, roots, cancel, lookup).get(&target);
    if let Some(tag) = etag {
        req = req.header("if-none-match", tag);
    }
    let mut response = req.call().map_err(|e| match e {
        ureq::Error::Timeout(_) => format!("no answer within {} s", LIST_TIMEOUT.as_secs()),
        ureq::Error::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => "nothing answers there".into(),
        other => format!("cannot be reached: {other}"),
    })?;
    let status = response.status().as_u16();
    match status {
        304 if etag.is_some() => return Ok(Answer::NotModified),
        304 => return Err("answered 304 Not Modified to a request that named no list".into()),
        429 => return Err("asks to be asked less often (HTTP 429)".into()),
        200 => {}
        _ => return Err(format!("not a Project Avalanche master of this version (HTTP {status})")),
    }
    let tag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .filter(|t| !t.is_empty() && t.len() <= MAX_ETAG && t.bytes().all(|b| (0x21..=0x7E).contains(&b)))
        .map(str::to_string);
    let body =
        response.body_mut().with_config().limit(MAX_BLOCKLIST_BYTES as u64 + 1).read_to_vec().map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => format!("its blocklist is larger than {MAX_BLOCKLIST_BYTES} bytes"),
            ureq::Error::Timeout(_) => format!("no whole blocklist within {} s", LIST_TIMEOUT.as_secs()),
            e => format!("its blocklist does not read: {e}"),
        })?;
    let list = Blocklist::from_json(&body).map_err(|e| ssx_core::text::message(&e).to_string())?;
    let hashes = from_hex_list(&list.resources).map_err(|e| ssx_core::text::message(&e).to_string())?;
    Ok(Answer::List { hashes, etag: tag })
}
