use std::collections::HashMap;
use std::ffi::{c_char, c_long, c_void, CStr, CString};
use std::ptr;
use std::sync::{Arc, Mutex, OnceLock};

use curl_impersonate_sys as curl;
use serde_json::{Map, Value};

use super::{charset_of_raw, python_scalar, Body, HttpOptions};
use crate::scrapling::page::PageData;

static GLOBAL: OnceLock<Result<(), String>> = OnceLock::new();

fn global_init() -> Result<(), String> {
    GLOBAL
        .get_or_init(|| {
            let code = unsafe { curl::curl_global_init(curl::CURL_GLOBAL_DEFAULT) };
            if code == curl::CURLE_OK {
                Ok(())
            } else {
                Err(format!("curl_global_init failed with code {code}"))
            }
        })
        .clone()
}

#[derive(Debug)]
struct Easy(*mut curl::CURL);

// libcurl permits moving an easy handle between threads as long as only one
// thread uses it at a time. CompatSession's mutex provides that exclusion.
unsafe impl Send for Easy {}

impl Easy {
    fn new() -> Result<Self, String> {
        global_init()?;
        let handle = unsafe { curl::curl_easy_init() };
        if handle.is_null() {
            Err("curl_easy_init returned null".to_string())
        } else {
            Ok(Self(handle))
        }
    }
}

impl Drop for Easy {
    fn drop(&mut self) {
        unsafe { curl::curl_easy_cleanup(self.0) };
    }
}

#[derive(Debug)]
struct SessionState {
    easy: Easy,
    cookies: Vec<String>,
}

impl SessionState {
    fn new() -> Result<Self, String> {
        Ok(Self {
            easy: Easy::new()?,
            cookies: Vec::new(),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CompatSession(Arc<Mutex<SessionState>>);

impl CompatSession {
    pub(crate) fn new() -> Result<Self, String> {
        SessionState::new().map(|state| Self(Arc::new(Mutex::new(state))))
    }
}

struct Slist(*mut curl::curl_slist);

impl Slist {
    fn new() -> Self {
        Self(ptr::null_mut())
    }

    fn append(&mut self, value: &CString) -> Result<(), String> {
        let next = unsafe { curl::curl_slist_append(self.0, value.as_ptr()) };
        if next.is_null() {
            Err("curl_slist_append ran out of memory".to_string())
        } else {
            self.0 = next;
            Ok(())
        }
    }
}

impl Drop for Slist {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { curl::curl_slist_free_all(self.0) };
        }
    }
}

#[derive(Default)]
struct Transfer {
    body: Vec<u8>,
    headers: Vec<u8>,
}

unsafe extern "C" fn write_body(
    data: *mut c_char,
    size: usize,
    count: usize,
    userdata: *mut c_void,
) -> usize {
    let Some(length) = size.checked_mul(count) else {
        return 0;
    };
    let transfer = &mut *(userdata.cast::<Transfer>());
    transfer
        .body
        .extend_from_slice(std::slice::from_raw_parts(data.cast::<u8>(), length));
    length
}

unsafe extern "C" fn write_header(
    data: *mut c_char,
    size: usize,
    count: usize,
    userdata: *mut c_void,
) -> usize {
    let Some(length) = size.checked_mul(count) else {
        return 0;
    };
    let transfer = &mut *(userdata.cast::<Transfer>());
    transfer
        .headers
        .extend_from_slice(std::slice::from_raw_parts(data.cast::<u8>(), length));
    length
}

unsafe fn set_long(easy: &Easy, option: curl::CURLoption, value: c_long) -> Result<(), String> {
    code(curl::curl_easy_setopt(easy.0, option, value))
}

unsafe fn set_ptr(easy: &Easy, option: curl::CURLoption, value: *mut c_void) -> Result<(), String> {
    code(curl::curl_easy_setopt(easy.0, option, value))
}

unsafe fn set_str(easy: &Easy, option: curl::CURLoption, value: &CString) -> Result<(), String> {
    code(curl::curl_easy_setopt(easy.0, option, value.as_ptr()))
}

fn code(value: curl::CURLcode) -> Result<(), String> {
    if value == curl::CURLE_OK {
        Ok(())
    } else {
        let detail = unsafe { CStr::from_ptr(curl::curl_easy_strerror(value)) }.to_string_lossy();
        Err(format!("curl: ({value}) {detail}"))
    }
}

fn impersonation_alias(value: &str) -> &str {
    match value {
        "chrome" => "chrome146",
        "edge" => "edge101",
        "safari" | "safari_beta" => "safari2601",
        "safari_ios" | "safari_ios_beta" => "safari260_ios",
        "chrome_android" => "chrome131_android",
        "firefox" => "firefox147",
        "tor" => "tor145",
        other => other,
    }
}

fn request_url(url: &str, params: &[(String, String)]) -> Result<CString, String> {
    let mut parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    let mut merged: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let mut old_counts = HashMap::new();
    let mut new_counts = HashMap::new();
    for (name, _) in &merged {
        *old_counts.entry(name.clone()).or_insert(0usize) += 1;
    }
    for (name, _) in params {
        *new_counts.entry(name.clone()).or_insert(0usize) += 1;
    }
    for (name, value) in params {
        if old_counts.get(name.as_str()) == Some(&1) && new_counts.get(name.as_str()) == Some(&1) {
            if let Some(existing) = merged.iter_mut().find(|(key, _)| key == name) {
                existing.1 = value.clone();
                continue;
            }
        }
        merged.push((name.clone(), value.clone()));
    }
    if !params.is_empty() {
        parsed.query_pairs_mut().clear().extend_pairs(merged.iter());
    }
    CString::new(parsed.as_str()).map_err(|_| "URL contains a NUL byte".to_string())
}

fn selected_proxy<'a>(url: &str, options: &'a HttpOptions) -> Option<&'a str> {
    if let Some(proxy) = options.proxy.as_deref() {
        return Some(proxy);
    }
    let parsed = url::Url::parse(url).ok()?;
    let scheme = parsed.scheme();
    let host = parsed.host_str();
    let lookup = |wanted: &str| {
        options
            .proxies
            .iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, value)| value.as_str())
    };
    if let Some(host) = host {
        if let Some(proxy) =
            lookup(&format!("{scheme}://{host}")).or_else(|| lookup(&format!("all://{host}")))
        {
            return Some(proxy);
        }
    }
    lookup(scheme).or_else(|| lookup("all"))
}

