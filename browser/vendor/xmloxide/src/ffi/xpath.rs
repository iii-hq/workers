//! `XPath` evaluation FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

use crate::tree::{Document, NodeId};
use crate::xpath::{XPathNode, XPathValue};

use super::strings::to_c_string;
use super::{clear_last_error, set_last_error};

/// `XPath` result type constants.
pub const XMLOXIDE_XPATH_NODESET: i32 = 1;
/// `XPath` boolean result type.
pub const XMLOXIDE_XPATH_BOOLEAN: i32 = 2;
/// `XPath` number result type.
pub const XMLOXIDE_XPATH_NUMBER: i32 = 3;
/// `XPath` string result type.
pub const XMLOXIDE_XPATH_STRING: i32 = 4;

/// Evaluates an `XPath` expression against a context node.
///
/// Returns a pointer to the result on success, or null on failure.
/// On failure, call [`xmloxide_last_error`](super::xmloxide_last_error) for details.
///
/// The returned result must be freed with [`xmloxide_xpath_free_result`].
///
/// # Safety
///
/// `doc` must be a valid document pointer. `expr` must be a valid
/// null-terminated UTF-8 string. `context_node` must be a valid node
/// id within the document (use 0 to use the document root).
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_eval(
    doc: *const Document,
    context_node: u32,
    expr: *const c_char,
) -> *mut XPathValue {
    clear_last_error();
    if doc.is_null() || expr.is_null() {
        set_last_error("null pointer argument");
        return std::ptr::null_mut();
    }
    // SAFETY: Null checks above. Caller guarantees both pointers are valid.
    let doc = unsafe { &*doc };
    // SAFETY: Null check above. Caller guarantees `expr` is a valid null-terminated string.
    let c_expr = unsafe { CStr::from_ptr(expr) };
    let Ok(expr_str) = c_expr.to_str() else {
        set_last_error("invalid UTF-8 in XPath expression");
        return std::ptr::null_mut();
    };

    let ctx_node = if context_node == 0 {
        doc.root()
    } else if let Some(id) = NodeId::from_raw(context_node) {
        id
    } else {
        set_last_error("invalid node id");
        return std::ptr::null_mut();
    };

    match crate::xpath::evaluate(doc, ctx_node, expr_str) {
        Ok(val) => Box::into_raw(Box::new(val)),
        Err(e) => {
            set_last_error(&format!("{e}"));
            std::ptr::null_mut()
        }
    }
}

/// Returns the type of an `XPath` result.
///
/// Returns one of the `XMLOXIDE_XPATH_*` constants, or -1 on error.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_result_type(result: *const XPathValue) -> i32 {
    if result.is_null() {
        return -1;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    match val {
        XPathValue::NodeSet(_) => XMLOXIDE_XPATH_NODESET,
        XPathValue::Boolean(_) => XMLOXIDE_XPATH_BOOLEAN,
        XPathValue::Number(_) => XMLOXIDE_XPATH_NUMBER,
        XPathValue::String(_) => XMLOXIDE_XPATH_STRING,
    }
}

/// Returns the boolean value of an `XPath` result.
///
/// Converts non-boolean results using `XPath` type coercion rules.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_result_boolean(result: *const XPathValue) -> i32 {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    i32::from(val.to_boolean())
}

/// Returns the numeric value of an `XPath` result.
///
/// Converts non-number results using `XPath` type coercion rules.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_result_number(result: *const XPathValue) -> f64 {
    if result.is_null() {
        return f64::NAN;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    val.to_number()
}

/// Returns the string value of an `XPath` result.
///
/// Converts non-string results using `XPath` type coercion rules.
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_result_string(result: *const XPathValue) -> *mut c_char {
    if result.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    to_c_string(&val.to_xpath_string())
}

/// Returns the number of nodes in an `XPath` nodeset result.
///
/// Returns 0 if the result is not a nodeset.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_nodeset_count(result: *const XPathValue) -> usize {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    match val {
        XPathValue::NodeSet(nodes) => nodes.len(),
        _ => 0,
    }
}

