//! Regression for the identity boundary: reject oversized borrowed IDs before copying them.
use lugus_agent::ToolCall;
use lugus_app::{ErrorKind, Scope, agent_contract::scope_for_call};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct MeasuringAllocator;
thread_local! {
    static MEASURING: Cell<bool> = const { Cell::new(false) };
    static LARGEST: Cell<usize> = const { Cell::new(0) };
}
fn record(size: usize) {
    MEASURING.with(|enabled| {
        if enabled.get() {
            LARGEST.with(|largest| largest.set(largest.get().max(size)));
        }
    });
}
// SAFETY: Every operation delegates the unchanged layout/pointer to System.
unsafe impl GlobalAlloc for MeasuringAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(size);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: MeasuringAllocator = MeasuringAllocator;

#[test]
fn oversized_call_identity_is_rejected_without_proportional_allocation() {
    let bound = Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: Some("run".into()),
    };
    let call = ToolCall {
        run_id: "run".into(),
        call_id: "x".repeat(1024 * 1024),
        name: "lugus_read_fetch".into(),
        arguments: serde_json::json!({"fetch_id":"fetch"}),
    };
    LARGEST.with(|n| n.set(0));
    MEASURING.with(|n| n.set(true));
    let result = scope_for_call(&bound, &call);
    MEASURING.with(|n| n.set(false));
    let largest = LARGEST.with(Cell::get);
    assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidInput);
    assert!(
        largest <= Scope::MAX_ID_BYTES,
        "allocated {largest} bytes for an invalid borrowed identity"
    );
}
