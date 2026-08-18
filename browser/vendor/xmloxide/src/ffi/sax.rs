//! FFI wrappers for the SAX2 streaming parser.
#![allow(unsafe_code, clippy::missing_safety_doc)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::ffi::{clear_last_error, set_last_error};
use crate::parser::ParseOptions;
use crate::sax::{self, SaxHandler};

/// C function pointer type for `start_element` events.
///
/// Arguments: `local_name`, `prefix` (may be null), `namespace` (may be null),
/// `attr_names` array, `attr_values` array, `attr_count`, `user_data`.
pub type StartElementCb = Option<
    unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        *const c_char,
        *const *const c_char,
        *const *const c_char,
        usize,
        *mut std::ffi::c_void,
    ),
>;

/// C function pointer type for `end_element` events.
///
/// Arguments: `local_name`, `prefix` (may be null), `namespace` (may be null),
/// `user_data`.
pub type EndElementCb = Option<
    unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, *mut std::ffi::c_void),
>;

/// C function pointer type for `characters` / `cdata` / `comment` events.
///
/// Arguments: `content`, `user_data`.
pub type TextCb = Option<unsafe extern "C" fn(*const c_char, *mut std::ffi::c_void)>;

/// C function pointer type for `processing_instruction` events.
///
/// Arguments: `target`, `data` (may be null), `user_data`.
pub type PiCb = Option<unsafe extern "C" fn(*const c_char, *const c_char, *mut std::ffi::c_void)>;

/// A SAX handler specified as C function pointers.
///
/// Set any callback to `NULL` to ignore that event type.
/// `user_data` is passed through to every callback.
#[repr(C)]
pub struct XmloxideSaxHandler {
    pub start_element: StartElementCb,
    pub end_element: EndElementCb,
    pub characters: TextCb,
    pub cdata: TextCb,
    pub comment: TextCb,
    pub processing_instruction: PiCb,
    pub user_data: *mut std::ffi::c_void,
}

/// Bridge that implements the Rust `SaxHandler` trait by forwarding events
/// to C function pointers.
struct FfiSaxBridge {
    handler: *const XmloxideSaxHandler,
}

impl SaxHandler for FfiSaxBridge {
    fn start_element(
        &mut self,
        local_name: &str,
        prefix: Option<&str>,
        namespace: Option<&str>,
        attributes: &[(String, String, Option<String>, Option<String>)],
    ) {
        // SAFETY: handler pointer validity is the caller's responsibility.
        let h = unsafe { &*self.handler };
        let Some(cb) = h.start_element else { return };

        let c_local = CString::new(local_name).unwrap_or_default();
        let c_prefix = prefix.and_then(|s| CString::new(s).ok());
        let c_ns = namespace.and_then(|s| CString::new(s).ok());

        // Build parallel arrays of attribute names and values.
        let c_names: Vec<CString> = attributes
            .iter()
            .filter_map(|(name, _, _, _)| CString::new(name.as_str()).ok())
            .collect();
        let c_values: Vec<CString> = attributes
            .iter()
            .filter_map(|(_, value, _, _)| CString::new(value.as_str()).ok())
            .collect();
        let name_ptrs: Vec<*const c_char> = c_names.iter().map(|s| s.as_ptr()).collect();
        let value_ptrs: Vec<*const c_char> = c_values.iter().map(|s| s.as_ptr()).collect();

        unsafe {
            cb(
                c_local.as_ptr(),
                c_prefix.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                c_ns.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                name_ptrs.as_ptr(),
                value_ptrs.as_ptr(),
                c_names.len(),
                h.user_data,
            );
        }
    }

    fn end_element(&mut self, local_name: &str, prefix: Option<&str>, namespace: Option<&str>) {
        let h = unsafe { &*self.handler };
        let Some(cb) = h.end_element else { return };

        let c_local = CString::new(local_name).unwrap_or_default();
        let c_prefix = prefix.and_then(|s| CString::new(s).ok());
        let c_ns = namespace.and_then(|s| CString::new(s).ok());

        unsafe {
            cb(
                c_local.as_ptr(),
                c_prefix.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                c_ns.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                h.user_data,
            );
        }
    }

    fn characters(&mut self, content: &str) {
        let h = unsafe { &*self.handler };
        let Some(cb) = h.characters else { return };
        let c_content = CString::new(content).unwrap_or_default();
        unsafe { cb(c_content.as_ptr(), h.user_data) };
    }

    fn cdata(&mut self, content: &str) {
        let h = unsafe { &*self.handler };
        let Some(cb) = h.cdata else { return };
        let c_content = CString::new(content).unwrap_or_default();
        unsafe { cb(c_content.as_ptr(), h.user_data) };
    }

    fn comment(&mut self, content: &str) {
        let h = unsafe { &*self.handler };
        let Some(cb) = h.comment else { return };
        let c_content = CString::new(content).unwrap_or_default();
        unsafe { cb(c_content.as_ptr(), h.user_data) };
    }

    fn processing_instruction(&mut self, target: &str, data: Option<&str>) {
        let h = unsafe { &*self.handler };
        let Some(cb) = h.processing_instruction else {
            return;
        };
        let c_target = CString::new(target).unwrap_or_default();
        let c_data = data.and_then(|s| CString::new(s).ok());
        unsafe {
            cb(
                c_target.as_ptr(),
                c_data.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                h.user_data,
            );
        }
    }
}

/// Parses XML with SAX streaming, dispatching events to C function pointers.
///
/// `xml` must be a valid null-terminated UTF-8 C string.
/// `handler` must point to a valid `XmloxideSaxHandler` struct.
///
/// Returns 0 on success, -1 on error. Use `xmloxide_last_error()` for details.
#[no_mangle]
pub unsafe extern "C" fn xmloxide_sax_parse(
    xml: *const c_char,
    handler: *const XmloxideSaxHandler,
) -> i32 {
    if xml.is_null() || handler.is_null() {
        set_last_error("null argument");
        return -1;
    }
    clear_last_error();

    let Ok(input) = CStr::from_ptr(xml).to_str() else {
        set_last_error("invalid UTF-8 in input");
        return -1;
    };

    let mut bridge = FfiSaxBridge { handler };
    match sax::parse_sax(input, &ParseOptions::default(), &mut bridge) {
        Ok(()) => 0,
        Err(e) => {
            set_last_error(&e.to_string());
            -1
        }
    }
}
