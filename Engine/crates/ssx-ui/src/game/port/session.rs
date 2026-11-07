use crate::game::fe::ava::session::{ChatIn, ChatOut, SessionPlayer, chat};

impl crate::game::GameUi {
    pub fn session_begin(&mut self, server: &str, max_players: Option<u16>) {
        if let Some(fe) = self.fe.as_mut() {
            fe.ava.session.begin(server, max_players);
        }
    }

    pub fn session_end(&mut self) {
        self.fe_call(chat::close_entries);
        let Some(fe) = self.fe.as_mut() else { return };
        if fe.ava.session.chat.up {
            self.nav.raise(chat::DISABLE);
        }
        fe.ava.session.end();
    }

    pub fn session_level(&mut self, up: bool) {
        let Some(fe) = self.fe.as_mut() else { return };
        if fe.ava.session.chat.up == up || (up && !fe.ava.session.active) {
            return;
        }
        fe.ava.session.chat.up = up;
        fe.note(format!("chat: the layer {}", if up { "comes (the level's HUD is up)" } else { "goes" }));
        if !up {
            self.fe_call(chat::close_entries);
        }
        self.nav.raise(if up { chat::ENABLE } else { chat::DISABLE });
    }

    pub fn session_players(&mut self, players: Vec<SessionPlayer>) {
        if let Some(fe) = self.fe.as_mut()
            && fe.ava.session.active
        {
            fe.ava.session.set_players(players);
        }
    }

    pub fn session_chat(&mut self, items: Vec<ChatIn>, enabled: Option<bool>) {
        let Some(fe) = self.fe.as_mut() else { return };
        if !fe.ava.session.active {
            return;
        }
        if enabled.is_some() && fe.ava.session.chat.enabled.is_none() {
            fe.ava.session.chat.enabled = enabled;
        }
        if !items.is_empty() {
            chat::receive(fe, &mut self.loc, items);
        }
    }

    pub fn take_chat_out(&mut self) -> Vec<ChatOut> {
        self.fe.as_mut().map(|f| std::mem::take(&mut f.ava.session.chat.out)).unwrap_or_default()
    }

    pub fn session_rounds(&mut self, mode: Option<ssx_net_proto::Mode>) {
        if let Some(fe) = self.fe.as_mut()
            && fe.ava.session.active
            && fe.ava.session.rounds != mode
        {
            fe.ava.session.rounds = mode;
            fe.ava.session.list.dirty = true;
        }
    }

    pub fn session_banner(&mut self, text: &str) {
        let Some(fe) = self.fe.as_mut() else { return };
        if fe.ava.session.active && fe.ava.session.banner != text {
            fe.ava.session.banner = text.to_string();
        }
    }
}
