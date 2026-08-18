//! XML Catalog FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::catalog::Catalog;

use super::strings::to_c_string;
use super::{clear_last_error, set_last_error};

/// Parses an XML Catalog from a null-terminated UTF-8 XML string.
///
/// Returns a pointer to the catalog on success, or null on failure.
/// The returned catalog must be freed with [`xmloxide_free_catalog`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_catalog(input: *const c_char) -> *mut Catalog {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above.
    let c_str = unsafe { CStr::from_ptr(input) };
    let Ok(s) = c_str.to_str() else {
        set_last_error("invalid UTF-8");
        return std::ptr::null_mut();
    };
    match Catalog::parse(s) {
        Ok(cat) => Box::into_raw(Box::new(cat)),
        Err(e) => {
            set_last_error(&e.message);
            std::ptr::null_mut()
        }
    }
}

/// Frees a catalog previously returned by `xmloxide_parse_catalog`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `catalog` must have been returned by `xmloxide_parse_catalog`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_catalog(catalog: *mut Catalog) {
    if !catalog.is_null() {
        // SAFETY: `catalog` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(catalog));
        }
    }
}

/// Resolves a system identifier using the catalog.
///
/// Returns a caller-owned C string with the resolved URI, or null if not found.
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `catalog` must be a valid catalog pointer. `system_id` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_catalog_resolve_system(
    catalog: *const Catalog,
    system_id: *const c_char,
) -> *mut c_char {
    if catalog.is_null() || system_id.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above.
    let catalog = unsafe { &*catalog };
    let c_id = unsafe { CStr::from_ptr(system_id) };
    let Ok(id_str) = c_id.to_str() else {
        return std::ptr::null_mut();
    };
    match catalog.resolve_system(id_str) {
        Some(resolved) => to_c_string(&resolved),
        None => std::ptr::null_mut(),
    }
}

/// Resolves a public identifier using the catalog.
///
/// Returns a caller-owned C string with the resolved URI, or null if not found.
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `catalog` must be a valid catalog pointer. `public_id` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_catalog_resolve_public(
    catalog: *const Catalog,
    public_id: *const c_char,
) -> *mut c_char {
    if catalog.is_null() || public_id.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above.
    let catalog = unsafe { &*catalog };
    let c_id = unsafe { CStr::from_ptr(public_id) };
    let Ok(id_str) = c_id.to_str() else {
        return std::ptr::null_mut();
    };
    match catalog.resolve_public(id_str) {
        Some(resolved) => to_c_string(&resolved),
        None => std::ptr::null_mut(),
    }
}

/// Resolves a URI using the catalog.
///
/// Returns a caller-owned C string with the resolved URI, or null if not found.
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `catalog` must be a valid catalog pointer. `uri` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_catalog_resolve_uri(
    catalog: *const Catalog,
    uri: *const c_char,
) -> *mut c_char {
    if catalog.is_null() || uri.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above.
    let catalog = unsafe { &*catalog };
    let c_uri = unsafe { CStr::from_ptr(uri) };
    let Ok(uri_str) = c_uri.to_str() else {
        return std::ptr::null_mut();
    };
    match catalog.resolve_uri(uri_str) {
        Some(resolved) => to_c_string(&resolved),
        None => std::ptr::null_mut(),
    }
}
