use super::*;

impl GameUi {
    pub fn views_loading(&self) -> bool {
        !self.pending_views.is_empty()
            || !self.due_views.is_empty()
            || !self.loading_views.is_empty()
            || self.nav_wait.is_some()
            || self.relayout.is_some()
    }

    pub fn menus_settled(&self) -> bool {
        use super::fe::{busy, progression};
        let Some(fe) = self.fe.as_ref() else { return false };
        let pending = self.values.int(progression::PENDING_NAVIGATION);
        fe.busy.counts[busy::LEVEL] == 0
            && (pending == 0 || pending as u32 == progression::NO_SCREEN)
            && fe.launch.is_none()
            && !fe.in_drop
            && !self.nav.busy()
            && !self.views_loading()
    }

    pub fn loading_view_names(&self) -> Vec<&str> {
        let waited = match &self.nav_wait {
            Some(NavWait::Enter(v)) | Some(NavWait::Exit { view: v, .. }) => Some(v.as_str()),
            None => None,
        };
        self.pending_views
            .iter()
            .chain(&self.due_views)
            .chain(&self.loading_views)
            .map(String::as_str)
            .chain(waited)
            .collect()
    }

    pub fn settle_views(
        &mut self,
        vm: &mut Vm,
        lib: &mut Library,
        player: &mut Player,
        dt: f64,
        max_updates: usize,
    ) -> bool {
        for _ in 0..max_updates {
            if !self.views_loading() {
                return true;
            }
            self.update(vm, lib, player, dt);
        }
        !self.views_loading()
    }

    pub fn update(&mut self, vm: &mut Vm, lib: &mut Library, player: &mut Player, dt: f64) {
        const STEP: f64 = 1.0 / FE_HZ;
        if self.fe.as_ref().is_some_and(|f| fe::pads::poll_change(f, &self.values).is_some()) {
            self.fe_call(fe::pads::poll);
        }
        self.fe_clock += dt.max(0.0);
        let mut scene = Some(dt);
        let mut steps = 0;
        while self.fe_clock + 1e-9 >= STEP && steps < 2 {
            self.fe_clock -= STEP;
            self.fe_update(vm, lib, player, STEP, scene.take());
            steps += 1;
        }
        if steps == 2 {
            self.fe_clock = self.fe_clock.min(STEP);
        }
        // TODO(guess): a port choice — the globe's scene moves with every drawn frame (the game updates it at 30 Hz and blends snapshots: its render clock interpolates the last two 30 Hz snapshots, 0x829E1580 / 0x829E17A0, never past the newest); settle: nothing to decode (the globe's render objects' interpolation).
        if let Some(dt) = scene {
            self.fe_call(|fe, env| fe::globe::scene_update(fe, env, dt));
        }
    }

    pub fn take_stops(&mut self, vm: &mut Vm, lib: &mut Library) {
        for stop in vm.take_stops() {
            let path = lib.movie(stop.movie).path.clone();
            let line = match lib.origin(stop.movie).cloned() {
                Some(o) => {
                    if let Some(content) = lib.files().content() {
                        content.refuse_movie(&o.id, &o.source, &stop.why);
                    }
                    let key = format!("{}{path}", crate::fs::FE_ROOT);
                    lib.forget_movies(|k| k == key);
                    let then = if vm.reload_stopped(stop.movie) {
                        "the disc's movie is loaded in its place"
                    } else {
                        "its clips are off the display list, the disc's movie is loaded next time"
                    };
                    format!("ui: the movie {} of {} ({path}) stopped: {}; {then}", o.id, o.source, stop.why)
                }
                None => format!("ui: the movie {path} stopped: {}", stop.why),
            };
            if let Some(fe) = self.fe.as_mut() {
                fe.note(&line);
            }
            self.stats.content_notices.push(line);
        }
    }

