//! Tree navigation and node inspection FFI functions.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::os::raw::c_char;

use crate::tree::{Document, NodeId, NodeKind};

use super::strings::to_c_string;

// Node type constants matching common XML conventions.

/// Element node type constant.
pub const XMLOXIDE_NODE_ELEMENT: i32 = 1;
/// Text node type constant.
pub const XMLOXIDE_NODE_TEXT: i32 = 3;
/// CDATA section node type constant.
pub const XMLOXIDE_NODE_CDATA: i32 = 4;
/// Entity reference node type constant.
pub const XMLOXIDE_NODE_ENTITY_REF: i32 = 5;
/// Processing instruction node type constant.
pub const XMLOXIDE_NODE_PI: i32 = 7;
/// Comment node type constant.
pub const XMLOXIDE_NODE_COMMENT: i32 = 8;
/// Document node type constant.
pub const XMLOXIDE_NODE_DOCUMENT: i32 = 9;
/// Document type node type constant.
pub const XMLOXIDE_NODE_DOCUMENT_TYPE: i32 = 10;

/// Helper to convert `Option<NodeId>` to a raw u32 (0 = no node).
fn node_id_to_raw(id: Option<NodeId>) -> u32 {
    id.map_or(0, NodeId::into_raw)
}

/// Helper to safely dereference a document pointer and node id.
///
/// Returns `None` if either the document is null or the raw node id is 0.
unsafe fn doc_and_node(doc: *const Document, raw_node: u32) -> Option<(&'static Document, NodeId)> {
    if doc.is_null() {
        return None;
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    let node_id = NodeId::from_raw(raw_node)?;
    Some((doc, node_id))
}

/// Returns the document root node id.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_root(doc: *const Document) -> u32 {
    if doc.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    doc.root().into_raw()
}

/// Returns the root element of the document, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_doc_root_element(doc: *const Document) -> u32 {
    if doc.is_null() {
        return 0;
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &*doc };
    node_id_to_raw(doc.root_element())
}

/// Returns the parent of a node, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_parent(doc: *const Document, node: u32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    node_id_to_raw(doc.parent(node_id))
}

/// Returns the first child of a node, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_first_child(doc: *const Document, node: u32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    node_id_to_raw(doc.first_child(node_id))
}

/// Returns the last child of a node, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_last_child(doc: *const Document, node: u32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    node_id_to_raw(doc.last_child(node_id))
}

/// Returns the next sibling of a node, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_next_sibling(doc: *const Document, node: u32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    node_id_to_raw(doc.next_sibling(node_id))
}

/// Returns the previous sibling of a node, or 0 if none.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_prev_sibling(doc: *const Document, node: u32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    node_id_to_raw(doc.prev_sibling(node_id))
}

/// Returns the node type as an integer constant.
///
/// Returns -1 if the document or node is invalid.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_type(doc: *const Document, node: u32) -> i32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return -1;
    };
    match &doc.node(node_id).kind {
        NodeKind::Document => XMLOXIDE_NODE_DOCUMENT,
        NodeKind::Element { .. } => XMLOXIDE_NODE_ELEMENT,
        NodeKind::Text { .. } => XMLOXIDE_NODE_TEXT,
        NodeKind::CData { .. } => XMLOXIDE_NODE_CDATA,
        NodeKind::Comment { .. } => XMLOXIDE_NODE_COMMENT,
        NodeKind::ProcessingInstruction { .. } => XMLOXIDE_NODE_PI,
        NodeKind::EntityRef { .. } => XMLOXIDE_NODE_ENTITY_REF,
        NodeKind::DocumentType { .. } => XMLOXIDE_NODE_DOCUMENT_TYPE,
    }
}

/// Returns the name of a node (element local name or PI target).
///
/// Returns null for node types that have no name. The returned string
/// must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_name(doc: *const Document, node: u32) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    match doc.node_name(node_id) {
        Some(name) => to_c_string(name),
        None => std::ptr::null_mut(),
    }
}

