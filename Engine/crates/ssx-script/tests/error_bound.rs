use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ssx_script::{FileMap, Limits, ScriptError, ScriptVm};

struct Counting;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static TEXT_BUFFERS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
            if layout.align() == 1 {
                TEXT_BUFFERS.fetch_add(layout.size(), Ordering::Relaxed);
            }
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) && new_size > layout.size() {
            ALLOCATED.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            if layout.align() == 1 {
                TEXT_BUFFERS.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            }
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const TEXT: usize = 4 << 20;

#[test]
fn errors_cost_the_host_little() {
    let limits = Limits { call: Limits::MAX_CHECKPOINTS, call_backstop: Duration::from_secs(5), memory: 64 << 20 };
    let vm = ScriptVm::new(FileMap::new(), limits).unwrap();
    let setup = format!(
        "local s = string.rep('x', {TEXT}) \
         function plain() error(s) end \
         local o = setmetatable({{}}, {{__tostring = function() return s end}}) \
         function object() error(o) end"
    );
    let script = vm.run_source("setup", &setup).unwrap();
    let mut cost = Vec::new();
    for name in ["plain", "object"] {
        let h = vm.handler(&script, name).unwrap().unwrap();
        ALLOCATED.store(0, Ordering::Relaxed);
        TEXT_BUFFERS.store(0, Ordering::Relaxed);
        ACTIVE.store(true, Ordering::Relaxed);
        let r = vm.call::<()>(&h, ());
        ACTIVE.store(false, Ordering::Relaxed);
        let Err(ScriptError::Script(msg)) = r else { panic!("{name}: {:?}", r.err()) };
        assert!(msg.len() < ssx_script::MAX_ERROR_LEN + 64, "{name}: {}", msg.len());
        let (all, text) = (ALLOCATED.load(Ordering::Relaxed), TEXT_BUFFERS.load(Ordering::Relaxed));
        eprintln!("{name}: {all} bytes allocated during the failing call, {text} of them host text buffers");
        cost.push((all, text));
    }
    for (name, (_, text)) in ["plain", "object"].iter().zip(&cost) {
        assert!(*text < 64 << 10, "{name}: the host copied {text} bytes of text");
    }
    assert_eq!(vm.stopped(), None);
}
