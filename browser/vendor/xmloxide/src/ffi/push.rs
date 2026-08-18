//! FFI wrappers for the push/incremental parser.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use crate::ffi::{clear_last_error, set_last_error};
use crate::parser::PushParser;
use crate::tree::Document;

/// Creates a new push parser with default options.
///
/// Returns a pointer to the parser, or null on failure.
/// The parser must be freed with [`xmloxide_push_parser_free`] or consumed
/// by [`xmloxide_push_parser_finish`].
#[no_mangle]
pub extern "C" fn xmloxide_push_parser_new() -> *mut PushParser {
    clear_last_error();
    Box::into_raw(Box::new(PushParser::new()))
}

/// Feeds a chunk of raw bytes into the push parser.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_push_parser_push(
    parser: *mut PushParser,
    data: *const u8,
    len: usize,
) {
    if parser.is_null() || data.is_null() {
        return;
    }
    let parser = &mut *parser;
    let slice = std::slice::from_raw_parts(data, len);
    parser.push(slice);
}

/// Finalizes parsing and returns the constructed document.
///
/// This **consumes** the parser — the parser pointer becomes invalid after
/// this call and must not be used again. Do NOT call `xmloxide_push_parser_free`
/// on a parser that has been finished.
///
/// Returns a document pointer on success, or null on failure.
/// The returned document must be freed with `xmloxide_free_doc`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_push_parser_finish(parser: *mut PushParser) -> *mut Document {
    if parser.is_null() {
        set_last_error("null parser pointer");
        return std::ptr::null_mut();
    }
    clear_last_error();
    let parser = *Box::from_raw(parser);
    match parser.finish() {
        Ok(doc) => Box::into_raw(Box::new(doc)),
        Err(e) => {
            set_last_error(&e.to_string());
            std::ptr::null_mut()
        }
    }
}

/// Returns the number of bytes currently buffered in the push parser.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_push_parser_buffered_bytes(parser: *const PushParser) -> usize {
    if parser.is_null() {
        return 0;
    }
    (*parser).buffered_bytes()
}

/// Resets the push parser, discarding all buffered data.
///
/// After this call the parser is in the same state as a newly created one
/// and can be reused for another document.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_push_parser_reset(parser: *mut PushParser) {
    if parser.is_null() {
        return;
    }
    (*parser).reset();
}

/// Frees a push parser without finishing it.
///
/// Use this to discard a parser whose data you no longer need.
/// Passing null is safe and does nothing.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_push_parser_free(parser: *mut PushParser) {
    if parser.is_null() {
        return;
    }
    drop(Box::from_raw(parser));
}
