//! FFI wrappers for the `XmlReader` pull-based streaming API.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::ffi::{clear_last_error, set_last_error};
use crate::reader::{XmlNodeType, XmlReader};

/// FFI-safe reader that owns the input string.
///
/// The Rust `XmlReader<'a>` borrows its input, but C callers need an
/// opaque handle that owns everything. We heap-allocate the input
/// string and create a reader that borrows from it with an erased
/// lifetime. This is safe because the string is never moved or
/// reallocated while the reader exists.
/// Opaque reader handle for FFI consumers.
///
/// This struct has no public fields — C callers interact with it
/// exclusively through the `xmloxide_reader_*` functions.
pub struct FfiReader {
    /// The owned input string, heap-allocated and never moved.
    /// Must be declared before `reader` so it outlives it during drop.
    _input: Box<str>,
    /// The reader. Its lifetime is tied to `_input` but erased to `'static`.
    reader: XmlReader<'static>,
}

impl FfiReader {
    fn new(input: String) -> Self {
        let boxed: Box<str> = input.into_boxed_str();
        // SAFETY: We extend the borrow's lifetime to 'static. This is safe
        // because `_input` is heap-allocated, never moved or reallocated,
        // and outlives `reader` (fields are dropped in declaration order,
        // so `reader` is dropped before `_input`).
        let reader = unsafe {
            let static_ref: &'static str = &*(std::ptr::from_ref::<str>(&boxed));
            XmlReader::new(static_ref)
        };
        Self {
            _input: boxed,
            reader,
        }
    }
}

// --- XmlReader node type constants matching libxml2's xmlReaderTypes ---

/// No node (reader not yet advanced).
pub const XMLOXIDE_READER_NONE: i32 = 0;
/// Element start tag.
pub const XMLOXIDE_READER_ELEMENT: i32 = 1;
/// Attribute.
pub const XMLOXIDE_READER_ATTRIBUTE: i32 = 2;
/// Text node.
pub const XMLOXIDE_READER_TEXT: i32 = 3;
/// CDATA section.
pub const XMLOXIDE_READER_CDATA: i32 = 4;
/// Processing instruction.
pub const XMLOXIDE_READER_PI: i32 = 7;
/// XML comment.
pub const XMLOXIDE_READER_COMMENT: i32 = 8;
/// Document type declaration.
pub const XMLOXIDE_READER_DOCUMENT_TYPE: i32 = 10;
/// Whitespace-only text.
pub const XMLOXIDE_READER_WHITESPACE: i32 = 13;
/// Element end tag.
pub const XMLOXIDE_READER_END_ELEMENT: i32 = 15;
/// XML declaration.
pub const XMLOXIDE_READER_XML_DECLARATION: i32 = 17;
/// End of document.
pub const XMLOXIDE_READER_END_DOCUMENT: i32 = -1;

fn node_type_to_int(nt: XmlNodeType) -> i32 {
    match nt {
        XmlNodeType::None => XMLOXIDE_READER_NONE,
        XmlNodeType::Element => XMLOXIDE_READER_ELEMENT,
        XmlNodeType::EndElement => XMLOXIDE_READER_END_ELEMENT,
        XmlNodeType::Text => XMLOXIDE_READER_TEXT,
        XmlNodeType::CData => XMLOXIDE_READER_CDATA,
        XmlNodeType::Comment => XMLOXIDE_READER_COMMENT,
        XmlNodeType::ProcessingInstruction => XMLOXIDE_READER_PI,
        XmlNodeType::XmlDeclaration => XMLOXIDE_READER_XML_DECLARATION,
        XmlNodeType::DocumentType => XMLOXIDE_READER_DOCUMENT_TYPE,
        XmlNodeType::Whitespace => XMLOXIDE_READER_WHITESPACE,
        XmlNodeType::Attribute => XMLOXIDE_READER_ATTRIBUTE,
        XmlNodeType::EndDocument => XMLOXIDE_READER_END_DOCUMENT,
    }
}

fn to_c_string(s: &str) -> *mut c_char {
    match CString::new(s) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// Creates a new `XmlReader` from a null-terminated UTF-8 string.
///
/// Returns an opaque reader pointer, or null on failure.
/// The reader must be freed with [`xmloxide_reader_free`].
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_new(input: *const c_char) -> *mut FfiReader {
    if input.is_null() {
        set_last_error("null input pointer");
        return std::ptr::null_mut();
    }
    clear_last_error();
    let c_str = CStr::from_ptr(input);
    let Ok(s) = c_str.to_str() else {
        set_last_error("input is not valid UTF-8");
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(FfiReader::new(s.to_string())))
}

/// Advances the reader to the next node.
///
/// Returns 1 if the reader advanced to a node, 0 if the document ended,
/// or -1 on error.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_read(reader: *mut FfiReader) -> i32 {
    if reader.is_null() {
        return -1;
    }
    match (*reader).reader.read() {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(e) => {
            set_last_error(&e.to_string());
            -1
        }
    }
}

