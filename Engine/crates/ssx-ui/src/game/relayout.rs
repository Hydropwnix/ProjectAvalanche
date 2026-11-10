use std::collections::VecDeque;

use crate::avm1::Vm;
use crate::avm1::runtime::nav as navev;
use crate::avm1::value::Value;
use crate::display::Player;
use crate::library::Library;

use super::GameUi;

const MAX_WAIT: u32 = 300;

pub(super) struct Relayout {
    unload: VecDeque<String>,
    load: VecDeque<String>,
    waiting: Option<(bool, String)>,
    waited: u32,
}

impl GameUi {
    pub(super) fn relayout_step(&mut self, vm: &mut Vm, lib: &mut Library, player: &mut Player) {
        if self.relayout.is_none() {
            let at_rest = self.nav_wait.is_none()
                && self.pending_views.is_empty()
                && self.due_views.is_empty()
                && self.loading_views.is_empty()
                && !self.nav.busy()
                && self.fe.as_ref().is_none_or(|f| !f.movie_queued() && !f.in_drop);
            if !self.relayout_due || !at_rest {
                return;
            }
            self.relayout_due = false;
            self.views_display = self.display;
            let mut views: Vec<String> = self
                .store
                .loaded_view_models
                .iter()
                .filter_map(|v| self.store.view_models.get(v).map(|m| m.view.clone()))
                .collect();
            if self.fe.as_ref().is_some_and(|f| !f.popups.stack.is_empty()) {
                views.push(super::fe::popup::VIEW.into());
            }
            self.display_note(format!("display: the views {views:?} unloaded and loaded again"));
            self.relayout = Some(Relayout {
                unload: views.iter().rev().cloned().collect(),
                load: views.into(),
                waiting: None,
                waited: 0,
            });
        }
        let Some(r) = self.relayout.as_mut() else { return };
        if let Some((entered, view)) = r.waiting.clone() {
            r.waited += 1;
            if r.waited < MAX_WAIT {
                return;
            }
            r.waiting = None;
            let line =
                format!("display: the view {view} did not answer its {}", if entered { "load" } else { "unload" });
            self.display_note(line);
            if entered {
                self.loading_views.retain(|v| *v != view);
            }
        }
        let Some(r) = self.relayout.as_mut() else { return };
        r.waited = 0;
        if let Some(view) = r.unload.pop_front() {
            vm.game_event(lib, player, navev::UNLOAD_VIEW, Value::str(&view));
            r.waiting = Some((false, view));
        } else if let Some(view) = r.load.pop_front() {
            vm.game_event(lib, player, navev::LOAD_VIEW, Value::str(&view));
            self.loading_views.push(view.clone());
            r.waiting = Some((true, view));
        } else {
            self.relayout = None;
            self.display_note("display: the views are laid out for the display".into());
        }
    }

    pub(super) fn relayout_answer(&mut self, entered: bool, view: &str) {
        if let Some(r) = self.relayout.as_mut()
            && r.waiting.as_ref().is_some_and(|(e, v)| *e == entered && v == view)
        {
            r.waiting = None;
        }
    }
}
