//! Validation FFI functions (DTD, `RelaxNG`, XSD, Schematron).
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::c_char;

use crate::tree::Document;
use crate::validation::dtd::{self, Dtd};
use crate::validation::relaxng::{self, RelaxNgSchema};
use crate::validation::schematron::{self, SchematronSchema};
use crate::validation::xsd::{self, XsdSchema};
use crate::validation::ValidationResult;

use super::strings::to_c_string;
use super::{clear_last_error, set_last_error};

/// Parses a DTD from a null-terminated UTF-8 string.
///
/// Returns a pointer to the DTD on success, or null on failure.
/// The returned DTD must be freed with [`xmloxide_free_dtd`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_dtd(input: *const c_char) -> *mut Dtd {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let Ok(s) = c_str.to_str() else {
        set_last_error("invalid UTF-8");
        return std::ptr::null_mut();
    };
    match dtd::parse_dtd(s) {
        Ok(dtd) => Box::into_raw(Box::new(dtd)),
        Err(e) => {
            set_last_error(&e.message);
            std::ptr::null_mut()
        }
    }
}

/// Frees a DTD previously returned by `xmloxide_parse_dtd`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `dtd` must have been returned by `xmloxide_parse_dtd`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_dtd(dtd: *mut Dtd) {
    if !dtd.is_null() {
        // SAFETY: `dtd` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(dtd));
        }
    }
}

/// Validates a document against a DTD.
///
/// Returns a pointer to the validation result on success, or null on failure.
/// The returned result must be freed with [`xmloxide_free_validation_result`].
///
/// Note: DTD validation may populate the document's `id_map`, so the
/// document pointer must be mutable.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `dtd` must be a valid DTD pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validate_dtd(
    doc: *mut Document,
    dtd: *const Dtd,
) -> *mut ValidationResult {
    clear_last_error();
    if doc.is_null() || dtd.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &mut *doc };
    let dtd = unsafe { &*dtd };
    let result = dtd::validate(doc, dtd);
    Box::into_raw(Box::new(result))
}

/// Parses a `RelaxNG` schema from a null-terminated UTF-8 XML string.
///
/// Returns a pointer to the schema on success, or null on failure.
/// The returned schema must be freed with [`xmloxide_free_relaxng`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string containing a `RelaxNG` schema.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_relaxng(input: *const c_char) -> *mut RelaxNgSchema {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let Ok(s) = c_str.to_str() else {
        set_last_error("invalid UTF-8");
        return std::ptr::null_mut();
    };
    match relaxng::parse_relaxng(s) {
        Ok(schema) => Box::into_raw(Box::new(schema)),
        Err(e) => {
            set_last_error(&e.message);
            std::ptr::null_mut()
        }
    }
}

/// Frees a `RelaxNG` schema previously returned by `xmloxide_parse_relaxng`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `schema` must have been returned by `xmloxide_parse_relaxng`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_relaxng(schema: *mut RelaxNgSchema) {
    if !schema.is_null() {
        // SAFETY: `schema` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(schema));
        }
    }
}

/// Validates a document against a `RelaxNG` schema.
///
/// Returns a pointer to the validation result on success, or null on failure.
/// The returned result must be freed with [`xmloxide_free_validation_result`].
///
/// # Safety
///
/// `doc` must be a valid document pointer. `schema` must be a valid `RelaxNG` schema pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validate_relaxng(
    doc: *const Document,
    schema: *const RelaxNgSchema,
) -> *mut ValidationResult {
    clear_last_error();
    if doc.is_null() || schema.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &*doc };
    let schema = unsafe { &*schema };
    let result = relaxng::validate(doc, schema);
    Box::into_raw(Box::new(result))
}

/// Parses an XSD schema from a null-terminated UTF-8 XML string.
///
/// Returns a pointer to the schema on success, or null on failure.
/// The returned schema must be freed with [`xmloxide_free_xsd`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string containing an XSD schema.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_xsd(input: *const c_char) -> *mut XsdSchema {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let Ok(s) = c_str.to_str() else {
        set_last_error("invalid UTF-8");
        return std::ptr::null_mut();
    };
    match xsd::parse_xsd(s) {
        Ok(schema) => Box::into_raw(Box::new(schema)),
        Err(e) => {
            set_last_error(&e.message);
            std::ptr::null_mut()
        }
    }
}

/// Frees an XSD schema previously returned by `xmloxide_parse_xsd`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `schema` must have been returned by `xmloxide_parse_xsd`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_xsd(schema: *mut XsdSchema) {
    if !schema.is_null() {
        // SAFETY: `schema` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(schema));
        }
    }
}

/// Validates a document against an XSD schema.
///
/// Returns a pointer to the validation result on success, or null on failure.
/// The returned result must be freed with [`xmloxide_free_validation_result`].
///
/// # Safety
///
/// `doc` must be a valid document pointer. `schema` must be a valid XSD schema pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validate_xsd(
    doc: *const Document,
    schema: *const XsdSchema,
) -> *mut ValidationResult {
    clear_last_error();
    if doc.is_null() || schema.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &*doc };
    let schema = unsafe { &*schema };
    let result = xsd::validate_xsd(doc, schema);
    Box::into_raw(Box::new(result))
}

