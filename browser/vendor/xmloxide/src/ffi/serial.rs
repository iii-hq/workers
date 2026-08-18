//! Serialization FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::serial::SerializeOptions;
use crate::tree::Document;

use super::strings::to_c_string;
use super::{clear_last_error, set_last_error};

/// Serializes a document to an XML string.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_serialize(doc: *const Document) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    let output = crate::serial::serialize(doc);
    to_c_string(&output)
}

/// Serializes a document to a pretty-printed XML string.
///
/// Uses the default two-space indentation. Returns a caller-owned C string
/// that must be freed with `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_serialize_pretty(doc: *const Document) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    let opts = SerializeOptions::default().indent(true);
    let output = crate::serial::serialize_with_options(doc, &opts);
    to_c_string(&output)
}

/// Serializes a document to a pretty-printed XML string with a custom indent.
///
/// `indent_str` is the string used for each indentation level (e.g., `"\t"`
/// or `"    "`). Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer. `indent_str` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_serialize_pretty_custom(
    doc: *const Document,
    indent_str: *const c_char,
) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    if indent_str.is_null() {
        set_last_error("null indent_str pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &*doc };
    let c_indent = unsafe { CStr::from_ptr(indent_str) };
    let Ok(indent) = c_indent.to_str() else {
        set_last_error("invalid UTF-8 in indent_str");
        return std::ptr::null_mut();
    };
    let opts = SerializeOptions::default().indent(true).indent_str(indent);
    let output = crate::serial::serialize_with_options(doc, &opts);
    to_c_string(&output)
}

/// Serializes a document to an HTML string.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_serialize_html(doc: *const Document) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    let output = crate::serial::html::serialize_html(doc);
    to_c_string(&output)
}

/// Serializes a document to an HTML5 string.
///
/// Uses the WHATWG HTML serialization algorithm: void elements are not
/// self-closed, raw text elements (`<script>`, `<style>`) are not escaped,
/// and foreign content (`SVG`/`MathML`) uses self-closing tags when empty.
///
/// Returns a caller-owned C string that must be freed with
/// `xmloxide_free_string`. Returns null on failure.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_serialize_html5(doc: *const Document) -> *mut c_char {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    let output = crate::serial::html::serialize_html5(doc);
    to_c_string(&output)
}
