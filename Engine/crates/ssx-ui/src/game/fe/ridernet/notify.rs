use std::collections::VecDeque;

use crate::avm1::Ctx;
use crate::avm1::value::Value;
use crate::game::fe::{ActionParam, FeEnv, FrontEnd};
use crate::game::loc::Localization;

pub const DP: &str = "dp_notification_data";

pub const KIND_BADGE: u32 = 0x14;
pub const QUEUE_MAX: usize = 25;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notification {
    pub kind: u32,
    pub id: i32,
    pub title: String,
    pub message: String,
    pub icon: String,
}

#[derive(Clone, Debug, Default)]
pub struct Notifications {
    pub queue: VecDeque<Notification>,
    pub current: Option<Notification>,
    pub request: u32,
    pub expanded: bool,
    next_id: i32,
}

pub fn style(kind: u32) -> i32 {
    match kind {
        10 | 0xB => 2,
        0x11 => 3,
        0x14 => 1,
        _ => 0,
    }
}

pub fn display_time(kind: u32) -> i32 {
    match kind {
        1 | 4 | 6 | 7 | 0xB | 0xC | 0xD | 0x13 | 0x15 | 0x18 => 10000,
        0x14 | 0x16 | 0x17 => 3000,
        _ => 6000,
    }
}

pub fn sounds(kind: u32) -> (i32, i32) {
    match kind {
        1 | 3 | 4 | 0x13 => (0x20, 0x21),
        8 => (0x22, 0x23),
        9..=0xB => (0x24, 0x25),
        0xE..=0x11 => (0x26, 0x27),
        0x14 => (0x1A, 0x1B),
        0x16 => (0x1C, 0x1D),
        _ => (0x1E, 0x1F),
    }
}

pub fn actionable(kind: u32) -> bool {
    matches!(kind, 2 | 3 | 4 | 6 | 8 | 9 | 10 | 0xC | 0xD | 0xF | 0x13 | 0x15)
}

impl Notifications {
    pub fn push(&mut self, mut n: Notification) {
        n.id = self.next_id;
        self.next_id += 1;
        self.queue.push_back(n);
        while self.queue.len() > QUEUE_MAX {
            self.queue.pop_front();
        }
    }

    fn take(&mut self) -> Option<Notification> {
        let all = match self.request {
            1 => false,
            2 => true,
            _ => return None,
        };
        let i = self.queue.iter().position(|n| all || matches!(n.kind, 7 | 0xE | 0x10 | 0x14 | 0x17))?;
        self.queue.remove(i)
    }
}

pub fn post_badge(fe: &mut FrontEnd, loc: &mut Localization, b: &ssx_assets::career::profile::BadgeText) {
    let n = Notification {
        kind: KIND_BADGE,
        id: 0,
        title: loc.text(&b.title),
        message: loc.text(&b.description),
        icon: b.icon.clone(),
    };
    fe.note(format!("notification: badge {} ({})", b.name, n.title));
    fe.notifications.push(n);
}

pub const KIND_HINT: u32 = 0x18;

pub fn post_hint(fe: &mut FrontEnd, loc: &mut Localization, r: &crate::game::fe::hint::Record) {
    let page = r.pages.first().cloned().unwrap_or_default();
    let n = Notification {
        kind: KIND_HINT,
        id: 0,
        title: loc.text(&r.title),
        message: loc.text(&page.body),
        icon: page.images[0].clone(),
    };
    fe.note(format!("notification: tutorial hint {:#010x} ({})", r.key, n.title));
    fe.notifications.push(n);
}

pub fn update(fe: &mut FrontEnd, env: &mut FeEnv) {
    if env.values.int(0xA6) != 1 || fe.notifications.current.is_some() {
        return;
    }
    if let Some(n) = fe.notifications.take() {
        fe.note(format!("notification {} published (type {:#x})", n.id, n.kind));
        fe.notifications.current = Some(n);
        fe.publish.push(DP.into());
    }
}

pub fn action(fe: &mut FrontEnd, action: &str, param: &ActionParam) {
    match action {
        "A_NotificationExecute" | "A_NotificationRelease" => fe.notifications.current = None,
        "A_NotificationSetRequestType" => {
            let n = param.number as u32;
            if n <= 2 {
                fe.notifications.request = n;
            }
        }
        "A_NotificationSetExpanded" => fe.notifications.expanded = param.number != 0.0,
        _ => {}
    }
}

pub fn build(fe: &FrontEnd, cx: &mut Ctx) -> Value {
    let o = Value::Object(cx.new_object());
    let n = fe.notifications.current.clone().unwrap_or_default();
    let (open, close) = sounds(n.kind);
    for (k, v) in [
        ("messagetitle", Value::str(&n.title)),
        ("message", Value::str(&n.message)),
        ("iconurl", Value::str(&n.icon)),
        ("moneyLabel", Value::str("")),
        ("moneyLocalized", Value::str("")),
        ("bracketLocalized", Value::str("")),
        ("numGeotags", Value::Number(0.0)),
        ("bracket", Value::Number(0.0)),
        ("isActionable", Value::Bool(actionable(n.kind))),
        ("openSound", Value::Number(f64::from(open))),
        ("closeSound", Value::Number(f64::from(close))),
        ("style", Value::Number(f64::from(style(n.kind)))),
        ("displayTime", Value::Number(f64::from(display_time(n.kind)))),
    ] {
        let _ = cx.set_member(&o, k, v);
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_queue_drops_its_oldest_and_a_request_takes_its_kind() {
        let mut q = Notifications::default();
        for i in 0..30 {
            q.push(Notification { kind: if i == 29 { KIND_BADGE } else { 1 }, ..Default::default() });
        }
        assert_eq!(q.queue.len(), QUEUE_MAX);
        assert_eq!(q.queue.front().map(|n| n.id), Some(5));
        assert!(q.take().is_none(), "no request");
        q.request = 1;
        assert_eq!(q.take().map(|n| n.kind), Some(KIND_BADGE));
        assert!(q.take().is_none(), "type 1 waits for request 2");
        q.request = 2;
        assert_eq!(q.take().map(|n| n.id), Some(5));
        assert_eq!((style(KIND_BADGE), display_time(KIND_BADGE), sounds(KIND_BADGE)), (1, 3000, (26, 27)));
    }
}
