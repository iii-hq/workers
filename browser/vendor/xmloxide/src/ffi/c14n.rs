//! Canonical XML (C14N) serialization FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::os::raw::c_char;

use crate::serial::c14n::{self, C14nOptions};
use crate::tree::{Document, NodeId};

use super::strings::to_c_string;
use super::{clear_last_error, set_last_error};

/// Canonicalizes a document using inclusive C14N with comments.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_canonicalize(doc: *const Document) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above.
    let doc = unsafe { &*doc };
    let output = c14n::canonicalize(doc, &C14nOptions::default());
    to_c_string(&output)
}

/// Canonicalizes a document with options.
///
/// `with_comments`: 1 to include comments, 0 to strip.
/// `exclusive`: 1 for exclusive C14N, 0 for inclusive.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_canonicalize_opts(
    doc: *const Document,
    with_comments: i32,
    exclusive: i32,
) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above.
    let doc = unsafe { &*doc };
    let opts = C14nOptions {
        with_comments: with_comments != 0,
        exclusive: exclusive != 0,
        inclusive_prefixes: vec![],
    };
    let output = c14n::canonicalize(doc, &opts);
    to_c_string(&output)
}

/// Canonicalizes a subtree rooted at the given node.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_canonicalize_subtree(
    doc: *const Document,
    node: u32,
    with_comments: i32,
    exclusive: i32,
) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    let Some(node_id) = NodeId::from_raw(node) else {
        set_last_error("invalid node id");
        return std::ptr::null_mut();
    };
    // SAFETY: Null check above.
    let doc = unsafe { &*doc };
    let opts = C14nOptions {
        with_comments: with_comments != 0,
        exclusive: exclusive != 0,
        inclusive_prefixes: vec![],
    };
    let output = c14n::canonicalize_subtree(doc, node_id, &opts);
    to_c_string(&output)
}