/// Returns the node id at the given index in an `XPath` nodeset result.
///
/// For attribute nodes, returns the id of the owner element (use
/// `xmloxide_xpath_nodeset_item_is_attribute`,
/// `xmloxide_xpath_nodeset_item_attr_name`, and
/// `xmloxide_xpath_nodeset_item_attr_value` to inspect the attribute).
///
/// Returns 0 if the result is not a nodeset or the index is out of bounds.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_nodeset_item(
    result: *const XPathValue,
    index: usize,
) -> u32 {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    match val {
        XPathValue::NodeSet(nodes) => nodes.get(index).map_or(0, |n| n.anchor().into_raw()),
        _ => 0,
    }
}

/// Returns 1 if the nodeset entry at `index` is an attribute node, 0
/// otherwise (including out-of-bounds and non-nodeset results).
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_nodeset_item_is_attribute(
    result: *const XPathValue,
    index: usize,
) -> c_int {
    if result.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `result` is a valid pointer from `xmloxide_xpath_eval`.
    let val = unsafe { &*result };
    match val {
        XPathValue::NodeSet(nodes) => {
            c_int::from(nodes.get(index).is_some_and(|n| n.is_attribute()))
        }
        _ => 0,
    }
}

/// Returns the qualified name of the attribute at `index` in a nodeset
/// result, or null if the entry is not an attribute.
///
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`, and
/// `doc` must be the document the result was evaluated against.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_nodeset_item_attr_name(
    doc: *const Document,
    result: *const XPathValue,
    index: usize,
) -> *mut c_char {
    // SAFETY: Null checks below; caller guarantees validity per the contract.
    let (doc, val) = unsafe {
        match (doc.as_ref(), result.as_ref()) {
            (Some(d), Some(v)) => (d, v),
            _ => return std::ptr::null_mut(),
        }
    };
    let XPathValue::NodeSet(nodes) = val else {
        return std::ptr::null_mut();
    };
    let Some(&XPathNode::Attribute { owner, index }) = nodes.get(index) else {
        return std::ptr::null_mut();
    };
    let Some(attr) = doc.attributes(owner).get(index as usize) else {
        return std::ptr::null_mut();
    };
    let qname = match &attr.prefix {
        Some(prefix) => format!("{prefix}:{}", attr.name),
        None => attr.name.clone(),
    };
    to_c_string(&qname)
}

/// Returns the value of the attribute at `index` in a nodeset result, or
/// null if the entry is not an attribute.
///
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `result` must be a valid pointer returned by `xmloxide_xpath_eval`, and
/// `doc` must be the document the result was evaluated against.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_nodeset_item_attr_value(
    doc: *const Document,
    result: *const XPathValue,
    index: usize,
) -> *mut c_char {
    // SAFETY: Null checks below; caller guarantees validity per the contract.
    let (doc, val) = unsafe {
        match (doc.as_ref(), result.as_ref()) {
            (Some(d), Some(v)) => (d, v),
            _ => return std::ptr::null_mut(),
        }
    };
    let XPathValue::NodeSet(nodes) = val else {
        return std::ptr::null_mut();
    };
    let Some(&XPathNode::Attribute { owner, index }) = nodes.get(index) else {
        return std::ptr::null_mut();
    };
    let Some(attr) = doc.attributes(owner).get(index as usize) else {
        return std::ptr::null_mut();
    };
    to_c_string(&attr.value)
}

/// Frees an `XPath` result previously returned by `xmloxide_xpath_eval`.
///
/// Passing null is safe and does nothing.
///
/// # Safety
///
/// `result` must have been returned by `xmloxide_xpath_eval`, or be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_xpath_free_result(result: *mut XPathValue) {
    if !result.is_null() {
        // SAFETY: `result` was created by `Box::into_raw` in `xmloxide_xpath_eval`, and is non-null.
        unsafe {
            drop(Box::from_raw(result));
        }
    }
}
