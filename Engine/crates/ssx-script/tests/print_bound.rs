use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ssx_script::{FileMap, Limits, ScriptVm};

struct Counting;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) && new_size > layout.size() {
            ALLOCATED.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
fn print_copies_at_most_its_line() {
    let limits = Limits { call: Limits::MAX_CHECKPOINTS, call_backstop: Duration::from_secs(5), memory: 64 << 20 };
    let vm = ScriptVm::new(FileMap::new(), limits).unwrap();
    let setup = "local s = string.rep('x', 1048576) local t = {} for i = 1, 64 do t[i] = s end \
                 function shout() print(table.unpack(t)) end \
                 local o = setmetatable({}, {__tostring = function() return s end}) \
                 function shout_objects() print(o, o, o, o, o, o, o, o) end";
    let script = vm.run_source("setup", setup).unwrap();
    for name in ["shout", "shout_objects"] {
        let h = vm.handler(&script, name).unwrap().unwrap();
        ALLOCATED.store(0, Ordering::Relaxed);
        ACTIVE.store(true, Ordering::Relaxed);
        let r = vm.call::<()>(&h, ());
        ACTIVE.store(false, Ordering::Relaxed);
        r.unwrap();
        let bytes = ALLOCATED.load(Ordering::Relaxed);
        eprintln!("{name}: {bytes} bytes allocated on the host");
        assert!(bytes < 64 << 10, "{name}: the host allocated {bytes} bytes for one printed line");
    }
    let out = vm.take_output();
    assert_eq!(out.lines.len(), 2);
    assert!(out.lines.iter().all(|l| l.len() < ssx_script::MAX_OUTPUT_LINE + 64));
    assert!(
        out.lines[0].ends_with(&format!("({} bytes cut)", 64 * (1 << 20) + 63 - 4096)),
        "{}",
        &out.lines[0][4090..]
    );
}
