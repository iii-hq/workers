//! String lifecycle helpers for the FFI layer.
#![allow(unsafe_code)]

use std::ffi::CString;
use std::os::raw::c_char;

/// Converts a Rust `&str` to a caller-owned C string.
///
/// Returns null if the string contains interior null bytes.
pub(crate) fn to_c_string(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Frees a string previously returned by an xmloxide FFI function.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// The pointer must have been returned by an xmloxide FFI function,
/// or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: `ptr` was created by `CString::into_raw` via `to_c_string`, and is non-null.
        unsafe {
            drop(CString::from_raw(ptr));
        }
    }
}
