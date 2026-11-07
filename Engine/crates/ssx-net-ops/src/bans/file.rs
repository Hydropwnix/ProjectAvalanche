use serde::Deserialize;
use ssx_net_proto::operator::{Expiry, utc};
use toml::Spanned;

use super::{Ban, BanStore, MAX_ENTRIES, Mute, Target};
use crate::range::Range;
use crate::rules::{FileError, by, expiry, player_id, reason, toml_string};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    ban: Option<Spanned<Vec<Spanned<RawBan>>>>,
    mute: Option<Spanned<Vec<Spanned<RawMute>>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBan {
    player: Option<Spanned<String>>,
    ip: Option<Spanned<String>>,
    range: Option<Spanned<String>>,
    reason: Spanned<String>,
    by: Spanned<String>,
    at: Spanned<i64>,
    until: Spanned<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMute {
    player: Spanned<String>,
    reason: Spanned<String>,
    by: Spanned<String>,
    at: Spanned<i64>,
    until: Spanned<i64>,
}

fn common(
    text: &str,
    r: &Spanned<String>,
    b: &Spanned<String>,
    at: &Spanned<i64>,
    until: &Spanned<i64>,
) -> Result<(String, String, u64, Expiry), FileError> {
    let err = |s: &std::ops::Range<usize>, m: String| FileError::at(text, s.start, m);
    reason(r.get_ref()).map_err(|m| err(&r.span(), m))?;
    by(b.get_ref()).map_err(|m| err(&b.span(), m))?;
    let when =
        u64::try_from(*at.get_ref()).map_err(|_| err(&at.span(), "at: a Unix time in seconds, 0 or more".into()))?;
    let end = expiry(*until.get_ref()).ok_or_else(|| err(&until.span(), "until: 0 (never) or a Unix time".into()))?;
    if let Expiry::At(t) = end
        && t.get() <= when
    {
        return Err(err(&until.span(), "until: after `at` (or 0: never)".into()));
    }
    Ok((r.get_ref().clone(), b.get_ref().clone(), when, end))
}

pub(super) fn parse(text: &str) -> Result<BanStore, FileError> {
    let raw: RawFile = toml::from_str(text)
        .map_err(|e| FileError::at(text, e.span().map_or(0, |s| s.start), e.message().to_string()))?;
    let mut store = BanStore::default();
    if let Some(list) = raw.ban {
        if list.get_ref().len() > MAX_ENTRIES {
            return Err(FileError::at(text, list.span().start, format!("more than {MAX_ENTRIES} bans")));
        }
        for entry in list.into_inner() {
            let span = entry.span();
            let b = entry.into_inner();
            let target = match (&b.player, &b.ip, &b.range) {
                (Some(p), None, None) => {
                    Target::Player(player_id(p.get_ref()).map_err(|m| FileError::at(text, p.span().start, m))?)
                }
                (None, Some(ip), None) => {
                    let r = Range::parse(ip.get_ref()).map_err(|m| FileError::at(text, ip.span().start, m))?;
                    if !r.is_address() || r.to_string() != *ip.get_ref() {
                        return Err(FileError::at(text, ip.span().start, format!("ip: one address as {r} writes it")));
                    }
                    Target::Address(r)
                }
                (None, None, Some(range)) => {
                    let r = Range::parse(range.get_ref()).map_err(|m| FileError::at(text, range.span().start, m))?;
                    if r.is_address() || r.to_string() != *range.get_ref() {
                        return Err(FileError::at(
                            text,
                            range.span().start,
                            format!("range: a range as {r} writes it"),
                        ));
                    }
                    Target::Address(r)
                }
                _ => return Err(FileError::at(text, span.start, "a ban names exactly one of player, ip, range")),
            };
            let (reason, by, at, until) = common(text, &b.reason, &b.by, &b.at, &b.until)?;
            if store.bans.iter().any(|o| o.target == target) {
                return Err(FileError::at(text, span.start, format!("{target} is banned twice")));
            }
            store.bans.push(Ban { target, reason, by, at, until });
        }
    }
    if let Some(list) = raw.mute {
        if list.get_ref().len() > MAX_ENTRIES {
            return Err(FileError::at(text, list.span().start, format!("more than {MAX_ENTRIES} mutes")));
        }
        for entry in list.into_inner() {
            let span = entry.span();
            let m = entry.into_inner();
            let player = player_id(m.player.get_ref()).map_err(|e| FileError::at(text, m.player.span().start, e))?;
            let (reason, by, at, until) = common(text, &m.reason, &m.by, &m.at, &m.until)?;
            if store.mutes.iter().any(|o| o.player == player) {
                return Err(FileError::at(text, span.start, "a player muted twice"));
            }
            store.mutes.push(Mute { player, reason, by, at, until });
        }
    }
    Ok(store)
}

const HEADER: &str = "\
# Project Avalanche: the server's bans and mutes (written by the server: the console's and RCON's ban, unban, mute,
# unmute). Edit it while the server is stopped, or run `bans reload` after an edit. Each ban names one of `player`
# (the player id, 64 hex digits), `ip` (one address) or `range` (192.0.2.0/24, 2001:db8::/32 or 192.0.2.*); `at` and
# `until` are Unix times in seconds (UTC) on the server's clock, `until = 0` never ends.
";

fn when(t: u64) -> String {
    utc(t)
}

pub(super) fn write(store: &BanStore) -> String {
    let mut out = String::from(HEADER);
    for b in &store.bans {
        out.push_str("\n[[ban]]\n");
        match b.target {
            Target::Player(id) => out.push_str(&format!("player = \"{}\"\n", ssx_net_proto::keys::to_hex(&id))),
            Target::Address(r) if r.is_address() => out.push_str(&format!("ip = \"{r}\"\n")),
            Target::Address(r) => out.push_str(&format!("range = \"{r}\"\n")),
        }
        entry(&mut out, &b.reason, &b.by, b.at, b.until);
    }
    for m in &store.mutes {
        out.push_str("\n[[mute]]\n");
        out.push_str(&format!("player = \"{}\"\n", ssx_net_proto::keys::to_hex(&m.player)));
        entry(&mut out, &m.reason, &m.by, m.at, m.until);
    }
    out
}

fn entry(out: &mut String, reason: &str, by: &str, at: u64, until: Expiry) {
    out.push_str(&format!("reason = {}\n", toml_string(reason)));
    out.push_str(&format!("by = {}\n", toml_string(by)));
    out.push_str(&format!("at = {at}  # {}\n", when(at)));
    match until {
        Expiry::Never => out.push_str("until = 0  # never\n"),
        Expiry::At(t) => out.push_str(&format!("until = {}  # {}\n", t.get(), when(t.get()))),
    }
}
