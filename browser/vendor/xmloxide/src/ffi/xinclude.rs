//! `XInclude` processing FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use crate::tree::Document;
use crate::xinclude::{self, XIncludeOptions};

use super::{clear_last_error, set_last_error};

/// Processes `XInclude` elements in a document using file-based resolution.
///
/// Returns the number of successful inclusions, or -1 on failure.
/// On failure, call [`xmloxide_last_error`](super::xmloxide_last_error) for details.
///
/// The resolver reads included files from the filesystem relative to the
/// working directory.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_process_xincludes(doc: *mut Document) -> i32 {
    clear_last_error();
    if doc.is_null() {
        set_last_error("null document pointer");
        return -1;
    }
    // SAFETY: Null check above.
    let doc = unsafe { &mut *doc };
    let resolver = |href: &str| std::fs::read_to_string(href).ok();
    let result = xinclude::process_xincludes(doc, resolver, &XIncludeOptions::default());
    let count = i32::try_from(result.inclusions).unwrap_or(i32::MAX);
    if !result.errors.is_empty() {
        let msgs: Vec<String> = result.errors.iter().map(|e| e.message.clone()).collect();
        set_last_error(&msgs.join("; "));
    }
    count
}
