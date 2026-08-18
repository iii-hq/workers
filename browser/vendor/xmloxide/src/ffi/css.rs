//! CSS selector FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::css;
use crate::tree::Document;

use super::{clear_last_error, set_last_error};

/// Evaluates a CSS selector against a subtree and returns matching node IDs.
///
/// `scope` is the node to search within (typically the root element).
/// `selector` is a null-terminated UTF-8 CSS selector string.
///
/// On success, sets `*out_count` to the number of matching nodes and returns
/// a heap-allocated array of `uint32_t` node IDs. The caller must free the
/// array with [`xmloxide_free_nodeid_array`].
///
/// Returns null on failure (parse error in selector or null arguments).
///
/// # Safety
///
/// `doc` must be a valid document pointer. `selector` must be a valid
/// null-terminated UTF-8 string. `out_count` must be a valid pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_css_select(
    doc: *const Document,
    scope: u32,
    selector: *const c_char,
    out_count: *mut usize,
) -> *mut u32 {
    clear_last_error();
    if doc.is_null() || selector.is_null() || out_count.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }

    // SAFETY: Null checks above.
    let doc_ref = unsafe { &*doc };
    let css_sel = unsafe { CStr::from_ptr(selector) };
    let Ok(sel) = css_sel.to_str() else {
        set_last_error("invalid UTF-8 in selector");
        return std::ptr::null_mut();
    };

    let Some(scope_id) = crate::tree::NodeId::from_raw(scope) else {
        set_last_error("invalid scope node id");
        return std::ptr::null_mut();
    };

    match css::select(doc_ref, scope_id, sel) {
        Ok(nodes) => {
            let ids: Vec<u32> = nodes.iter().map(|n| n.into_raw()).collect();
            let len = ids.len();
            // SAFETY: out_count was checked non-null.
            unsafe { *out_count = len };
            if ids.is_empty() {
                // Return a non-null sentinel for empty results that can safely
                // be freed (dangling pointer with zero length).
                return std::ptr::NonNull::dangling().as_ptr();
            }
            let boxed = ids.into_boxed_slice();
            Box::into_raw(boxed).cast::<u32>()
        }
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Frees a node ID array returned by [`xmloxide_css_select`].
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `ptr` must have been returned by `xmloxide_css_select` with the
/// corresponding `count`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_nodeid_array(ptr: *mut u32, count: usize) {
    if !ptr.is_null() && count > 0 {
        // SAFETY: Pointer and count were returned by `xmloxide_css_select`.
        unsafe {
            let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, count));
        }
    }
}

/// Returns the first node matching a CSS selector, or 0 if none found.
///
/// This is a convenience wrapper — it evaluates the selector and returns
/// only the first match.
///
/// # Safety
///
/// `doc` must be a valid document pointer. `selector` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_css_select_first(
    doc: *const Document,
    scope: u32,
    selector: *const c_char,
) -> u32 {
    clear_last_error();
    if doc.is_null() || selector.is_null() {
        set_last_error("null pointer argument");
        return 0;
    }

    // SAFETY: Null checks above.
    let doc_ref = unsafe { &*doc };
    let css_sel = unsafe { CStr::from_ptr(selector) };
    let Ok(sel) = css_sel.to_str() else {
        set_last_error("invalid UTF-8 in selector");
        return 0;
    };

    let Some(scope_id) = crate::tree::NodeId::from_raw(scope) else {
        set_last_error("invalid scope node id");
        return 0;
    };

    match css::select(doc_ref, scope_id, sel) {
        Ok(nodes) => nodes.first().map_or(0, |n| n.into_raw()),
        Err(e) => {
            set_last_error(&e.to_string());
            0
        }
    }
}
