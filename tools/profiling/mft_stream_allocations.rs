//! Allocation requests for the fixture workload; do not use its instrumented timings.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
};

#[path = "../../support/mft_stream_workload.rs"]
mod workload;

struct CountingAllocator;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REQUESTED_BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every allocation operation forwards the original pointer/layout to
// System and returns its result unchanged. Counters neither allocate nor panic.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() && ACTIVE.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplies a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() && ACTIVE.load(Relaxed) {
            ALLOCATIONS.fetch_add(1, Relaxed);
            REQUESTED_BYTES.fetch_add(layout.size() as u64, Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: all pointers managed by this allocator come from System.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: forward the caller's live allocation and requested size.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() && ACTIVE.load(Relaxed) {
            REALLOCATIONS.fetch_add(1, Relaxed);
            REQUESTED_BYTES.fetch_add(size as u64, Relaxed);
        }
        pointer
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn main() {
    if let Err(error) = run() {
        eprintln!("allocation profile failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> ntfs_mft::MftResult<()> {
    for source in [workload::Source::Memory, workload::Source::File] {
        for resolve in [false, true] {
            workload::check(&workload::run(source, resolve)?);
            ALLOCATIONS.store(0, Relaxed);
            REALLOCATIONS.store(0, Relaxed);
            REQUESTED_BYTES.store(0, Relaxed);
            ACTIVE.store(true, Relaxed);
            let result = workload::run(source, resolve);
            ACTIVE.store(false, Relaxed);
            workload::check(&result?);
            println!(
                "source={source:?} extensions={resolve} allocations={} reallocations={} requested_bytes={}",
                ALLOCATIONS.load(Relaxed),
                REALLOCATIONS.load(Relaxed),
                REQUESTED_BYTES.load(Relaxed)
            );
        }
    }
    Ok(())
}
