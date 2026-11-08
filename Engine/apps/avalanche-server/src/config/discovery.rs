use std::ops::Range;

use serde::Deserialize;
use ssx_net_proto::master::{MasterUrl, the_master};
use toml::Spanned;

use super::ConfigError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    pub public: bool,
    pub master: Option<MasterUrl>,
    pub region: String,
    pub tags: Vec<String>,
    pub contact: String,
    pub query: bool,
    pub lan: bool,
    pub blocklist: Option<std::path::PathBuf>,
}

impl Default for Discovery {
    fn default() -> Self {
        Discovery {
            public: true,
            master: the_master(),
            region: String::new(),
            tags: Vec::new(),
            contact: String::new(),
            query: true,
            lan: false,
            blocklist: None,
        }
    }
}

impl Discovery {
    pub fn unlisted() -> Self {
        Discovery { public: false, query: false, ..Discovery::default() }
    }

    pub fn heartbeat(&self) -> Option<&MasterUrl> {
        self.master.as_ref().filter(|_| self.public)
    }

    pub fn lines(&self) -> Vec<String> {
        let chosen = ssx_core::master::master();
        let master = match &self.master {
            Some(url) if chosen.url() == Some(url.to_string().as_str()) => chosen.describe(),
            Some(url) => format!("{url} (given by the program that started the server)"),
            None => chosen.describe(),
        };
        let listed = match (self.public, &self.master) {
            (true, Some(_)) => "yes: listed on the public server list; heartbeats to the master",
            (true, None) => "yes, but there is no master to be listed on",
            (false, _) => "no: not listed, no heartbeat (joined by its address, a link or the LAN tab)",
        };
        let follows = if self.master.is_some() { "its blocklist followed, listed or not" } else { "no blocklist" };
        vec![
            format!("  discovery: public {} ({listed})", self.public),
            format!("  the master: {master}; {follows}"),
            format!(
                "  the UDP query {}, LAN discovery {}",
                if self.query { "on" } else { "off" },
                if self.lan { "on" } else { "off" }
            ),
        ]
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawDiscovery {
    public: Option<Spanned<bool>>,
    masters: Option<Spanned<toml::Value>>,
    region: Option<Spanned<String>>,
    tags: Option<Spanned<Vec<Spanned<String>>>>,
    contact: Option<Spanned<String>>,
    query: Option<Spanned<bool>>,
    lan: Option<Spanned<bool>>,
    blocklist: Option<Spanned<String>>,
}

pub(super) fn parse_discovery(
    d: RawDiscovery,
    at: &dyn Fn(Range<usize>, String) -> ConfigError,
    notices: &mut Vec<String>,
) -> Result<Discovery, ConfigError> {
    let mut out = Discovery::default();
    let fixed = format!("the master server is fixed ({})", ssx_core::OFFICIAL_MASTER);
    let old = d.masters.map(|m| (m.span(), matches!(m.get_ref(), toml::Value::Array(a) if a.is_empty())));
    out.public = match (d.public.as_ref().map(|p| *p.get_ref()), old) {
        (Some(public), Some((span, _))) => {
            let what = if public { "listed" } else { "not listed" };
            notices.push(
                at(span, format!("masters is not read: {fixed}; public = {public} as written: {what}")).to_string(),
            );
            public
        }
        (Some(public), None) => public,
        (None, Some((span, true))) => {
            notices.push(
                at(
                    span,
                    format!(
                        "masters = [] is not read: {fixed}; an empty list kept the server off every master's list, so \
                         it stays unlisted (public = false: write that instead); it follows the master's blocklist as \
                         every server does"
                    ),
                )
                .to_string(),
            );
            false
        }
        (None, Some((span, false))) => {
            notices.push(
                at(span, format!("masters is not read: {fixed}; the server is listed (public = true, the default)"))
                    .to_string(),
            );
            true
        }
        (None, None) => true,
    };
    if let Some(r) = d.region {
        ssx_net_proto::master::region(r.get_ref()).map_err(|e| at(r.span(), e))?;
        out.region = r.into_inner();
    }
    if let Some(t) = d.tags {
        let span = t.span();
        out.tags = t.into_inner().into_iter().map(Spanned::into_inner).collect();
        ssx_net_proto::master::tags(&out.tags).map_err(|e| at(span, e))?;
    }
    if let Some(c) = d.contact {
        ssx_net_proto::master::contact(c.get_ref()).map_err(|e| at(c.span(), e))?;
        out.contact = c.into_inner();
    }
    out.query = d.query.as_ref().map_or(out.public, |q| *q.get_ref());
    out.lan = d.lan.is_some_and(|l| l.into_inner());
    if let Some(b) = d.blocklist {
        out.blocklist = Some(crate::blocklist::local_path(b.get_ref()).map_err(|e| at(b.span(), e))?);
    }
    if out.public && !out.query {
        let span = d.query.map_or(0..0, |q| q.span());
        if d.public.is_some() {
            return Err(at(
                span,
                "query = false: a listed server (public = true) answers the UDP query the master checks it with; set \
                 public = false for a server that is not listed"
                    .into(),
            ));
        }
        notices.push(
            at(
                span,
                "query = false: a listed server answers the UDP query the master checks it with, so this server is not \
                 listed (write public = false to say so)"
                    .into(),
            )
            .to_string(),
        );
        out.public = false;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ssx_net_proto::master::the_master;

    use super::Discovery;
    use crate::config::{Config, parse};

    fn err(text: &str) -> String {
        match parse(Path::new("server.toml"), text) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("accepted: {text}"),
        }
    }

    #[test]
    fn discovery_is_listed_by_default_and_names_no_master() {
        let d = parse(Path::new("s.toml"), "").expect("empty").discovery;
        assert_eq!(d, Discovery::default());
        assert!(d.public && d.query && !d.lan);
        assert_eq!(d.master, the_master());
        assert_eq!(d.heartbeat(), the_master().as_ref(), "a listed server heartbeats to the master");
        assert_eq!(Config::defaults().discovery, d, "a missing file: the same defaults");
        let unlisted = parse(Path::new("s.toml"), "[discovery]\npublic = false\n").expect("unlisted").discovery;
        assert_eq!(unlisted, Discovery::unlisted());
        assert!(!unlisted.public && !unlisted.query && unlisted.heartbeat().is_none(), "no heartbeat");
        assert_eq!(unlisted.master, the_master(), "the master's blocklist followed, listed or not");
        let d = parse(
            Path::new("s.toml"),
            "[discovery]\npublic = true\nregion = \"eu-central\"\ntags = [\"race\", \"vanilla\"]\n\
             contact = \"admin@example.org\"\nquery = true\nlan = true\n",
        )
        .expect("valid")
        .discovery;
        assert!(d.public && d.query && d.lan);
        assert_eq!((d.region.as_str(), d.tags.len(), d.contact.as_str()), ("eu-central", 2, "admin@example.org"));
        let lan_only = parse(Path::new("s.toml"), "[discovery]\npublic = false\nquery = true\nlan = true\n").unwrap();
        assert!(!lan_only.discovery.public && lan_only.discovery.query && lan_only.discovery.lan);
        let no_query = parse(Path::new("server.toml"), "[discovery]\nquery = false\nlan = true\n").expect("reads");
        assert!(!no_query.discovery.public && !no_query.discovery.query && no_query.discovery.heartbeat().is_none());
        assert_eq!(no_query.notices.len(), 1);
        assert!(
            no_query.notices[0].starts_with("server.toml:2:9: query = false: a listed server answers the UDP query")
        );
        assert!(
            err("[discovery]\npublic = true\nquery = false\n")
                .starts_with("server.toml:3:9: query = false: a listed server (public = true) answers")
        );
        for key in ["master", "master_url", "official_master", "masters_url"] {
            assert!(err(&format!("[discovery]\n{key} = \"http://127.0.0.1:9\"\n")).starts_with("server.toml:2:1"));
        }
        assert!(err("[master]\nurl = \"http://127.0.0.1:9\"\n").starts_with("server.toml:1:"));
        assert!(err("[discovery]\nregion = \"EU\"\n").contains("region"));
        assert!(err("[discovery]\ntags = [\"a\", \"a\"]\n").contains("twice"));
        assert!(err("[discovery]\ncontact = \"a\\nb\"\n").contains("contact"));
        assert!(err("[discovery]\nlan = \"no\"\n").starts_with("server.toml:2:7"));
        assert!(err("[discovery]\nlan_port = 1\n").starts_with("server.toml:2:1"));
        assert!(err("[discovery]\npublic = \"yes\"\n").starts_with("server.toml:2:10"));
    }

    #[test]
    fn an_older_files_masters_are_ignored_with_a_line() {
        let load = |text: &str| parse(Path::new("server.toml"), text).unwrap_or_else(|e| panic!("{text}: {e}"));
        let empty = load("[discovery]\nmasters = []\nquery = true\n");
        assert!(!empty.discovery.public && empty.discovery.query);
        assert_eq!(empty.notices.len(), 1);
        assert!(empty.notices[0].starts_with("server.toml:2:11: masters = [] is not read: the master server is fixed"));
        assert!(empty.notices[0].contains("stays unlisted (public = false"), "{}", empty.notices[0]);
        assert!(empty.notices[0].contains("follows the master's blocklist"), "{}", empty.notices[0]);
        assert_eq!(empty.discovery.heartbeat(), None);
        let empty_only = load("[discovery]\nmasters = []\n");
        assert!(!empty_only.discovery.public && !empty_only.discovery.query);
        let old_public = load(
            "[discovery]\npublic = true\nmasters = [\"https://master.example.org\", \"http://127.0.0.1:27900\"]\n\
             query = true\n",
        );
        assert!(old_public.discovery.public);
        assert_eq!(old_public.notices.len(), 1);
        assert!(old_public.notices[0].starts_with("server.toml:3:11: masters is not read"), "{:?}", old_public.notices);
        assert!(old_public.notices[0].contains("public = true as written: listed"));
        let listed = load("[discovery]\nmasters = [\"https://master.example.org\"]\n");
        assert!(listed.discovery.public && listed.discovery.query);
        assert!(listed.notices[0].contains("the server is listed (public = true, the default)"));
        let kept_off = load("[discovery]\npublic = false\nmasters = [\"https://m.example.org\"]\n");
        assert!(!kept_off.discovery.public && kept_off.notices[0].contains("public = false as written: not listed"));
        assert_eq!(kept_off.discovery.heartbeat(), None, "an unlisted server sends no heartbeat");
        for value in [
            format!("[{}]", (0..9).map(|i| format!("\"https://m{i}.example.org\"")).collect::<Vec<_>>().join(", ")),
            "[\"ftp://m\"]".into(),
            "\"https://m.example.org\"".into(),
            "[1, 2]".into(),
        ] {
            let c = load(&format!("[discovery]\nmasters = {value}\n"));
            assert!(c.discovery.public && c.notices.len() == 1, "{value}");
        }
        for c in [&empty, &empty_only, &old_public, &listed, &kept_off] {
            assert_eq!(c.discovery.master, the_master(), "the old key names no master");
        }
        assert!(load("").notices.is_empty());
        assert!(load("[discovery]\npublic = true\n").notices.is_empty());
    }
}