/// Returns the direct text content of a text, comment, CDATA, or PI node.
///
/// Returns null for element and document nodes. The returned string
/// must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_text(doc: *const Document, node: u32) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    match doc.node_text(node_id) {
        Some(text) => to_c_string(text),
        None => std::ptr::null_mut(),
    }
}

/// Returns the concatenated text content of a node and all descendants.
///
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_text_content(
    doc: *const Document,
    node: u32,
) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    let text = doc.text_content(node_id);
    to_c_string(&text)
}

/// Returns the namespace URI of an element node, or null if none.
///
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_namespace(doc: *const Document, node: u32) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    match doc.node_namespace(node_id) {
        Some(ns) => to_c_string(ns),
        None => std::ptr::null_mut(),
    }
}

/// Returns the value of an attribute by name on an element node.
///
/// Returns null if the attribute is not present. The returned string
/// must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer. `name` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_attribute(
    doc: *const Document,
    node: u32,
    name: *const c_char,
) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    if name.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: Null check above. Caller guarantees `name` is a valid null-terminated string.
    let c_name = unsafe { std::ffi::CStr::from_ptr(name) };
    let Ok(name_str) = c_name.to_str() else {
        return std::ptr::null_mut();
    };
    match doc.attribute(node_id, name_str) {
        Some(val) => to_c_string(val),
        None => std::ptr::null_mut(),
    }
}

/// Returns the number of attributes on an element node.
///
/// Returns 0 for non-element nodes.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_attribute_count(doc: *const Document, node: u32) -> usize {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return 0;
    };
    doc.attributes(node_id).len()
}

/// Returns the name of the attribute at the given index.
///
/// Returns null if the index is out of range. The returned string must
/// be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_attribute_name_at(
    doc: *const Document,
    node: u32,
    index: usize,
) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    let attrs = doc.attributes(node_id);
    match attrs.get(index) {
        Some(attr) => to_c_string(&attr.name),
        None => std::ptr::null_mut(),
    }
}

/// Returns the value of the attribute at the given index.
///
/// Returns null if the index is out of range. The returned string must
/// be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_attribute_value_at(
    doc: *const Document,
    node: u32,
    index: usize,
) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    let attrs = doc.attributes(node_id);
    match attrs.get(index) {
        Some(attr) => to_c_string(&attr.value),
        None => std::ptr::null_mut(),
    }
}

/// Helper to safely dereference a mutable document pointer and node id.
unsafe fn doc_and_node_mut(
    doc: *mut Document,
    raw_node: u32,
) -> Option<(&'static mut Document, NodeId)> {
    if doc.is_null() {
        return None;
    }
    // SAFETY: Null check above. Caller guarantees `doc` is a valid pointer from a parse function.
    let doc = unsafe { &mut *doc };
    let node_id = NodeId::from_raw(raw_node)?;
    Some((doc, node_id))
}

/// Creates a new element node and returns its id (0 on failure).
///
/// The node is detached — use `xmloxide_append_child` to add it to the tree.
/// The returned node is owned by the document and freed when the document is freed.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `name` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_create_element(doc: *mut Document, name: *const c_char) -> u32 {
    if doc.is_null() || name.is_null() {
        return 0;
    }
    // SAFETY: Null checks above. Caller guarantees valid pointers.
    let doc = unsafe { &mut *doc };
    let c_name = unsafe { std::ffi::CStr::from_ptr(name) };
    let Ok(name_str) = c_name.to_str() else {
        return 0;
    };
    let node = doc.create_node(NodeKind::Element {
        name: name_str.to_string(),
        prefix: None,
        namespace: None,
        attributes: vec![],
    });
    node.into_raw()
}

/// Creates a new text node and returns its id (0 on failure).
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `content` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_create_text(doc: *mut Document, content: *const c_char) -> u32 {
    if doc.is_null() || content.is_null() {
        return 0;
    }
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_content = unsafe { std::ffi::CStr::from_ptr(content) };
    let Ok(text) = c_content.to_str() else {
        return 0;
    };
    let node = doc.create_node(NodeKind::Text {
        content: text.to_string(),
    });
    node.into_raw()
}

