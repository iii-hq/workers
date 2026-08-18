//! HTML5 parsing FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::html5::{parse_html5, parse_html5_with_options, Html5ParseOptions};
use crate::tree::Document;

use super::{clear_last_error, set_last_error};

/// Parses an HTML5 string into a document using the WHATWG parsing algorithm.
///
/// Returns a pointer to the document on success, or null on failure.
/// The returned document must be freed with [`xmloxide_free_doc`](super::document::xmloxide_free_doc).
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_html5(input: *const c_char) -> *mut Document {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let s = match c_str.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8: {e}"));
            return std::ptr::null_mut();
        }
    };
    match parse_html5(s) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Parses an HTML5 fragment with the given context element.
///
/// This implements the fragment parsing algorithm (the algorithm behind
/// `innerHTML`). The `context_element` is the tag name of the context
/// (e.g., `"body"`, `"div"`, `"table"`).
///
/// Returns a pointer to the document on success, or null on failure.
/// The returned document must be freed with [`xmloxide_free_doc`](super::document::xmloxide_free_doc).
///
/// # Safety
///
/// `input` and `context_element` must be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_html5_fragment(
    input: *const c_char,
    context_element: *const c_char,
) -> *mut Document {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    if context_element.is_null() {
        set_last_error("null context_element pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid null-terminated strings.
    let c_input = unsafe { CStr::from_ptr(input) };
    let s = match c_input.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in input: {e}"));
            return std::ptr::null_mut();
        }
    };
    let c_ctx = unsafe { CStr::from_ptr(context_element) };
    let ctx = match c_ctx.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in context_element: {e}"));
            return std::ptr::null_mut();
        }
    };
    let opts = Html5ParseOptions {
        scripting: false,
        fragment_context: Some(ctx.to_string()),
    };
    match parse_html5_with_options(s, &opts) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}