fn body_bytes(body: &Body) -> Result<(Vec<u8>, &'static str), String> {
    match body {
        Body::Json(value) => serde_json::to_vec(value)
            .map(|bytes| (bytes, "application/json"))
            .map_err(|error| error.to_string()),
        Body::Form(Value::Object(values)) => {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            for (name, value) in values {
                serializer.append_pair(name, &python_scalar(value));
            }
            Ok((
                serializer.finish().into_bytes(),
                "application/x-www-form-urlencoded",
            ))
        }
        Body::Form(value) => Ok((
            python_scalar(value).into_bytes(),
            "application/octet-stream",
        )),
    }
}

fn response_headers(raw: &[u8]) -> Map<String, Value> {
    let text = String::from_utf8_lossy(raw);
    let block = text
        .rsplit("\r\n\r\n")
        .find(|block| block.starts_with("HTTP/"))
        .unwrap_or("");
    let mut headers = Map::new();
    for line in block.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            if let Some(existing) = headers.get(&name).and_then(Value::as_str) {
                let combined = format!("{existing}, {value}");
                headers.insert(name, combined.into());
            } else {
                headers.insert(name, value.into());
            }
        }
    }
    headers
}

fn response_cookies(raw: &[u8]) -> Map<String, Value> {
    let mut cookies = Map::new();
    let text = String::from_utf8_lossy(raw);
    let block = text
        .rsplit("\r\n\r\n")
        .find(|block| block.starts_with("HTTP/"))
        .unwrap_or("");
    for line in block.lines().skip(1) {
        // Header names are case-insensitive; match any casing, not just the
        // two common spellings.
        let Some(value) = line
            .split_once(':')
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, value)| value)
        else {
            continue;
        };
        if let Some((name, value)) = value.trim().split(';').next().unwrap_or("").split_once('=') {
            cookies.insert(name.trim().to_string(), value.trim().into());
        }
    }
    cookies
}

