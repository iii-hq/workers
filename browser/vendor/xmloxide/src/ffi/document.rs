//! Document parsing and lifecycle FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::tree::Document;

use crate::error::ErrorSeverity;

use super::strings::to_c_string;
use super::{
    clear_last_error, set_last_error, set_last_error_structured, XMLOXIDE_ERR_ERROR,
    XMLOXIDE_ERR_FATAL, XMLOXIDE_ERR_WARNING,
};

/// Parses a null-terminated UTF-8 XML string into a document.
///
/// Returns a pointer to the document on success, or null on failure.
/// On failure, call [`xmloxide_last_error`](super::xmloxide_last_error) for details.
///
/// The returned document must be freed with [`xmloxide_free_doc`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_str(input: *const c_char) -> *mut Document {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `input` is a valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let s = match c_str.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8: {e}"));
            return std::ptr::null_mut();
        }
    };
    match Document::parse_str(s) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error_structured(
                &e.message,
                e.location.line,
                e.location.column,
                XMLOXIDE_ERR_FATAL,
            );
            std::ptr::null_mut()
        }
    }
}

/// Parses raw bytes as XML, with automatic encoding detection.
///
/// Returns a pointer to the document on success, or null on failure.
/// On failure, call [`xmloxide_last_error`](super::xmloxide_last_error) for details.
///
/// The returned document must be freed with [`xmloxide_free_doc`].
///
/// # Safety
///
/// `data` must point to `len` valid bytes.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_bytes(data: *const u8, len: usize) -> *mut Document {
    clear_last_error();
    if data.is_null() {
        set_last_error("null data pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `data` points to `len` valid bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    match Document::parse_bytes(bytes) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error_structured(
                &e.message,
                e.location.line,
                e.location.column,
                XMLOXIDE_ERR_FATAL,
            );
            std::ptr::null_mut()
        }
    }
}

/// Frees a document previously returned by a parse function.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `doc` must have been returned by `xmloxide_parse_str` or
/// `xmloxide_parse_bytes`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_doc(doc: *mut Document) {
    if !doc.is_null() {
        // SAFETY: `doc` was created by `Box::into_raw` in a parse function, and is non-null.
        unsafe {
            drop(Box::from_raw(doc));
        }
    }
}

/// Returns the XML version string from the document's XML declaration.
///
/// Returns null if no version was declared. The returned string must
/// be freed with [`xmloxide_free_string`](super::strings::xmloxide_free_string).
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_version(doc: *const Document) -> *mut c_char {
    if doc.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    match &doc.version {
        Some(v) => to_c_string(v),
        None => std::ptr::null_mut(),
    }
}

/// Returns the encoding string from the document's XML declaration.
///
/// Returns null if no encoding was declared. The returned string must
/// be freed with [`xmloxide_free_string`](super::strings::xmloxide_free_string).
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_encoding(doc: *const Document) -> *mut c_char {
    if doc.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    match &doc.encoding {
        Some(e) => to_c_string(e),
        None => std::ptr::null_mut(),
    }
}

/// Parses an HTML string into a document.
///
/// Returns a pointer to the document on success, or null on failure.
/// The returned document must be freed with [`xmloxide_free_doc`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_html(input: *const c_char) -> *mut Document {
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
    match crate::html::parse_html(s) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error_structured(
                &e.message,
                e.location.line,
                e.location.column,
                XMLOXIDE_ERR_FATAL,
            );
            std::ptr::null_mut()
        }
    }
}

/// Parses an XML file from a filesystem path.
///
/// Returns a pointer to the document on success, or null on failure.
/// The returned document must be freed with [`xmloxide_free_doc`].
///
/// # Safety
///
/// `path` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_file(path: *const c_char) -> *mut Document {
    clear_last_error();
    if path.is_null() {
        set_last_error("null path pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(path) };
    let s = match c_str.to_str() {
        Ok(s) => s,
        Err(e) => {
            set_last_error(&format!("invalid UTF-8 in path: {e}"));
            return std::ptr::null_mut();
        }
    };
    match Document::parse_file(s) {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error_structured(
                &e.message,
                e.location.line,
                e.location.column,
                XMLOXIDE_ERR_FATAL,
            );
            std::ptr::null_mut()
        }
    }
}

/// Helper to convert `ErrorSeverity` to FFI severity constant.
fn severity_to_ffi(s: ErrorSeverity) -> i32 {
    match s {
        ErrorSeverity::Warning => XMLOXIDE_ERR_WARNING,
        ErrorSeverity::Error => XMLOXIDE_ERR_ERROR,
        ErrorSeverity::Fatal => XMLOXIDE_ERR_FATAL,
    }
}

/// Returns the number of parse diagnostics (warnings + recovered errors)
/// stored on a document.
///
/// Documents parsed in recovery mode collect diagnostics during parsing.
/// Returns 0 if the document has no diagnostics or the pointer is null.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_diagnostic_count(doc: *const Document) -> usize {
    if doc.is_null() {
        return 0;
    }
    let doc = unsafe { &*doc };
    doc.diagnostics.len()
}

/// Returns the error message of the diagnostic at the given index.
///
/// Returns null if the index is out of range. The returned string must
/// be freed with [`xmloxide_free_string`](super::strings::xmloxide_free_string).
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_diagnostic_message(
    doc: *const Document,
    index: usize,
) -> *mut c_char {
    if doc.is_null() {
        return std::ptr::null_mut();
    }
    let doc = unsafe { &*doc };
    match doc.diagnostics.get(index) {
        Some(d) => to_c_string(&d.message),
        None => std::ptr::null_mut(),
    }
}

/// Returns the line number of the diagnostic at the given index.
///
/// Returns 0 if the index is out of range or the document pointer is null.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_diagnostic_line(doc: *const Document, index: usize) -> u32 {
    if doc.is_null() {
        return 0;
    }
    let doc = unsafe { &*doc };
    doc.diagnostics.get(index).map_or(0, |d| d.location.line)
}

/// Returns the column number of the diagnostic at the given index.
///
/// Returns 0 if the index is out of range or the document pointer is null.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_diagnostic_column(doc: *const Document, index: usize) -> u32 {
    if doc.is_null() {
        return 0;
    }
    let doc = unsafe { &*doc };
    doc.diagnostics.get(index).map_or(0, |d| d.location.column)
}

/// Returns the severity of the diagnostic at the given index.
///
/// Returns `XMLOXIDE_ERR_WARNING` (0), `XMLOXIDE_ERR_ERROR` (1), or
/// `XMLOXIDE_ERR_FATAL` (2). Returns -1 if out of range.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_diagnostic_severity(
    doc: *const Document,
    index: usize,
) -> i32 {
    if doc.is_null() {
        return -1;
    }
    let doc = unsafe { &*doc };
    doc.diagnostics
        .get(index)
        .map_or(-1, |d| severity_to_ffi(d.severity))
}