/// Returns the node type of the current node.
///
/// Returns one of the `XMLOXIDE_READER_*` constants.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_node_type(reader: *const FfiReader) -> i32 {
    if reader.is_null() {
        return XMLOXIDE_READER_NONE;
    }
    node_type_to_int((*reader).reader.node_type())
}

/// Returns the name of the current node, or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_name(reader: *const FfiReader) -> *mut c_char {
    if reader.is_null() {
        return std::ptr::null_mut();
    }
    match (*reader).reader.name() {
        Some(name) => to_c_string(name),
        None => std::ptr::null_mut(),
    }
}

/// Returns the local name of the current node (without prefix), or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_local_name(reader: *const FfiReader) -> *mut c_char {
    if reader.is_null() {
        return std::ptr::null_mut();
    }
    match (*reader).reader.local_name() {
        Some(name) => to_c_string(name),
        None => std::ptr::null_mut(),
    }
}

/// Returns the namespace prefix of the current node, or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_prefix(reader: *const FfiReader) -> *mut c_char {
    if reader.is_null() {
        return std::ptr::null_mut();
    }
    match (*reader).reader.prefix() {
        Some(p) => to_c_string(p),
        None => std::ptr::null_mut(),
    }
}

/// Returns the namespace URI of the current node, or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_namespace_uri(reader: *const FfiReader) -> *mut c_char {
    if reader.is_null() {
        return std::ptr::null_mut();
    }
    match (*reader).reader.namespace_uri() {
        Some(ns) => to_c_string(ns),
        None => std::ptr::null_mut(),
    }
}

/// Returns the value of the current node (text content, comment, etc.), or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_value(reader: *const FfiReader) -> *mut c_char {
    if reader.is_null() {
        return std::ptr::null_mut();
    }
    match (*reader).reader.value() {
        Some(v) => to_c_string(v),
        None => std::ptr::null_mut(),
    }
}

/// Returns the depth of the current node in the document tree.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_depth(reader: *const FfiReader) -> u32 {
    if reader.is_null() {
        return 0;
    }
    (*reader).reader.depth()
}

/// Returns whether the current element is a self-closing (empty) element.
///
/// Returns 1 for empty elements, 0 otherwise.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_is_empty_element(reader: *const FfiReader) -> i32 {
    if reader.is_null() {
        return 0;
    }
    i32::from((*reader).reader.is_empty_element())
}

/// Returns whether the current node has a value.
///
/// Returns 1 if it has a value, 0 otherwise.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_has_value(reader: *const FfiReader) -> i32 {
    if reader.is_null() {
        return 0;
    }
    i32::from((*reader).reader.has_value())
}

/// Returns the number of attributes on the current element.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_attribute_count(reader: *const FfiReader) -> usize {
    if reader.is_null() {
        return 0;
    }
    (*reader).reader.attribute_count()
}

/// Returns the value of an attribute by name on the current element, or null.
///
/// The returned string must be freed with `xmloxide_free_string`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_get_attribute(
    reader: *const FfiReader,
    name: *const c_char,
) -> *mut c_char {
    if reader.is_null() || name.is_null() {
        return std::ptr::null_mut();
    }
    let Ok(name) = CStr::from_ptr(name).to_str() else {
        return std::ptr::null_mut();
    };
    match (*reader).reader.get_attribute(name) {
        Some(v) => to_c_string(v),
        None => std::ptr::null_mut(),
    }
}

/// Moves the reader to the first attribute of the current element.
///
/// Returns 1 if successful, 0 if no attributes or not on an element.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_move_to_first_attribute(reader: *mut FfiReader) -> i32 {
    if reader.is_null() {
        return 0;
    }
    i32::from((*reader).reader.move_to_first_attribute())
}

/// Moves the reader to the next attribute of the current element.
///
/// Returns 1 if successful, 0 if no more attributes.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_move_to_next_attribute(reader: *mut FfiReader) -> i32 {
    if reader.is_null() {
        return 0;
    }
    i32::from((*reader).reader.move_to_next_attribute())
}

/// Moves the reader back to the element from an attribute.
///
/// Returns 1 if the reader was moved back, 0 if not on an attribute.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_move_to_element(reader: *mut FfiReader) -> i32 {
    if reader.is_null() {
        return 0;
    }
    i32::from((*reader).reader.move_to_element())
}

/// Frees a reader. Passing null is safe and does nothing.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_reader_free(reader: *mut FfiReader) {
    if reader.is_null() {
        return;
    }
    drop(Box::from_raw(reader));
}
