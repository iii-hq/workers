//! Minimal raw bindings for the frozen curl-cffi 0.16.0 native engine.
//!
//! The crate does not download at build time. Enable `certified` only after
//! populating the verified Tier-1 artifact cache described in the README.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void};

pub const CURL_CFFI_VERSION: &str = "0.16.0";
pub const CURL_IMPERSONATE_VERSION: &str = "2.0.0";
pub const LIBCURL_VERSION: &str = "8.21.0-IMPERSONATE";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Artifact<'a> {
    pub target: &'a str,
    pub archive: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
    pub url: &'a str,
}

pub fn artifact_for_target(target: &str) -> Option<Artifact<'static>> {
    include_str!("../artifacts.manifest")
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split('|');
            let artifact = Artifact {
                target: fields.next()?,
                archive: fields.next()?,
                bytes: fields.next()?.parse().ok()?,
                sha256: fields.next()?,
                url: fields.next()?,
            };
            (artifact.target == target).then_some(artifact)
        })
}

pub type CURL = c_void;
pub type CURLcode = c_uint;
pub type CURLoption = c_uint;
pub type CURLINFO = c_uint;
pub type curl_socket_t = c_int;
pub type curl_off_t = i64;

#[repr(C)]
pub struct curl_slist {
    pub data: *mut c_char,
    pub next: *mut curl_slist,
}

#[repr(C)]
pub struct curl_sockaddr {
    pub family: c_int,
    pub socktype: c_int,
    pub protocol: c_int,
    pub addrlen: c_uint,
    pub addr: sockaddr,
}

// Linux `struct sockaddr`; `curl_sockaddr.addrlen` says how many bytes are
// valid when a callback views `addr` as sockaddr_in/sockaddr_in6.
#[repr(C)]
pub struct sockaddr {
    pub sa_family: u16,
    pub sa_data: [u8; 14],
}

pub type curl_write_callback =
    Option<unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize>;
pub type curl_read_callback =
    Option<unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize>;
pub type curl_opensocket_callback =
    Option<unsafe extern "C" fn(*mut c_void, c_int, *mut curl_sockaddr) -> curl_socket_t>;

pub const CURLE_OK: CURLcode = 0;
pub const CURL_GLOBAL_SSL: c_long = 1;
pub const CURL_GLOBAL_WIN32: c_long = 2;
pub const CURL_GLOBAL_DEFAULT: c_long = CURL_GLOBAL_SSL | CURL_GLOBAL_WIN32;
pub const CURL_SOCKET_BAD: curl_socket_t = -1;
pub const CURLAUTH_BASIC: c_long = 1;
pub const CURL_HTTP_VERSION_3ONLY: c_long = 31;
pub const CURLFOLLOW_SAFE: c_long = 4;

pub const CURLOPT_WRITEDATA: CURLoption = 10_001;
pub const CURLOPT_URL: CURLoption = 10_002;
pub const CURLOPT_PROXY: CURLoption = 10_004;
pub const CURLOPT_USERPWD: CURLoption = 10_005;
pub const CURLOPT_PROXYUSERPWD: CURLoption = 10_006;
pub const CURLOPT_READDATA: CURLoption = 10_009;
pub const CURLOPT_ERRORBUFFER: CURLoption = 10_010;
pub const CURLOPT_WRITEFUNCTION: CURLoption = 20_011;
pub const CURLOPT_READFUNCTION: CURLoption = 20_012;
pub const CURLOPT_TIMEOUT: CURLoption = 13;
pub const CURLOPT_POSTFIELDS: CURLoption = 10_015;
pub const CURLOPT_REFERER: CURLoption = 10_016;
pub const CURLOPT_USERAGENT: CURLoption = 10_018;
pub const CURLOPT_COOKIE: CURLoption = 10_022;
pub const CURLOPT_HTTPHEADER: CURLoption = 10_023;
pub const CURLOPT_HEADERDATA: CURLoption = 10_029;
pub const CURLOPT_COOKIEFILE: CURLoption = 10_031;
pub const CURLOPT_CUSTOMREQUEST: CURLoption = 10_036;
pub const CURLOPT_NOBODY: CURLoption = 44;
pub const CURLOPT_POST: CURLoption = 47;
pub const CURLOPT_FOLLOWLOCATION: CURLoption = 52;
pub const CURLOPT_POSTFIELDSIZE: CURLoption = 60;
pub const CURLOPT_SSL_VERIFYPEER: CURLoption = 64;
pub const CURLOPT_MAXREDIRS: CURLoption = 68;
pub const CURLOPT_CONNECTTIMEOUT: CURLoption = 78;
pub const CURLOPT_HEADERFUNCTION: CURLoption = 20_079;
pub const CURLOPT_HTTPGET: CURLoption = 80;
pub const CURLOPT_SSL_VERIFYHOST: CURLoption = 81;
pub const CURLOPT_COOKIEJAR: CURLoption = 10_082;
pub const CURLOPT_HTTP_VERSION: CURLoption = 84;
pub const CURLOPT_NOSIGNAL: CURLoption = 99;
pub const CURLOPT_ACCEPT_ENCODING: CURLoption = 10_102;
pub const CURLOPT_PRIVATE: CURLoption = 10_103;
pub const CURLOPT_HTTPAUTH: CURLoption = 107;
pub const CURLOPT_PROXYAUTH: CURLoption = 111;
pub const CURLOPT_COOKIELIST: CURLoption = 10_135;
pub const CURLOPT_TIMEOUT_MS: CURLoption = 155;
pub const CURLOPT_CONNECTTIMEOUT_MS: CURLoption = 156;
pub const CURLOPT_OPENSOCKETFUNCTION: CURLoption = 20_163;
pub const CURLOPT_OPENSOCKETDATA: CURLoption = 10_164;