fn cookie_key(line: &str) -> Option<(&str, &str, &str)> {
    let fields = line.split('\t').collect::<Vec<_>>();
    (fields.len() == 7).then(|| (fields[0], fields[2], fields[5]))
}

fn apply_cookie_changes(cookies: &mut Vec<String>, changes: *mut curl::curl_slist) {
    let mut current = changes;
    while !current.is_null() {
        let raw = unsafe { CStr::from_ptr((*current).data) }.to_string_lossy();
        if let Some((action, value)) = raw.split_once('\t') {
            if let Some(key) = cookie_key(value) {
                cookies.retain(|existing| cookie_key(existing) != Some(key));
                if action == "SET" {
                    cookies.push(value.to_string());
                }
            }
        }
        current = unsafe { (*current).next };
    }
    if !changes.is_null() {
        unsafe { curl::curl_slist_free_all(changes) };
    }
}

fn request_cookie_lines(url: &str, options: &HttpOptions) -> Result<Vec<CString>, String> {
    let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;
    let mut host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_ascii_lowercase();
    if !host.contains('.') && host.parse::<std::net::Ipv4Addr>().is_err() {
        host.push_str(".local");
    }
    options
        .cookies
        .iter()
        .map(|(name, value)| {
            CString::new(format!("{host}\tFALSE\t/\tFALSE\t0\t{name}\t{value}"))
                .map_err(|_| "cookie contains a NUL byte".to_string())
        })
        .collect()
}

