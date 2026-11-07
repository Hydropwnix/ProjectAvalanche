use crate::avm1::Ctx;
use crate::display::InstanceId;
use crate::display::hit::{Hit, text_char_at};
use crate::game::fe::FrontEnd;
use crate::game::input::Button;
use crate::game::loc::Localization;

use super::rows::{TabPart, tab_parts};
use super::{TABS, Tab};

const VIEW: &str = "ava_browser";
const TABS_FIELD: &str = "txtTabs";
const STATUS_FIELD: &str = "txtStatus";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    Press(Button),
    Tab(Tab),
}

fn field(cx: &Ctx, order: &[InstanceId], name: &str) -> Option<InstanceId> {
    order.iter().copied().find(|&id| {
        let Some(i) = cx.player.get(id) else { return false };
        if i.name != name {
            return false;
        }
        let view = i.parent.and_then(|p| cx.player.get(p)).and_then(|v| v.parent).and_then(|p| cx.player.get(p));
        view.is_some_and(|v| v.name == VIEW)
    })
}

pub fn click(
    fe: &FrontEnd,
    cx: &mut Ctx,
    loc: &mut Localization,
    order: &[InstanceId],
    stage: (f32, f32),
) -> Option<Click> {
    if !fe.ava.browser.active {
        return None;
    }
    let mut meshes = std::mem::take(&mut cx.vm.pointer.meshes);
    let shown = |cx: &Ctx, meshes: &mut _, id| Hit { player: cx.player, lib: cx.lib, meshes }.shown_at(id, stage);
    let mut out = None;
    if let Some(tabs) = field(cx, order, TABS_FIELD)
        && shown(cx, &mut meshes, tabs)
        && let Some(c) = text_char_at(cx.player, tabs, stage)
    {
        out = tab_parts(loc).into_iter().find(|(_, r)| r.contains(&c)).map(|(p, _)| match p {
            TabPart::Bumper(b) => Click::Press(b),
            TabPart::Tab(t) => Click::Tab(t),
        });
    }
    if out.is_none()
        && let Some(status) = field(cx, order, STATUS_FIELD)
        && shown(cx, &mut meshes, status)
        && text_char_at(cx.player, status, stage).is_some()
    {
        out = Some(Click::Press(Button::Ras));
    }
    cx.vm.pointer.meshes = meshes;
    out
}

pub fn tab_step(fe: &FrontEnd, target: Tab) -> Option<Button> {
    let b = &fe.ava.browser;
    if !b.active || b.tab == target {
        return None;
    }
    let pos = |t: Tab| TABS.iter().position(|x| x.0 == t).unwrap_or(0) as i64;
    let n = TABS.len() as i64;
    let forward = (pos(target) - pos(b.tab)).rem_euclid(n);
    Some(if forward <= n / 2 { Button::RightBumper } else { Button::LeftBumper })
}
