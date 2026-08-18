//! C FFI layer for xmloxide.
//!
//! Provides a C-compatible API for using xmloxide from C/C++ and other
//! languages that support C FFI. All symbols use the `xmloxide_` prefix.
//!
//! # Error Handling
//!
//! Functions that can fail return null pointers (for pointer types) or 0
//! (for `NodeId` values). The last error message is stored in thread-local
//! storage and can be retrieved via [`xmloxide_last_error`].
//!
//! # String Ownership
//!
//! All strings returned by FFI functions are caller-owned C strings that
//! must be freed via [`xmloxide_free_string`](strings::xmloxide_free_string).
//!
//! # Safety
//!
//! All `extern "C"` functions in this module are inherently unsafe because
//! they accept raw pointers from C callers.

// FFI functions require unsafe blocks throughout.
#![allow(unsafe_code, clippy::missing_safety_doc)]

pub mod c14n;
pub mod catalog;
pub mod css;
pub mod document;
pub mod html5;
pub mod push;
pub mod reader;
pub mod sax;
pub mod serial;
pub mod strings;
pub mod tree;
pub mod validation;
pub mod xinclude;
pub mod xpath;

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

/// Structured error stored in thread-local storage.
struct StructuredError {
    message: CString,
    line: u32,
    column: u32,
    severity: i32, // 0=warning, 1=error, 2=fatal
}

/// Severity constants matching libxml2's `xmlErrorLevel`.
pub const XMLOXIDE_ERR_WARNING: i32 = 0;
pub const XMLOXIDE_ERR_ERROR: i32 = 1;
pub const XMLOXIDE_ERR_FATAL: i32 = 2;

thread_local! {
    static LAST_ERROR: RefCell<Option<StructuredError>> = const { RefCell::new(None) };
}

/// Stores an error message in thread-local storage (no location info).
fn set_last_error(msg: &str) {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok().map(|message| StructuredError {
            message,
            line: 0,
            column: 0,
            severity: XMLOXIDE_ERR_FATAL,
        });
    });
}

/// Stores a structured error with location in thread-local storage.
fn set_last_error_structured(msg: &str, line: u32, column: u32, severity: i32) {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = CString::new(msg).ok().map(|message| StructuredError {
            message,
            line,
            column,
            severity,
        });
    });
}

/// Clears the thread-local error.
fn clear_last_error() {
    LAST_ERROR.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Returns the last error message, or null if no error occurred.
///
/// The returned string is owned by the library and must NOT be freed
/// by the caller. It is valid until the next FFI call on the same thread.
#[no_mangle]
pub extern "C" fn xmloxide_last_error() -> *const c_char {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        match borrow.as_ref() {
            Some(e) => e.message.as_ptr(),
            None => std::ptr::null(),
        }
    })
}

/// Returns the line number of the last error, or 0 if unknown.
#[no_mangle]
pub extern "C" fn xmloxide_last_error_line() -> u32 {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        borrow.as_ref().map_or(0, |e| e.line)
    })
}

/// Returns the column number of the last error, or 0 if unknown.
#[no_mangle]
pub extern "C" fn xmloxide_last_error_column() -> u32 {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        borrow.as_ref().map_or(0, |e| e.column)
    })
}

/// Returns the severity of the last error.
///
/// Returns `XMLOXIDE_ERR_WARNING` (0), `XMLOXIDE_ERR_ERROR` (1), or
/// `XMLOXIDE_ERR_FATAL` (2). Returns -1 if no error occurred.
#[no_mangle]
pub extern "C" fn xmloxide_last_error_severity() -> i32 {
    LAST_ERROR.with(|cell| {
        let borrow = cell.borrow();
        borrow.as_ref().map_or(-1, |e| e.severity)
    })
}