/// Returns whether the validation result indicates a valid document.
///
/// Returns 1 for valid, 0 for invalid or null.
///
/// # Safety
///
/// `result` must be a valid validation result pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validation_is_valid(result: *const ValidationResult) -> i32 {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer.
    let result = unsafe { &*result };
    i32::from(result.is_valid)
}

/// Returns the number of validation errors.
///
/// Returns 0 if the result is null.
///
/// # Safety
///
/// `result` must be a valid validation result pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validation_error_count(result: *const ValidationResult) -> usize {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above.
    let result = unsafe { &*result };
    result.errors.len()
}

/// Returns the error message at the given index.
///
/// Returns null if the index is out of range. The returned string must be
/// freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `result` must be a valid validation result pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validation_error_message(
    result: *const ValidationResult,
    index: usize,
) -> *mut c_char {
    if result.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above.
    let result = unsafe { &*result };
    match result.errors.get(index) {
        Some(err) => to_c_string(&err.to_string()),
        None => std::ptr::null_mut(),
    }
}

/// Returns the number of validation warnings.
///
/// Returns 0 if the result is null.
///
/// # Safety
///
/// `result` must be a valid validation result pointer, or null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validation_warning_count(
    result: *const ValidationResult,
) -> usize {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above.
    let result = unsafe { &*result };
    result.warnings.len()
}

/// Returns the warning message at the given index.
///
/// Returns null if the index is out of range. The returned string must be
/// freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `result` must be a valid validation result pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validation_warning_message(
    result: *const ValidationResult,
    index: usize,
) -> *mut c_char {
    if result.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above.
    let result = unsafe { &*result };
    match result.warnings.get(index) {
        Some(warn) => to_c_string(&warn.to_string()),
        None => std::ptr::null_mut(),
    }
}

/// Parses an ISO Schematron schema from a null-terminated UTF-8 XML string.
///
/// Returns a pointer to the schema on success, or null on failure.
/// The returned schema must be freed with [`xmloxide_free_schematron`].
///
/// # Safety
///
/// `input` must be a valid null-terminated UTF-8 string containing a Schematron schema.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_parse_schematron(input: *const c_char) -> *mut SchematronSchema {
    clear_last_error();
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees valid null-terminated string.
    let c_str = unsafe { CStr::from_ptr(input) };
    let Ok(s) = c_str.to_str() else {
        set_last_error("invalid UTF-8");
        return std::ptr::null_mut();
    };
    match schematron::parse_schematron(s) {
        Ok(schema) => Box::into_raw(Box::new(schema)),
        Err(e) => {
            set_last_error(&e.message);
            std::ptr::null_mut()
        }
    }
}

/// Frees a Schematron schema previously returned by `xmloxide_parse_schematron`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `schema` must have been returned by `xmloxide_parse_schematron`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_schematron(schema: *mut SchematronSchema) {
    if !schema.is_null() {
        // SAFETY: `schema` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(schema));
        }
    }
}

/// Validates a document against an ISO Schematron schema.
///
/// Returns a pointer to the validation result on success, or null on failure.
/// The returned result must be freed with [`xmloxide_free_validation_result`].
///
/// # Safety
///
/// `doc` must be a valid document pointer. `schema` must be a valid Schematron schema pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validate_schematron(
    doc: *const Document,
    schema: *const SchematronSchema,
) -> *mut ValidationResult {
    clear_last_error();
    if doc.is_null() || schema.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &*doc };
    let schema = unsafe { &*schema };
    let result = schematron::validate_schematron(doc, schema);
    Box::into_raw(Box::new(result))
}

/// Validates a document against a Schematron schema using a specific phase.
///
/// `phase` is the name of the phase to activate (null-terminated UTF-8).
/// If `phase` is null, all patterns are active (equivalent to
/// [`xmloxide_validate_schematron`]).
///
/// Returns a pointer to the validation result on success, or null on failure.
/// The returned result must be freed with [`xmloxide_free_validation_result`].
///
/// # Safety
///
/// `doc` and `schema` must be valid pointers. `phase` must be a valid
/// null-terminated UTF-8 string, or null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_validate_schematron_with_phase(
    doc: *const Document,
    schema: *const SchematronSchema,
    phase: *const c_char,
) -> *mut ValidationResult {
    clear_last_error();
    if doc.is_null() || schema.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &*doc };
    let schema = unsafe { &*schema };

    if phase.is_null() {
        let result = schematron::validate_schematron(doc, schema);
        return Box::into_raw(Box::new(result));
    }

    // SAFETY: Null check above.
    let phase_raw = unsafe { CStr::from_ptr(phase) };
    let Ok(phase_name) = phase_raw.to_str() else {
        set_last_error("invalid UTF-8 in phase name");
        return std::ptr::null_mut();
    };
    let result = schematron::validate_schematron_with_phase(doc, schema, phase_name);
    Box::into_raw(Box::new(result))
}

/// Frees a validation result previously returned by a validate function.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `result` must have been returned by a validate function, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_free_validation_result(result: *mut ValidationResult) {
    if !result.is_null() {
        // SAFETY: `result` was created by `Box::into_raw`, and is non-null.
        unsafe {
            drop(Box::from_raw(result));
        }
    }
}
