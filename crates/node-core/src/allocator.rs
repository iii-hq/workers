//! A cap on the memory V8 allocates OUTSIDE its object heap.
//!
//! `heap_limits` bounds the object heap and nothing else. `ArrayBuffer` — and
//! everything built on it, so every `TypedArray` — is backed by V8's
//! `ArrayBuffer::Allocator`, which is not part of that budget. A runtime
//! configured `heap_mb: 128` could hold hundreds of megabytes resident and
//! still OOM correctly on the object heap, because the two are accounted
//! separately. On a worker running untrusted code for many tenants at once,
//! that is a way for one tenant to exhaust the host.
//!
//! This is the same hazard class the log buffer and `op_iii_call` payloads
//! already guard against: Rust-side memory reachable through a sanctioned
//! path, invisible to `heap_limits`.
//!
//! # Panic safety
//!
//! Every function here is called by V8 through `extern "C"`. Unwinding across
//! that boundary aborts the process — killing every other tenant's runtime,
//! the exact failure this worker's id registry exists to prevent. So nothing
//! below can panic: no `unwrap`, no indexing, no formatting, no arithmetic
//! that can overflow. Allocation failure returns null, and V8 treats that as
//! unrecoverable: the asking runtime dies with `node-engine::oom`, which is
//! the same way exceeding `heap_mb` ends.

use std::alloc::{alloc, alloc_zeroed, dealloc, Layout};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use deno_core::v8;

/// Every block is allocated AND freed with this alignment, so `free` can
/// rebuild the exact `Layout` `alloc` used from the length V8 hands back.
/// V8 expects malloc-grade alignment; 16 satisfies it on every target here.
const ALIGN: usize = 16;

/// What a zero-length allocation returns. `Layout` permits a zero size but
/// `alloc` does not, and V8 accepts any aligned non-null pointer for an empty
/// buffer. Never freed, never counted — see `free`.
const EMPTY: *mut c_void = ALIGN as *mut c_void;

/// Live off-heap bytes for one isolate, and the ceiling they may reach.
pub struct ExternalMemoryCap {
    used: AtomicUsize,
    cap: usize,
}