fn one_attempt(
    url: &str,
    options: &HttpOptions,
    session: &mut SessionState,
) -> Result<PageData, String> {
    // curl-cffi reports setopt failures with the option number, caller value,
    // and libcurl's option hex. Keep these validations ahead of handle setup
    // so the Rust binding has the same deterministic error without relying on
    // undefined behavior at the variadic FFI boundary.
    if options.compat_timeout_ms < 0 {
        return Err(format!(
            "Failed to setopt 155 {}, curl: (43) setopt 0x9b got bad argument. See https://curl.se/libcurl/c/libcurl-errors.html first for more details.",
            options.compat_timeout_ms
        ));
    }
    if options.max_redirects < -1 {
        return Err(format!(
            "Failed to setopt 68 {}, curl: (43) setopt 0x44 got bad argument. See https://curl.se/libcurl/c/libcurl-errors.html first for more details.",
            options.max_redirects
        ));
    }
    let easy = &session.easy;
    unsafe { curl::curl_easy_reset(easy.0) };
    let final_url = request_url(url, &options.params)?;
    let mut keepalive = vec![final_url.clone()];
    let mut transfer = Transfer::default();
    let mut error_buffer = [0u8; 256];

    unsafe {
        set_str(easy, curl::CURLOPT_URL, &final_url)?;
        set_long(easy, curl::CURLOPT_NOSIGNAL, 1)?;
        set_long(
            easy,
            curl::CURLOPT_TIMEOUT_MS,
            options.compat_timeout_ms as c_long,
        )?;
        set_long(
            easy,
            curl::CURLOPT_FOLLOWLOCATION,
            options.compat_follow_redirects as c_long,
        )?;
        set_long(
            easy,
            curl::CURLOPT_MAXREDIRS,
            options.max_redirects as c_long,
        )?;
        let accept_encoding = CString::new("gzip, deflate, br, zstd").unwrap();
        set_str(easy, curl::CURLOPT_ACCEPT_ENCODING, &accept_encoding)?;
        keepalive.push(accept_encoding);
        set_ptr(
            easy,
            curl::CURLOPT_ERRORBUFFER,
            error_buffer.as_mut_ptr().cast(),
        )?;
        set_ptr(
            easy,
            curl::CURLOPT_WRITEDATA,
            (&mut transfer as *mut Transfer).cast(),
        )?;
        code(curl::curl_easy_setopt(
            easy.0,
            curl::CURLOPT_WRITEFUNCTION,
            write_body as unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize,
        ))?;
        set_ptr(
            easy,
            curl::CURLOPT_HEADERDATA,
            (&mut transfer as *mut Transfer).cast(),
        )?;
        code(curl::curl_easy_setopt(
            easy.0,
            curl::CURLOPT_HEADERFUNCTION,
            write_header as unsafe extern "C" fn(*mut c_char, usize, usize, *mut c_void) -> usize,
        ))?;
    }

    let empty = CString::new("").unwrap();
    let clear = CString::new("ALL").unwrap();
    let session_cookies = session
        .cookies
        .iter()
        .map(|value| CString::new(value.as_str()).expect("libcurl cookie lines contain no NUL"))
        .collect::<Vec<_>>();
    let request_cookies = request_cookie_lines(url, options)?;
    unsafe {
        set_str(easy, curl::CURLOPT_COOKIEFILE, &empty)?;
        set_str(easy, curl::CURLOPT_COOKIELIST, &clear)?;
        for cookie in session_cookies.iter().chain(&request_cookies) {
            set_str(easy, curl::CURLOPT_COOKIELIST, cookie)?;
        }
    }
    keepalive.extend([empty, clear]);

    if let Some(target) = options.impersonate.as_deref() {
        let target = CString::new(impersonation_alias(target))
            .map_err(|_| "impersonate contains a NUL byte".to_string())?;
        let result = unsafe { curl::curl_easy_impersonate(easy.0, target.as_ptr(), 1) };
        if result != curl::CURLE_OK {
            return Err(format!(
                "Impersonating {} is not supported",
                options.impersonate.as_deref().unwrap_or_default()
            ));
        }
        keepalive.push(target);
    }
    unsafe {
        if options.http3 {
            set_long(
                easy,
                curl::CURLOPT_HTTP_VERSION,
                curl::CURL_HTTP_VERSION_3ONLY,
            )?;
        }
        set_long(easy, curl::CURLOPT_SSL_VERIFYPEER, options.verify as c_long)?;
        set_long(
            easy,
            curl::CURLOPT_SSL_VERIFYHOST,
            if options.verify { 2 } else { 0 },
        )?;
    }

    for (option, credentials, auth_option) in [
        (
            curl::CURLOPT_USERPWD,
            options.auth.as_ref(),
            curl::CURLOPT_HTTPAUTH,
        ),
        (
            curl::CURLOPT_PROXYUSERPWD,
            options.proxy_auth.as_ref(),
            curl::CURLOPT_PROXYAUTH,
        ),
    ] {
        if let Some((username, password)) = credentials {
            let value = CString::new(format!("{username}:{password}"))
                .map_err(|_| "credentials contain a NUL byte".to_string())?;
            unsafe {
                set_str(easy, option, &value)?;
                set_long(easy, auth_option, curl::CURLAUTH_BASIC)?;
            }
            keepalive.push(value);
        }
    }
    if let Some(proxy) = selected_proxy(url, options) {
        let value = CString::new(proxy).map_err(|_| "proxy contains a NUL byte".to_string())?;
        unsafe { set_str(easy, curl::CURLOPT_PROXY, &value)? };
        keepalive.push(value);
    }

    let mut header_values = Vec::new();
    let mut headers = Slist::new();
    for (name, value) in &options.headers {
        let line = if value.is_empty() {
            format!("{name};")
        } else {
            format!("{name}: {value}")
        };
        let value = CString::new(line).map_err(|_| format!("header {name} contains a NUL byte"))?;
        headers.append(&value)?;
        header_values.push(value);
    }
    let expect = CString::new("Expect:").unwrap();
    headers.append(&expect)?;
    header_values.push(expect);

    let mut body = None;
    if options.method_allows_body()
        && (options.body.is_some() || matches!(options.method.as_str(), "post" | "put"))
    {
        let (bytes, content_type) = match &options.body {
            Some(value) => {
                let (bytes, content_type) = body_bytes(value)?;
                (bytes, Some(content_type))
            }
            None => (Vec::new(), None),
        };
        if let Some(content_type) = content_type {
            if !options
                .headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            {
                let value = CString::new(format!("Content-Type: {content_type}")).unwrap();
                headers.append(&value)?;
                header_values.push(value);
            }
        }
        unsafe {
            set_ptr(
                easy,
                curl::CURLOPT_POSTFIELDS,
                bytes.as_ptr().cast_mut().cast(),
            )?;
            set_long(easy, curl::CURLOPT_POSTFIELDSIZE, bytes.len() as c_long)?;
        }
        body = Some(bytes);
    }
    let method = CString::new(options.method.to_ascii_uppercase()).unwrap();
    if options.method != "get" {
        unsafe { set_str(easy, curl::CURLOPT_CUSTOMREQUEST, &method)? };
    }
    if !headers.0.is_null() {
        unsafe { set_ptr(easy, curl::CURLOPT_HTTPHEADER, headers.0.cast())? };
    }

    let result = unsafe { curl::curl_easy_perform(easy.0) };
    let mut changes = ptr::null_mut();
    if unsafe { curl::curl_easy_getinfo(easy.0, curl::CURLINFO_COOKIECHANGES, &mut changes) }
        == curl::CURLE_OK
    {
        apply_cookie_changes(&mut session.cookies, changes);
    }
    drop((body, method, header_values, keepalive));
    if result != curl::CURLE_OK {
        let detail = error_buffer
            .split(|byte| *byte == 0)
            .next()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| unsafe {
                CStr::from_ptr(curl::curl_easy_strerror(result))
                    .to_string_lossy()
                    .into_owned()
            });
        return Err(format!(
            "Failed to perform, curl: ({result}) {detail}. See https://curl.se/libcurl/c/libcurl-errors.html first for more details."
        ));
    }

    let mut status = 0 as c_long;
    let mut effective: *mut c_char = ptr::null_mut();
    unsafe {
        code(curl::curl_easy_getinfo(
            easy.0,
            curl::CURLINFO_RESPONSE_CODE,
            &mut status,
        ))?;
        code(curl::curl_easy_getinfo(
            easy.0,
            curl::CURLINFO_EFFECTIVE_URL,
            &mut effective,
        ))?;
    }
    let effective = if effective.is_null() {
        url.to_string()
    } else {
        unsafe { CStr::from_ptr(effective) }
            .to_string_lossy()
            .into_owned()
    };
    let headers = response_headers(&transfer.headers);
    let encoding = charset_of_raw(&headers).or_else(|| Some("utf-8".to_string()));
    let html = super::decode_body(&transfer.body, encoding.as_deref());
    Ok(PageData {
        status: u16::try_from(status).ok(),
        url: effective,
        headers,
        cookies: response_cookies(&transfer.headers),
        encoding,
        html,
        captured_xhr: vec![],
    })
}