    pub(super) fn fe_update(
        &mut self,
        vm: &mut Vm,
        lib: &mut Library,
        player: &mut Player,
        dt: f64,
        scene: Option<f64>,
    ) {
        if self.fe.as_ref().is_some_and(fe::FrontEnd::movie_queued) {
            self.input.clear();
            self.pointer_lost();
            let mut nav_events = Vec::new();
            if let Some(fe) = self.fe.as_mut() {
                let mut fenv = fe::FeEnv { values: &mut self.values, loc: &mut self.loc, nav_events: &mut nav_events };
                fe.update(&mut fenv, dt);
                if let Some(dt) = scene {
                    fe::globe::scene_update(fe, &mut fenv, dt);
                }
            }
            self.game_actions(&mut nav_events);
            for e in nav_events {
                self.nav.raise(&e);
            }
            self.run_nav(vm, lib, player);
            return;
        }
        self.take_stops(vm, lib);
        let mut nav_events = Vec::new();
        {
            let mut cx = vm.ctx(lib, player);
            self.take_calls(&mut cx);
            self.answer_resources(&mut cx);
            let mut env = HudEnv {
                values: &mut self.values,
                loc: &mut self.loc,
                event_type: self.event_type,
                sounds: &mut self.sounds,
                nav_events: &mut nav_events,
                gear: self.gear,
            };
            let active = |p: &str| self.store.active.iter().any(|x| x == p);
            if self.fe.is_none() || active("hud") {
                self.hud.update(&mut cx, &mut env, dt);
            } else {
                use hud::Component;
                let h = &mut self.hud;
                let hosted: [(&str, &mut dyn Component); 6] = [
                    ("finishmessage", &mut h.finish),
                    ("hudmeters", &mut h.meters),
                    ("boostnotification", &mut h.boost),
                    ("dynamicleaderboard", &mut h.dynlb),
                    ("nisprompt", &mut h.nis_prompt),
                    ("nistitle", &mut h.nis_title),
                ];
                for (p, c) in hosted {
                    if active(p) {
                        c.update(&mut cx, &mut env, dt);
                    }
                }
            }
            self.hud.hint.update(&mut cx, &mut env, dt, self.fe.as_ref().map(|f| &f.hints.records));
            if let Some(fe) = self.fe.as_mut() {
                let mut fenv = fe::FeEnv { values: &mut self.values, loc: &mut self.loc, nav_events: &mut nav_events };
                fe.update(&mut fenv, dt);
                if self.store.active.iter().any(|p| p == "globe") {
                    fe::globe::provider_update(fe, &mut fenv);
                }
                if let Some(dt) = scene {
                    fe::globe::scene_update(fe, &mut fenv, dt);
                }
            }
            self.game_actions(&mut nav_events);
            self.publish(&mut cx);
            self.tutorial.published();
            self.values.clear_dirty();
        }
        self.port_register(vm, lib, player);
        self.due_views = std::mem::take(&mut self.pending_views);
        if self.fe.as_ref().is_some_and(|f| f.input_lock > 0) {
            // TODO(guess): a button still held when the lock ends is a press at the game's first read after it; the port's input carries presses only and drops it; settle: the host's held buttons into `input` at the lock's end.
            self.input.clear();
            self.pointer_lost();
        } else {
            if let Some(device) = self.fe.as_mut().and_then(|f| fe::pads::disconnect_input(f, &self.values)) {
                let i = input::Input::Button { button: input::Button::Disconnect, down: true };
                self.input.push_from(i, device);
            }
            let mut cx = vm.ctx(lib, player);
            self.pointer_read(&mut cx);
            self.stats.inputs += self.input.deliver(&mut cx) as u64;
        }
        for e in nav_events {
            self.nav.raise(&e);
        }
        self.port_open();
        self.relayout_step(vm, lib, player);
        self.run_nav(vm, lib, player);
        // TODO(guess): the resource loads take one FE update; settle: the resource manager's service of the view model's request (0x825A1D48, callback 0x825A27E8).
        for view in std::mem::take(&mut self.due_views) {
            vm.game_event(lib, player, navev::LOAD_VIEW, Value::str(&view));
            self.loading_views.push(view);
        }
        vm.advance(lib, player, dt);
    }
}