impl ExternalMemoryCap {
    /// Bytes currently held in off-heap buffers. Test/observability only.
    pub fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }

    /// Take `len` from the budget, or refuse. `checked_add` because a
    /// caller-driven size is untrusted input like any other.
    fn reserve(&self, len: usize) -> bool {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            let Some(next) = current.checked_add(len) else {
                return false;
            };
            if next > self.cap {
                return false;
            }
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// Return `len` to the budget, saturating at zero. A plain `fetch_sub`
    /// would wrap to a huge value if V8 ever freed a length we did not count,
    /// and a wrapped counter refuses every later allocation for the life of
    /// the isolate.
    fn release(&self, len: usize) {
        let mut current = self.used.load(Ordering::Relaxed);
        loop {
            let next = current.saturating_sub(len);
            match self.used.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }
}

fn layout_for(len: usize) -> Option<Layout> {
    Layout::from_size_align(len, ALIGN).ok()
}

fn allocate_inner(cap: &ExternalMemoryCap, len: usize, zeroed: bool) -> *mut c_void {
    if len == 0 {
        return EMPTY;
    }
    if !cap.reserve(len) {
        return std::ptr::null_mut();
    }
    let Some(layout) = layout_for(len) else {
        cap.release(len);
        return std::ptr::null_mut();
    };
    // SAFETY: `layout` has a non-zero size, checked above.
    let ptr = unsafe {
        if zeroed {
            alloc_zeroed(layout)
        } else {
            alloc(layout)
        }
    };
    if ptr.is_null() {
        // The OS refused; give the budget back rather than leaking it.
        cap.release(len);
    }
    ptr.cast()
}

/// # Safety
/// Called only by V8, with a `handle` that outlives every allocation.
unsafe extern "C" fn allocate(handle: &ExternalMemoryCap, len: usize) -> *mut c_void {
    allocate_inner(handle, len, true)
}

/// # Safety
/// Called only by V8, with a `handle` that outlives every allocation.
unsafe extern "C" fn allocate_uninitialized(handle: &ExternalMemoryCap, len: usize) -> *mut c_void {
    allocate_inner(handle, len, false)
}

/// # Safety
/// `data` must be a pointer this allocator returned for exactly `len` bytes.
unsafe extern "C" fn free(handle: &ExternalMemoryCap, data: *mut c_void, len: usize) {
    if len == 0 || data.is_null() {
        // The `EMPTY` sentinel was never allocated and never counted.
        return;
    }
    let Some(layout) = layout_for(len) else {
        return;
    };
    // SAFETY: same layout `allocate_inner` used — ALIGN is fixed and `len` is
    // the length V8 recorded for this block.
    unsafe { dealloc(data.cast(), layout) };
    handle.release(len);
}

/// # Safety
/// Called once by V8 when the allocator is destroyed, with the pointer handed
/// to `new_rust_allocator`.
unsafe extern "C" fn drop_handle(handle: *const ExternalMemoryCap) {
    // Reclaim the reference `into_raw` leaked; the isolate's copy is going
    // away, and the manager may still hold its own.
    unsafe { drop(Arc::from_raw(handle)) };
}

static VTABLE: v8::RustAllocatorVtable<ExternalMemoryCap> = v8::RustAllocatorVtable {
    allocate,
    allocate_uninitialized,
    free,
    drop: drop_handle,
};

/// An allocator that refuses past `cap_bytes`, plus a handle for reading the
/// live total.
///
/// V8 treats a null from this allocator as unrecoverable rather than throwing
/// a catchable `RangeError`, so a tenant that blows the cap loses its runtime
/// with `node-engine::oom` — the same outcome as exceeding `heap_mb`. That is
/// the intended trade: the runtime that asked for too much dies, the worker
/// and every other tenant carry on.
pub fn capped(cap_bytes: usize) -> (Arc<ExternalMemoryCap>, v8::UniqueRef<v8::Allocator>) {
    let state = Arc::new(ExternalMemoryCap {
        used: AtomicUsize::new(0),
        cap: cap_bytes,
    });
    let raw = Arc::into_raw(state.clone());
    // SAFETY: `raw` is a live `Arc` pointer that `drop_handle` reclaims
    // exactly once, and `VTABLE` is built for this handle type.
    let allocator = unsafe { v8::new_rust_allocator(raw, &VTABLE) };
    (state, allocator)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(bytes: usize) -> ExternalMemoryCap {
        ExternalMemoryCap {
            used: AtomicUsize::new(0),
            cap: bytes,
        }
    }

    #[test]
    fn reserves_up_to_the_cap_and_refuses_past_it() {
        let c = cap(1000);
        assert!(c.reserve(600));
        assert!(c.reserve(400));
        assert_eq!(c.used(), 1000);
        assert!(!c.reserve(1), "cap must be a ceiling");
        c.release(400);
        assert!(c.reserve(400), "freed bytes return to the budget");
    }

    /// A wrapped counter would refuse every later allocation for the life of
    /// the isolate, which is a worse failure than the miscount it came from.
    #[test]
    fn releasing_more_than_was_reserved_saturates_at_zero() {
        let c = cap(1000);
        c.reserve(100);
        c.release(100_000);
        assert_eq!(c.used(), 0);
        assert!(c.reserve(1000), "still usable after an over-release");
    }

    #[test]
    fn an_overflowing_request_is_refused_not_wrapped() {
        let c = cap(usize::MAX);
        assert!(c.reserve(16));
        assert!(!c.reserve(usize::MAX), "checked_add must refuse");
        assert_eq!(c.used(), 16);
    }

    #[test]
    fn zero_length_allocations_are_free_and_uncounted() {
        let c = cap(64);
        let p = allocate_inner(&c, 0, true);
        assert!(
            !p.is_null(),
            "V8 needs a non-null pointer for an empty buffer"
        );
        assert_eq!(c.used(), 0);
        // SAFETY: the zero-length path never allocated, and `free` returns
        // early on len 0 without touching the pointer.
        unsafe { free(&c, p, 0) };
        assert_eq!(c.used(), 0);
    }

    #[test]
    fn a_real_allocation_round_trips_through_the_budget() {
        let c = cap(4096);
        let p = allocate_inner(&c, 1024, true);
        assert!(!p.is_null());
        assert_eq!(c.used(), 1024);
        // Zeroed as promised: `allocate` (not `allocate_uninitialized`) must
        // hand V8 cleared memory.
        // SAFETY: 1024 readable bytes were just allocated.
        let first = unsafe { *(p as *const u8) };
        assert_eq!(first, 0);
        // SAFETY: allocated here with exactly this length.
        unsafe { free(&c, p, 1024) };
        assert_eq!(c.used(), 0);
    }

    #[test]
    fn an_allocation_over_the_cap_returns_null_and_costs_nothing() {
        let c = cap(1024);
        let p = allocate_inner(&c, 4096, true);
        assert!(p.is_null(), "over-cap must refuse, not allocate");
        assert_eq!(c.used(), 0, "a refused request must not consume budget");
    }
}