/// Creates a new comment node and returns its id (0 on failure).
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `content` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_create_comment(
    doc: *mut Document,
    content: *const c_char,
) -> u32 {
    if doc.is_null() || content.is_null() {
        return 0;
    }
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_content = unsafe { std::ffi::CStr::from_ptr(content) };
    let Ok(text) = c_content.to_str() else {
        return 0;
    };
    let node = doc.create_node(NodeKind::Comment {
        content: text.to_string(),
    });
    node.into_raw()
}

/// Appends a child node to a parent. Returns 1 on success, 0 on failure.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_append_child(doc: *mut Document, parent: u32, child: u32) -> i32 {
    let Some((doc, parent_id)) = (unsafe { doc_and_node_mut(doc, parent) }) else {
        return 0;
    };
    let Some(child_id) = NodeId::from_raw(child) else {
        return 0;
    };
    doc.append_child(parent_id, child_id);
    1
}

/// Removes a node from the tree. Returns 1 on success, 0 on failure.
///
/// The node remains in the arena but is detached from the tree.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_remove_node(doc: *mut Document, node: u32) -> i32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node_mut(doc, node) }) else {
        return 0;
    };
    doc.remove_node(node_id);
    1
}

/// Deep-clones a node and its descendants. Returns the new node id (0 on failure).
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_clone_node(doc: *mut Document, node: u32, deep: i32) -> u32 {
    let Some((doc, node_id)) = (unsafe { doc_and_node_mut(doc, node) }) else {
        return 0;
    };
    let cloned = doc.clone_node(node_id, deep != 0);
    cloned.into_raw()
}

/// Sets the text content of a node. Returns 1 on success, 0 on failure.
///
/// For text, CDATA, and comment nodes, updates the content directly.
/// For element nodes, removes all children and replaces with a text node.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `content` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_set_text_content(
    doc: *mut Document,
    node: u32,
    content: *const c_char,
) -> i32 {
    if doc.is_null() || content.is_null() {
        return 0;
    }
    let Some(node_id) = NodeId::from_raw(node) else {
        return 0;
    };
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_content = unsafe { std::ffi::CStr::from_ptr(content) };
    let Ok(text) = c_content.to_str() else {
        return 0;
    };
    i32::from(doc.set_text_content(node_id, text))
}

/// Sets an attribute on an element node. Returns 1 on success, 0 on failure.
///
/// If the attribute already exists, its value is updated.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `name` and `value` must
/// be valid null-terminated UTF-8 strings.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_set_attribute(
    doc: *mut Document,
    node: u32,
    name: *const c_char,
    value: *const c_char,
) -> i32 {
    if doc.is_null() || name.is_null() || value.is_null() {
        return 0;
    }
    let Some(node_id) = NodeId::from_raw(node) else {
        return 0;
    };
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_name = unsafe { std::ffi::CStr::from_ptr(name) };
    let c_value = unsafe { std::ffi::CStr::from_ptr(value) };
    let Ok(name_str) = c_name.to_str() else {
        return 0;
    };
    let Ok(value_str) = c_value.to_str() else {
        return 0;
    };
    i32::from(doc.set_attribute(node_id, name_str, value_str))
}

/// Removes an attribute by name from an element node.
///
/// Returns 1 if the attribute was removed, 0 if not found or not an element.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `name` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_remove_attribute(
    doc: *mut Document,
    node: u32,
    name: *const c_char,
) -> i32 {
    if doc.is_null() || name.is_null() {
        return 0;
    }
    let Some(node_id) = NodeId::from_raw(node) else {
        return 0;
    };
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_name = unsafe { std::ffi::CStr::from_ptr(name) };
    let Ok(name_str) = c_name.to_str() else {
        return 0;
    };
    i32::from(doc.remove_attribute(node_id, name_str))
}

/// Inserts a node before a reference sibling. Returns 1 on success, 0 on failure.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_insert_before(
    doc: *mut Document,
    reference: u32,
    new_child: u32,
) -> i32 {
    let Some((doc, ref_id)) = (unsafe { doc_and_node_mut(doc, reference) }) else {
        return 0;
    };
    let Some(child_id) = NodeId::from_raw(new_child) else {
        return 0;
    };
    doc.insert_before(ref_id, child_id);
    1
}

