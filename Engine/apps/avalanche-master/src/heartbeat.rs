use std::net::IpAddr;
use std::time::Instant;

use ssx_net_proto::keys::{ServerKey, to_hex};
use ssx_net_proto::master::{Heartbeat, HeartbeatAnswer, HeartbeatEnvelope, MasterError, OneTimeSecret};
use tracing::{info, warn};

use crate::address::{address_key, canonical, never_a_server, not_public};
use crate::check::check;
use crate::http::Reply;
use crate::limits::Verdict;
use crate::state::{Master, Stats};

fn refused(master: &Master, status: u16, text: &str) -> Reply {
    Stats::count(&master.stats.refused);
    Reply::text(status, text)
}

fn answer(master: &Master, key: &ServerKey, secret: &OneTimeSecret, listed: bool, reason: &str) -> Reply {
    if !listed {
        Stats::count(&master.stats.refused);
    }
    match (HeartbeatAnswer { listed, reason: reason.to_string() }).seal(key, secret) {
        Ok(body) => Reply::octets(body),
        Err(e) => Reply::text(500, &format!("the answer: {e}")),
    }
}

fn short(key: &ServerKey) -> String {
    to_hex(key.as_bytes())[..16].to_string()
}

pub async fn heartbeat(master: &Master, peer: IpAddr, body: &[u8]) -> Reply {
    let now = Instant::now();
    let ip = canonical(peer);
    Stats::count(&master.stats.heartbeats);
    if never_a_server(ip) {
        return refused(master, 403, &format!("{ip} cannot be a server's address"));
    }
    if !master.config.allow_private
        && let Some(why) = not_public(ip)
    {
        return refused(
            master,
            403,
            &format!("this master lists public addresses only: the heartbeat came from {ip}, {why}"),
        );
    }
    match master.limits.heartbeats.check(now) {
        Verdict::Allowed => {}
        Verdict::Limited(wait) => return Reply::limited(wait).with_status(503),
        Verdict::Full => return Reply::text(503, "busy"),
    }
    let envelope = match HeartbeatEnvelope::read(body) {
        Ok(e) => e,
        Err(e) => return refused(master, 400, &e.to_string()),
    };
    let Ok(key) = ServerKey::from_bytes(envelope.server) else {
        return refused(master, 400, "the server key is a low-order point");
    };
    let secret = match master.challenges.check(&envelope.challenge, ip, now) {
        Ok(s) => s,
        Err(e) => return refused(master, 400, &e.to_string()),
    };
    let beat = match envelope.open(&key, &secret) {
        Ok(b) => Ok(b),
        Err(MasterError::Seal(_)) => {
            warn!("a heartbeat from {ip} for key {} does not open: forged or for another challenge", short(&key));
            return refused(master, 400, "the heartbeat does not open with its key and this master's challenge");
        }
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = master.challenges.redeem(&envelope.challenge, now) {
        return refused(
            master,
            if matches!(e, crate::challenge::ChallengeRefusal::Busy) { 503 } else { 400 },
            &e.to_string(),
        );
    }
    match master.limits.per_key.check(*key.as_bytes(), now) {
        Verdict::Allowed => {}
        Verdict::Limited(wait) => return Reply::limited(wait),
        Verdict::Full => return Reply::text(503, "busy"),
    }
    let beat = match beat {
        Ok(b) => b,
        Err(why) => return answer(master, &key, &secret, false, &format!("not listed: the heartbeat: {why}")),
    };
    decide(master, ip, key, &secret, beat, now).await
}

async fn decide(
    master: &Master,
    ip: IpAddr,
    key: ServerKey,
    secret: &OneTimeSecret,
    beat: Heartbeat,
    now: Instant,
) -> Reply {
    let k = *key.as_bytes();
    let blocked = master.blocklists.read().ok().and_then(|b| b.servers.get(&k).cloned());
    if let Some(reason) = blocked {
        let removed = master.registry.lock().ok().and_then(|mut r| r.remove(&k));
        if removed.is_some() {
            info!("took {} ({ip}) off the list: its key is blocked", short(&key));
        }
        let why = if reason.is_empty() { String::new() } else { format!(": {reason}") };
        return answer(
            master,
            &key,
            secret,
            false,
            &format!("not listed: this server's key is blocked by the master's operator{why}"),
        );
    }
    if beat.leaving {
        if master.registry.lock().ok().and_then(|mut r| r.remove(&k)).is_some() {
            info!("{:?} ({ip}:{}) left the list: the server stops", beat.name, beat.port);
        }
        return answer(master, &key, secret, false, "taken off the list (the server stops)");
    }
    let (needs_check, room) = match master.registry.lock() {
        Ok(r) => (r.needs_check(&k, ip, &beat, now), r.room_for(&k, ip, beat.port)),
        Err(_) => return Reply::text(503, "busy"),
    };
    if let Err(why) = room {
        return answer(master, &key, secret, false, &format!("not listed: {why}"));
    }
    if !needs_check {
        if let Ok(mut r) = master.registry.lock() {
            r.refresh(&k, &beat, now);
        }
        return answer(master, &key, secret, true, "listed");
    }
    match master.limits.checks.check(address_key(ip), now) {
        Verdict::Allowed => {}
        _ => {
            return answer(
                master,
                &key,
                secret,
                false,
                "not listed yet: too many checks from your address now; the next heartbeat tries again",
            );
        }
    }
    let Ok(Ok(_permit)) = tokio::time::timeout(crate::state::CHECK_WAIT, master.checks.acquire()).await else {
        return answer(
            master,
            &key,
            secret,
            false,
            "not listed yet: the master is checking many servers now; the next heartbeat tries again",
        );
    };
    match check(ip, &beat, &key).await {
        Ok(checked) => {
            let Ok(mut r) = master.registry.lock() else { return Reply::text(503, "busy") };
            if let Err(why) = r.room_for(&k, ip, beat.port) {
                return answer(master, &key, secret, false, &format!("not listed: {why}"));
            }
            let new = r.get(&k).is_none();
            let replaced = r.list(k, ip, beat.clone(), checked.info_hash, now);
            drop(r);
            for old in replaced {
                info!(
                    "took {} ({ip}:{}) off the list: the key {} answers there now",
                    &to_hex(&old.key)[..16],
                    old.beat.port,
                    short(&key)
                );
            }
            Stats::count(&master.stats.checks_passed);
            info!(
                "{} {:?} ({ip}:{}, key {}, query {:.1} ms, contact {:?})",
                if new { "listed" } else { "checked again" },
                beat.name,
                beat.port,
                short(&key),
                checked.query_rtt.as_secs_f64() * 1000.0,
                beat.contact
            );
            answer(master, &key, secret, true, "listed")
        }
        Err(why) => {
            Stats::count(&master.stats.checks_failed);
            let removed = master.registry.lock().ok().and_then(|mut r| r.remove(&k)).is_some();
            info!(
                "not listed: {:?} ({ip}:{}, key {}): {why}{}",
                beat.name,
                beat.port,
                short(&key),
                if removed { " (taken off)" } else { "" }
            );
            answer(master, &key, secret, false, &format!("not listed: {why}"))
        }
    }
}
