use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use bevy::ecs::error::{BevyError, ErrorContext};

static COUNT: AtomicU64 = AtomicU64::new(0);
static LAST: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
const KEPT: usize = 16;

pub fn handler(mut error: BevyError, ctx: ErrorContext) {
    if let Some(payload) = error.take_payload() {
        std::panic::resume_unwind(payload);
    }
    COUNT.fetch_add(1, Ordering::Relaxed);
    let text = format!("{ctx}: {error}");
    bevy::log::error!("system error: {text}");
    if let Ok(mut last) = LAST.lock() {
        if last.len() == KEPT {
            last.pop_front();
        }
        last.push_back(text);
    }
}

pub fn count() -> u64 {
    COUNT.load(Ordering::Relaxed)
}

pub fn last() -> Vec<String> {
    LAST.lock().map(|l| l.iter().cloned().collect()).unwrap_or_default()
}