/// Returns the element with the given ID attribute, or 0 if not found.
///
/// Note: the document's `id_map` must be populated first, typically by
/// running DTD validation with `xmloxide_validate_dtd`.
///
/// # Safety
///
/// `doc` must be a valid document pointer. `id` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_element_by_id(doc: *const Document, id: *const c_char) -> u32 {
    if doc.is_null() || id.is_null() {
        return 0;
    }
    // SAFETY: Null checks above.
    let doc = unsafe { &*doc };
    let c_id = unsafe { std::ffi::CStr::from_ptr(id) };
    let Ok(id_str) = c_id.to_str() else {
        return 0;
    };
    node_id_to_raw(doc.element_by_id(id_str))
}

/// Inserts a node after a reference sibling. Returns 1 on success, 0 on failure.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_insert_after(
    doc: *mut Document,
    reference: u32,
    new_child: u32,
) -> i32 {
    let Some((doc, ref_id)) = (unsafe { doc_and_node_mut(doc, reference) }) else {
        return 0;
    };
    let Some(child_id) = NodeId::from_raw(new_child) else {
        return 0;
    };
    doc.insert_after(ref_id, child_id);
    1
}

/// Replaces a node in the tree with another. Returns 1 on success, 0 on failure.
///
/// The old node is detached and the new node takes its position.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_replace_node(
    doc: *mut Document,
    old_node: u32,
    new_node: u32,
) -> i32 {
    let Some((doc, old_id)) = (unsafe { doc_and_node_mut(doc, old_node) }) else {
        return 0;
    };
    let Some(new_id) = NodeId::from_raw(new_node) else {
        return 0;
    };
    doc.replace_node(old_id, new_id);
    1
}

/// Creates a new processing instruction node and returns its id (0 on failure).
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `target` must be a valid
/// null-terminated UTF-8 string. `data` may be null.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_create_pi(
    doc: *mut Document,
    target: *const c_char,
    data: *const c_char,
) -> u32 {
    if doc.is_null() || target.is_null() {
        return 0;
    }
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_target = unsafe { std::ffi::CStr::from_ptr(target) };
    let Ok(target_str) = c_target.to_str() else {
        return 0;
    };
    let data_str = if data.is_null() {
        None
    } else {
        let c_data = unsafe { std::ffi::CStr::from_ptr(data) };
        match c_data.to_str() {
            Ok(s) => Some(s),
            Err(_) => return 0,
        }
    };
    let node = doc.create_processing_instruction(target_str, data_str);
    node.into_raw()
}

/// Renames an element node. Returns 1 on success, 0 on failure.
///
/// # Safety
///
/// `doc` must be a valid mutable document pointer. `new_name` must be a valid
/// null-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_rename_element(
    doc: *mut Document,
    node: u32,
    new_name: *const c_char,
) -> i32 {
    if doc.is_null() || new_name.is_null() {
        return 0;
    }
    let Some(node_id) = NodeId::from_raw(node) else {
        return 0;
    };
    // SAFETY: Null checks above.
    let doc = unsafe { &mut *doc };
    let c_name = unsafe { std::ffi::CStr::from_ptr(new_name) };
    let Ok(name_str) = c_name.to_str() else {
        return 0;
    };
    i32::from(doc.rename_element(node_id, name_str))
}

/// Returns the namespace prefix of an element node, or null if none.
///
/// For example, returns `"svg"` for `<svg:rect>`.
/// The returned string must be freed with `xmloxide_free_string`.
///
/// # Safety
///
/// `doc` must be a valid document pointer.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_node_prefix(doc: *const Document, node: u32) -> *mut c_char {
    let Some((doc, node_id)) = (unsafe { doc_and_node(doc, node) }) else {
        return std::ptr::null_mut();
    };
    match doc.node_prefix(node_id) {
        Some(prefix) => to_c_string(prefix),
        None => std::ptr::null_mut(),
    }
}
