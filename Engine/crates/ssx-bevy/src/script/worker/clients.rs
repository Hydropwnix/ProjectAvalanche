use std::collections::BTreeSet;

use ssx_content::policy::ClientScripts;
use ssx_script::host::game::{GameFrame, SESSION_KEYS};
use ssx_script::host::outbox::Destination;

use super::Worker;

pub(in crate::script) type Word = Option<(u64, ClientScripts)>;

impl Worker {
    pub(super) fn seen_session(&self) -> Option<u64> {
        match self.word {
            Some(Some((s, p))) if p != ClientScripts::Pending => Some(s),
            _ => None,
        }
    }

    pub(super) fn sees(&self) -> bool {
        self.seen_session().is_some()
    }

    fn clients_run(&self) -> bool {
        matches!(self.word, Some(None | Some((_, ClientScripts::Allowed))))
    }

    pub(super) fn word(&mut self, word: Word) {
        if word.is_some() {
            self.end_local("a network session begins");
        }
        self.drain_net();
        let dropped = std::mem::take(&mut self.dropped);
        if dropped > 0 {
            self.note(
                false,
                format!("scripts: {dropped} messages of the client mods to a server dropped: no session let them send"),
            );
        }
        let was = self.word.replace(word).flatten().map(|(s, _)| s);
        let ended = was.is_some() && was != word.map(|(s, _)| s);
        if ended {
            self.end_session("the session ended");
        }
        let running = self.clients_started;
        match word {
            None => {
                if ended && !running {
                    self.note(false, "scripts: the session ended: the client mods' scripts start".into());
                }
            }
            Some((_, ClientScripts::Pending)) if running => self.note(
                false,
                "scripts: a network session begins: the client mods' scripts go on and see nothing of it until its \
                 server's client_scripts policy is read (the join reads it)"
                    .into(),
            ),
            Some((_, ClientScripts::Pending)) => self.note(
                false,
                "scripts: the client mods' scripts wait for the server's client_scripts policy (the join reads it)"
                    .into(),
            ),
            Some((_, ClientScripts::Allowed)) => {
                let then = if running { "go on and see the session" } else { "start" };
                self.note(false, format!("scripts: the server allows client scripts: the client mods' scripts {then}"));
            }
            Some((_, p)) => {
                let then = if running { "are stopped for the session" } else { "do not start in this session" };
                self.note(
                    p == ClientScripts::Unstated,
                    format!("scripts: {}: the client mods' scripts {then}", p.reason()),
                );
                self.hold_clients(p.reason());
            }
        }
        self.release_held_off();
        self.refresh();
        self.collect();
    }

    pub(super) fn start_clients(&mut self) {
        if self.clients_started || !self.clients_run() {
            return;
        }
        self.refresh();
        if self.catalog.is_none() || (self.catalogue_session && !self.sees()) {
            if !self.start_waits {
                self.start_waits = true;
                let line = "scripts: the client mods' scripts wait for the session's content to be taken down";
                self.note(false, line.into());
            }
            return;
        }
        self.start_waits = false;
        self.clients_started = true;
        self.start_all(None);
    }

    fn hold_clients(&mut self, why: &str) {
        let names: Vec<String> = self.host.running().into_iter().filter(|n| !self.session.contains_key(n)).collect();
        for name in names {
            self.host.stop(&name, why);
        }
        self.clients_started = false;
        self.start_waits = false;
        self.held_off.clear();
    }

    pub(super) fn release_held_off(&mut self) {
        if !self.clients_started {
            self.held_off.clear();
            return;
        }
        let free: BTreeSet<String> = self.held_off.iter().filter(|n| !self.session.contains_key(*n)).cloned().collect();
        if free.is_empty() {
            return;
        }
        self.held_off.retain(|n| !free.contains(n));
        self.start_all(Some(&free));
    }

    pub(super) fn end_session(&mut self, why: &str) {
        for name in std::mem::take(&mut self.session).into_keys() {
            self.host.stop(&name, why);
        }
        self.held_session = None;
        #[cfg(feature = "net")]
        {
            self.refused = 0;
        }
        self.host.leave_session();
    }

    pub(super) fn session_scripts_off(&mut self) {
        self.drain_net();
        for name in std::mem::take(&mut self.session).into_keys() {
            self.host.stop(&name, "the server's resources with client scripts are gone");
        }
        self.held_session = None;
        self.release_held_off();
        self.collect();
    }

    pub(super) fn drain_net(&mut self) {
        let sees = self.sees();
        let allowed = matches!(self.word, Some(Some((_, ClientScripts::Allowed))));
        for m in self.host.drain(usize::MAX).messages {
            if m.to != Destination::Server {
                continue;
            }
            let resource = m.package.as_ref().is_some_and(|p| self.session.contains_key(p));
            if allowed || (sees && resource) {
                self.net_out.push(m);
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
        }
    }

    pub(super) fn as_seen(&self, f: GameFrame) -> GameFrame {
        match self.word {
            Some(Some((_, p))) if p != ClientScripts::Pending => f,
            Some(None) => {
                let mut f = f;
                f.riders.retain(|r| r.key < SESSION_KEYS);
                f.standings.retain(|s| s.rider.is_none_or(|k| k < SESSION_KEYS));
                f
            }
            _ => GameFrame { now: f.now, keys: f.keys, typing: f.typing, hud_busy: f.hud_busy, ..GameFrame::default() },
        }
    }

    pub(super) fn refresh(&mut self) {
        let sees = self.sees();
        let g = self.content.generation();
        if self.generation == Some(g) && (sees || !self.catalogue_session) {
            return;
        }
        if let Some(looks) = self.catalogue(!sees, g) {
            self.looks = Some(looks);
        }
    }

    pub(super) fn clients_line(&self) -> String {
        let state = match self.word {
            None => "none started: the game's first word is not in yet".to_string(),
            Some(None) if self.clients_started => "running (no network session)".into(),
            Some(None) if self.start_waits => "waiting for the session's content to be taken down".into(),
            Some(None) => "starting at the next frame (no network session)".into(),
            Some(Some((_, ClientScripts::Pending))) if self.clients_started => {
                "running, seeing nothing of the session until its server's client_scripts policy is read".into()
            }
            Some(Some((_, ClientScripts::Pending))) => {
                "waiting for the server's client_scripts policy (the join reads it)".into()
            }
            Some(Some((_, ClientScripts::Allowed))) if self.clients_started => {
                "running in the session: the server allows client scripts".into()
            }
            Some(Some((_, ClientScripts::Allowed))) => {
                "starting at the next frame: the server allows client scripts".into()
            }
            Some(Some((_, p))) => format!("off for this session: {}", p.reason()),
        };
        match self.dropped {
            0 => format!("[client mods] {state}"),
            n => format!("[client mods] {state} | their messages to a server dropped (no session let them send): {n}"),
        }
    }
}