fn blocking_fetch_in_session(
    url: &str,
    options: &HttpOptions,
    session: &mut SessionState,
) -> Result<PageData, String> {
    let attempts = options.retries.max(0) as usize;
    if attempts == 0 {
        return Err("No active session available.".to_string());
    }
    let mut last = String::new();
    for attempt in 0..attempts {
        match one_attempt(url, options, session) {
            Ok(page) => return Ok(page),
            Err(error) => {
                last = error;
                if attempt + 1 < attempts && !options.retry_delay.is_zero() {
                    std::thread::sleep(options.retry_delay);
                } else if attempt + 1 < attempts && options.retry_delay_negative {
                    return Err("sleep length must be non-negative".to_string());
                }
            }
        }
    }
    Err(last)
}

fn blocking_fetch(url: &str, options: &HttpOptions) -> Result<PageData, String> {
    let mut session = SessionState::new()?;
    blocking_fetch_in_session(url, options, &mut session)
}

pub async fn fetch_page(url: String, options: HttpOptions) -> Result<PageData, String> {
    tokio::task::spawn_blocking(move || blocking_fetch(&url, &options))
        .await
        .map_err(|error| format!("curl worker failed: {error}"))?
}

pub(crate) async fn fetch_page_with_session(
    url: String,
    options: HttpOptions,
    session: CompatSession,
) -> Result<PageData, String> {
    tokio::task::spawn_blocking(move || {
        let mut session = session
            .0
            .lock()
            .map_err(|_| "curl session lock is poisoned".to_string())?;
        blocking_fetch_in_session(&url, &options, &mut session)
    })
    .await
    .map_err(|error| format!("curl worker failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    use serde_json::json;

    use super::*;
    use crate::scrapling::fetch::HttpMode;

    fn one_request_server() -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut expected = None;
            loop {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..read]);
                if expected.is_none() {
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        expected = Some(end + 4 + length);
                    }
                }
                if expected.is_some_and(|length| bytes.len() >= length) {
                    break;
                }
            }
            sender.send(String::from_utf8(bytes).unwrap()).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Type: text/plain; charset=iso-8859-1\r\nSet-Cookie: sid=abc; Path=/\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
                )
                .unwrap();
        });
        (format!("http://{address}/endpoint"), receiver)
    }

    fn cookie_server(count: usize) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            for index in 0..count {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0u8; 4096];
                while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..read]);
                }
                sender.send(String::from_utf8(request).unwrap()).unwrap();
                let cookie = if index == 0 {
                    "Set-Cookie: stored=server; Path=/\r\n"
                } else {
                    ""
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\n{cookie}Content-Length: 2\r\nConnection: close\r\n\r\nok"
                )
                .unwrap();
            }
        });
        (format!("http://{address}"), receiver)
    }

    #[tokio::test]
    async fn persistent_session_reuses_server_cookies_but_not_request_cookies() {
        let (base, requests) = cookie_server(3);
        let session = CompatSession::new().unwrap();
        let base_options = HttpOptions::from_payload_for_mode(
            &json!({"impersonate":"", "stealthy_headers":false, "retries":1}),
            HttpMode::Compat,
        )
        .unwrap();
        fetch_page_with_session(format!("{base}/set"), base_options.clone(), session.clone())
            .await
            .unwrap();
        let mut temporary = base_options.clone();
        temporary.cookies = vec![("once".to_string(), "request".to_string())];
        fetch_page_with_session(format!("{base}/temporary"), temporary, session.clone())
            .await
            .unwrap();
        fetch_page_with_session(format!("{base}/again"), base_options, session)
            .await
            .unwrap();

        let first = requests.recv().unwrap();
        let second = requests.recv().unwrap().to_ascii_lowercase();
        let third = requests.recv().unwrap().to_ascii_lowercase();
        assert!(!first.to_ascii_lowercase().contains("cookie:"));
        assert!(second.contains("stored=server"), "{second}");
        assert!(second.contains("once=request"), "{second}");
        assert!(third.contains("stored=server"), "{third}");
        assert!(!third.contains("once=request"), "{third}");
    }

    #[test]
    fn compat_sends_delete_body_and_preserves_ordered_inputs() {
        let (url, request) = one_request_server();
        let payload = json!({
            "method": "delete",
            "json": {"delete": true},
            "params": {"b": [2, 3], "a": "x"},
            "headers": {"x-second": "2", "x-first": "1"},
            "cookies": {"second": "2", "first": "1"},
            "auth": ["user", "pass"],
            "impersonate": "chrome136",
            "stealthy_headers": false,
            "retries": 1,
            "include_html": true
        });
        let options = HttpOptions::from_payload_for_mode(&payload, HttpMode::Compat).unwrap();

        let page = blocking_fetch(&url, &options).unwrap();
        let raw = request.recv().unwrap();
        assert!(
            raw.starts_with("DELETE /endpoint?b=2&b=3&a=x HTTP/1.1\r\n"),
            "{raw}"
        );
        assert!(raw.ends_with("{\"delete\":true}"), "{raw}");
        assert!(
            raw.contains("Authorization: Basic dXNlcjpwYXNz\r\n"),
            "{raw}"
        );
        assert!(raw.contains("Cookie: second=2; first=1\r\n"), "{raw}");
        assert!(raw.find("x-second: 2").unwrap() < raw.find("x-first: 1").unwrap());
        assert_eq!(page.status, Some(201));
        assert_eq!(page.cookies["sid"], json!("abc"));
        assert_eq!(page.encoding.as_deref(), Some("iso-8859-1"));
        assert_eq!(page.html, "OK");
        let envelope = crate::scrapling::page::serialize_page(&page, &payload, true).unwrap();
        assert_eq!(envelope["status"], json!(201));
        assert_eq!(envelope["url"], json!(format!("{url}?b=2&b=3&a=x")));
        assert_eq!(envelope["headers"]["set-cookie"], json!("sid=abc; Path=/"));
        assert_eq!(envelope["cookies"]["sid"], json!("abc"));
        assert_eq!(envelope["encoding"], json!("iso-8859-1"));
        // HTML normalization belongs to the shared compatibility DOM. The
        // HTTP engine contract at this boundary is the decoded response body.
        assert_eq!(page.html, "OK");
    }

    #[test]
    fn unsupported_impersonation_matches_oracle_error() {
        let options = HttpOptions::from_payload_for_mode(
            &json!({"impersonate": "bogus", "retries": 1}),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(
            blocking_fetch("http://127.0.0.1:1", &options).unwrap_err(),
            "Impersonating bogus is not supported"
        );
    }

    #[test]
    fn invalid_signed_setopt_values_match_oracle_errors() {
        let timeout = HttpOptions::from_payload_for_mode(
            &json!({
                "timeout": -1,
                "retries": 1,
                "impersonate": "chrome136",
                "stealthy_headers": false
            }),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(
            blocking_fetch("http://127.0.0.1:1", &timeout).unwrap_err(),
            "Failed to setopt 155 -1000, curl: (43) setopt 0x9b got bad argument. See https://curl.se/libcurl/c/libcurl-errors.html first for more details."
        );

        let redirects = HttpOptions::from_payload_for_mode(
            &json!({
                "max_redirects": -2,
                "retries": 1,
                "impersonate": "chrome136",
                "stealthy_headers": false
            }),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(
            blocking_fetch("http://127.0.0.1:1", &redirects).unwrap_err(),
            "Failed to setopt 68 -2, curl: (43) setopt 0x44 got bad argument. See https://curl.se/libcurl/c/libcurl-errors.html first for more details."
        );
    }

    #[test]
    fn negative_retry_values_match_oracle_errors() {
        let retries = HttpOptions::from_payload_for_mode(
            &json!({
                "retries": -1,
                "impersonate": "chrome136",
                "stealthy_headers": false
            }),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(
            blocking_fetch("http://127.0.0.1:1", &retries).unwrap_err(),
            "No active session available."
        );

        let retry_delay = HttpOptions::from_payload_for_mode(
            &json!({
                "retries": 2,
                "retry_delay": -1,
                "impersonate": "chrome136",
                "stealthy_headers": false
            }),
            HttpMode::Compat,
        )
        .unwrap();
        assert_eq!(
            blocking_fetch("http://127.0.0.1:1", &retry_delay).unwrap_err(),
            "sleep length must be non-negative"
        );
    }

    #[test]
    fn form_values_use_python_urlencode_coercion() {
        let body = Body::Form(json!({
            "bool": true,
            "none": null,
            "obj": {"a": 1},
            "arr": [1, 2]
        }));
        let (bytes, content_type) = body_bytes(&body).unwrap();
        assert_eq!(content_type, "application/x-www-form-urlencoded");
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "bool=True&none=None&obj=%7B%27a%27%3A+1%7D&arr=%5B1%2C+2%5D"
        );
    }

    #[test]
    fn only_final_response_headers_and_cookies_are_exposed() {
        let raw = b"HTTP/1.1 302 Found\r\nSet-Cookie: stale=1\r\nX-Hop: first\r\n\r\nHTTP/1.1 200 OK\r\nSet-Cookie: final=2\r\nSet-Cookie: other=3\r\nX-Hop: second\r\nX-Hop: third\r\n\r\n";
        let headers = response_headers(raw);
        let cookies = response_cookies(raw);
        assert_eq!(headers["x-hop"], json!("second, third"));
        assert_eq!(headers["set-cookie"], json!("final=2, other=3"));
        assert_eq!(
            cookies,
            json!({"final": "2", "other": "3"})
                .as_object()
                .unwrap()
                .clone()
        );
    }

    #[test]
    fn compat_decodes_content_encoding_like_curl_cffi() {
        const GZIP_OK: &[u8] = &[
            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0xf3, 0xf7, 0x06, 0x00,
            0x2d, 0xd9, 0x36, 0xd7, 0x02, 0x00, 0x00, 0x00,
        ];
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        GZIP_OK.len()
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream.write_all(GZIP_OK).unwrap();
        });
        let options = HttpOptions::from_payload_for_mode(
            &json!({
                "retries": 1,
                "impersonate": "chrome136",
                "stealthy_headers": false
            }),
            HttpMode::Compat,
        )
        .unwrap();
        let page = blocking_fetch(&format!("http://{address}/gzip"), &options).unwrap();
        assert_eq!(page.html, "OK");
        assert_eq!(page.encoding.as_deref(), Some("utf-8"));
    }
}