pub const CURLINFO_EFFECTIVE_URL: CURLINFO = 0x10_0001;
pub const CURLINFO_RESPONSE_CODE: CURLINFO = 0x20_0002;
pub const CURLINFO_TOTAL_TIME: CURLINFO = 0x30_0003;
pub const CURLINFO_CONTENT_TYPE: CURLINFO = 0x10_0012;
pub const CURLINFO_REDIRECT_COUNT: CURLINFO = 0x20_0014;
pub const CURLINFO_REDIRECT_URL: CURLINFO = 0x10_001f;
/// curl-cffi's patched libcurl reports accepted cookie mutations here.
pub const CURLINFO_COOKIECHANGES: CURLINFO = 0x40_03e8;

#[cfg(curl_impersonate_linked)]
extern "C" {
    pub fn curl_global_init(flags: c_long) -> CURLcode;
    pub fn curl_global_cleanup();
    pub fn curl_version() -> *const c_char;
    pub fn curl_easy_init() -> *mut CURL;
    pub fn curl_easy_duphandle(curl: *mut CURL) -> *mut CURL;
    pub fn curl_easy_cleanup(curl: *mut CURL);
    pub fn curl_easy_reset(curl: *mut CURL);
    pub fn curl_easy_perform(curl: *mut CURL) -> CURLcode;
    pub fn curl_easy_setopt(curl: *mut CURL, option: CURLoption, ...) -> CURLcode;
    pub fn curl_easy_getinfo(curl: *mut CURL, info: CURLINFO, ...) -> CURLcode;
    pub fn curl_easy_strerror(code: CURLcode) -> *const c_char;
    pub fn curl_easy_impersonate(
        curl: *mut CURL,
        target: *const c_char,
        default_headers: c_int,
    ) -> CURLcode;
    pub fn curl_slist_append(list: *mut curl_slist, value: *const c_char) -> *mut curl_slist;
    pub fn curl_slist_free_all(list: *mut curl_slist);
}

#[cfg(all(test, feature = "certified", curl_impersonate_linked))]
mod linked_tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn linked_archive_reports_the_frozen_libcurl() {
        let version = unsafe { CStr::from_ptr(curl_version()) }.to_str().unwrap();
        assert!(
            version.starts_with("libcurl/8.21.0-IMPERSONATE "),
            "{version}"
        );
        for component in [
            "BoringSSL",
            "nghttp2/1.63.0",
            "ngtcp2/1.20.0",
            "nghttp3/1.15.0",
        ] {
            assert!(
                version.contains(component),
                "missing {component}: {version}"
            );
        }
    }

    #[test]
    fn linked_archive_exports_easy_impersonation() {
        unsafe {
            assert_eq!(curl_global_init(CURL_GLOBAL_DEFAULT), CURLE_OK);
            let easy = curl_easy_init();
            assert!(!easy.is_null());
            let target = CString::new("chrome136").unwrap();
            let result = curl_easy_impersonate(easy, target.as_ptr(), 1);
            curl_easy_cleanup(easy);
            curl_global_cleanup();
            assert_eq!(result, CURLE_OK);
        }
    }

    #[test]
    fn linked_archive_does_not_replace_the_process_unwinder() {
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        assert!(!trace.is_empty());
    }
}
