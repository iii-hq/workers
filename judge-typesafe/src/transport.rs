//! Bounded provider HTTP requests and retries.

use crate::client::ExecutionLimits;
use judge_contract::{ErrorCode, ProviderError, RequestOptions, RetryPolicy};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use std::{
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};
use tokio::{
    sync::Semaphore,
    time::{sleep, timeout_at, Instant},
};

const MAX_ERROR_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub(crate) struct Failure {
    pub(crate) code: ErrorCode,
    pub(crate) http_status: Option<u16>,
    pub(crate) provider_error: Option<ProviderError>,
    pub(crate) retry_after_ms: Option<u64>,
}

impl From<ErrorCode> for Failure {
    fn from(code: ErrorCode) -> Self {
        Self {
            code,
            http_status: None,
            provider_error: None,
            retry_after_ms: None,
        }
    }
}

pub(crate) fn validate_options(
    options: &RequestOptions,
    limits: ExecutionLimits,
) -> Result<(), ErrorCode> {
    limits.validate()?;
    let retry = &options.retry;
    let safe_duration = |ms| {
        ms > 0
            && i64::try_from(ms).is_ok()
            && Instant::now()
                .checked_add(Duration::from_millis(ms))
                .is_some()
    };
    if retry.max_retries > 10
        || !safe_duration(retry.backoff_initial_ms)
        || !safe_duration(retry.backoff_max_ms)
        || !safe_duration(retry.max_retry_after_ms)
        || !retry.backoff_jitter.is_finite()
        || !(0.0..=1.0).contains(&retry.backoff_jitter)
        || retry
            .http_statuses
            .iter()
            .any(|status| !(100..=599).contains(status))
        || options
            .attempt_timeout_ms
            .is_some_and(|ms| !safe_duration(ms))
    {
        return Err(ErrorCode::InvalidRequest);
    }
    request_headers(options)?;
    Ok(())
}

fn request_headers(options: &RequestOptions) -> Result<HeaderMap, ErrorCode> {
    if options.headers.len() > 64 {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut size = 0usize;
    let mut headers = HeaderMap::new();
    for (name, value) in &options.headers {
        size = size
            .checked_add(name.len())
            .and_then(|size| size.checked_add(value.len()))
            .ok_or(ErrorCode::InvalidRequest)?;
        if size > 16 * 1024 {
            return Err(ErrorCode::InvalidRequest);
        }
        let name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| ErrorCode::InvalidRequest)?;
        if matches!(
            name.as_str(),
            "authorization"
                | "proxy-authorization"
                | "proxy-authenticate"
                | "host"
                | "content-length"
                | "transfer-encoding"
                | "content-type"
                | "content-encoding"
                | "connection"
                | "keep-alive"
                | "proxy-connection"
                | "te"
                | "trailer"
                | "upgrade"
                | "expect"
        ) || headers.contains_key(&name)
        {
            return Err(ErrorCode::InvalidRequest);
        }
        let mut value = HeaderValue::from_str(value).map_err(|_| ErrorCode::InvalidRequest)?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    Ok(headers)
}

/// This guard also covers cancellation by dropping the future. Once a provider
/// rejects a request, its missing body cannot hide an accepted evaluation.
struct InFlight<'a> {
    unknown_usage: &'a AtomicBool,
    unresolved: bool,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if self.unresolved {
            self.unknown_usage.store(true, Ordering::SeqCst);
        }
    }
}

#[allow(clippy::too_many_arguments)] // Shared transport boundary used by both client operations.
pub(crate) async fn send_http(
    http: &reqwest::Client,
    permits: &Semaphore,
    key: &str,
    limits: ExecutionLimits,
    options: &RequestOptions,
    deadline: Instant,
    attempts: &AtomicUsize,
    unknown_usage: &AtomicBool,
    build: impl Fn() -> Result<reqwest::RequestBuilder, Failure>,
) -> Result<Vec<u8>, Failure> {
    validate_options(options, limits)?;
    let headers = request_headers(options)?;
    let redactor = Redactor::new(key, options);
    timeout_at(deadline, async {
        for retry in 0..=options.retry.max_retries {
            check_deadline(deadline)?;
            let result = {
                let _permit = permits.acquire().await.map_err(|_| ErrorCode::Transport)?;
                check_deadline(deadline)?;
                // Finish validation/building before counting an HTTP attempt.
                let request = build()?
                    .headers(headers.clone())
                    .bearer_auth(key)
                    .build()
                    .map_err(|_| ErrorCode::InvalidRequest)?;
                check_deadline(deadline)?;
                let attempt_deadline = options.attempt_timeout_ms.map_or(deadline, |ms| {
                    Instant::now()
                        .checked_add(Duration::from_millis(ms))
                        .unwrap_or(deadline)
                        .min(deadline)
                });
                let mut flight = InFlight {
                    unknown_usage,
                    unresolved: true,
                };
                attempts.fetch_add(1, Ordering::SeqCst);
                async {
                    let response = timeout_at(attempt_deadline, http.execute(request))
                        .await
                        .map_err(|_| ErrorCode::AttemptTimeout)?
                        .map_err(transport_error)?;
                    if !response.status().is_success() {
                        flight.unresolved = false;
                        return Err(
                            http_failure(response, limits, &redactor, attempt_deadline).await
                        );
                    }
                    let bytes = timeout_at(
                        attempt_deadline,
                        success_body(response, limits.max_response_bytes),
                    )
                    .await
                    .map_err(|_| ErrorCode::AttemptTimeout)??;
                    check_deadline(deadline)?;
                    flight.unresolved = false;
                    Ok(bytes)
                }
                .await
            }; // The permit and in-flight guard are released BEFORE backoff.
            match result {
                Ok(bytes) => return Ok(bytes),
                Err(failure) => {
                    check_deadline(deadline)?;
                    if retry == options.retry.max_retries || !retryable(&failure, &options.retry) {
                        return Err(failure);
                    }
                    let delay = retry_delay(
                        &options.retry,
                        retry,
                        failure.retry_after_ms,
                        rand::random(),
                    );
                    sleep(Duration::from_millis(delay)).await;
                }
            }
        }
        unreachable!("every final attempt returns")
    })
    .await
    .unwrap_or_else(|_| Err(ErrorCode::Deadline.into()))
}

fn check_deadline(deadline: Instant) -> Result<(), ErrorCode> {
    if Instant::now() >= deadline {
        Err(ErrorCode::Deadline)
    } else {
        Ok(())
    }
}

fn transport_error(error: reqwest::Error) -> Failure {
    if error.is_timeout() {
        ErrorCode::AttemptTimeout
    } else {
        ErrorCode::Transport
    }
    .into()
}

async fn success_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, Failure> {
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(ErrorCode::InvalidResponse.into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if chunk.len() > limit - bytes.len() {
            return Err(ErrorCode::InvalidResponse.into());
        }
        bytes
            .try_reserve_exact(chunk.len())
            .map_err(|_| ErrorCode::InvalidResponse)?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn http_failure(
    mut response: reqwest::Response,
    limits: ExecutionLimits,
    redactor: &Redactor,
    attempt_deadline: Instant,
) -> Failure {
    let status = response.status().as_u16();
    let retry_after_ms = retry_after(response.headers(), SystemTime::now());
    let limit = limits.max_response_bytes.min(MAX_ERROR_BYTES);
    let mut bytes = Vec::new();
    let mut truncated = response
        .content_length()
        .is_some_and(|size| size > limit as u64);
    loop {
        // A diagnostic timeout must not replace an already-known HTTP status
        // or retry hint. The outer total deadline/cancellation still wins.
        if Instant::now() >= attempt_deadline {
            truncated = true;
            break;
        }
        match timeout_at(attempt_deadline, response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                let count = chunk.len().min(limit - bytes.len());
                bytes.extend_from_slice(&chunk[..count]);
                if count < chunk.len() || (bytes.len() == limit && truncated) {
                    truncated = true;
                    break;
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(_)) | Err(_) => {
                truncated = true;
                break;
            }
        }
    }
    Failure {
        code: ErrorCode::Http,
        http_status: Some(status),
        provider_error: (!bytes.is_empty() || truncated)
            .then(|| redactor.provider_error(&bytes, truncated, limit)),
        retry_after_ms,
    }
}

fn retryable(failure: &Failure, policy: &RetryPolicy) -> bool {
    match failure.code {
        ErrorCode::Http => failure
            .http_status
            .is_some_and(|status| policy.http_statuses.contains(&status)),
        ErrorCode::Transport => policy.api_connection_error,
        ErrorCode::AttemptTimeout => policy.api_timeout_error,
        _ => false,
    }
}

fn retry_delay(policy: &RetryPolicy, retry: u32, retry_after_ms: Option<u64>, random: f64) -> u64 {
    if let Some(delay) =
        retry_after_ms.filter(|&ms| policy.respect_retry_after && ms <= policy.max_retry_after_ms)
    {
        return delay;
    }
    let base = policy
        .backoff_initial_ms
        .saturating_mul(1u64.checked_shl(retry).unwrap_or(u64::MAX))
        .min(policy.backoff_max_ms);
    ((base as f64 * (1.0 - policy.backoff_jitter * random)) as u64).min(base)
}

fn retry_after(headers: &HeaderMap, now: SystemTime) -> Option<u64> {
    fn number(value: &str) -> Option<u64> {
        let value = value.trim();
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(value.bytes().fold(0u64, |value, digit| {
            value
                .saturating_mul(10)
                .saturating_add(u64::from(digit - b'0'))
        }))
    }
    if let Some(ms) = headers
        .get("retry-after-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(number)
    {
        return Some(ms);
    }
    let value = headers.get("retry-after")?.to_str().ok()?.trim();
    if let Some(seconds) = number(value) {
        return Some(seconds.saturating_mul(1000));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    let delay = date.duration_since(now).unwrap_or_default().as_millis();
    Some(u64::try_from(delay).unwrap_or(u64::MAX))
}

struct Redactor {
    secrets: Vec<String>,
}

impl Redactor {
    fn new(key: &str, options: &RequestOptions) -> Self {
        let mut secrets = Vec::new();
        for secret in std::iter::once(key).chain(options.headers.values().map(String::as_str)) {
            if secret.is_empty() {
                continue;
            }
            secrets.push(secret.to_owned());
            // Malformed/truncated JSON may still contain escaped credentials.
            let escaped = serde_json::to_string(secret).expect("strings serialize");
            secrets.push(escaped[1..escaped.len() - 1].to_owned());
        }
        secrets.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        secrets.dedup();
        Self { secrets }
    }

    fn text(&self, value: &str, truncated: bool) -> String {
        let mut result = String::new();
        let mut rest = value;
        while !rest.is_empty() {
            if let Some(secret) = self
                .secrets
                .iter()
                .find(|secret| rest.starts_with(secret.as_str()))
            {
                result.push_str("[REDACTED]");
                rest = &rest[secret.len()..];
            } else if truncated && self.secrets.iter().any(|secret| secret.starts_with(rest)) {
                // The byte guard can cut through a credential. Do not return its prefix.
                result.push_str("[REDACTED]");
                break;
            } else {
                let character = rest.chars().next().expect("nonempty text");
                result.push(character);
                rest = &rest[character.len_utf8()..];
            }
        }
        result
    }

    fn json(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.text(text, false),
            Value::Array(values) => values.iter_mut().for_each(|value| self.json(value)),
            Value::Object(values) => {
                *values = std::mem::take(values)
                    .into_iter()
                    .map(|(key, mut value)| {
                        self.json(&mut value);
                        (self.text(&key, false), value)
                    })
                    .collect();
            }
            _ => {
                let original = value.to_string();
                let redacted = self.text(&original, false);
                if original != redacted {
                    *value = Value::String(redacted);
                }
            }
        }
    }

    fn provider_error(&self, bytes: &[u8], mut truncated: bool, limit: usize) -> ProviderError {
        let message = if let Ok(mut value) = serde_json::from_slice::<Value>(bytes) {
            self.json(&mut value);
            let detail = value.get_mut("detail").map(Value::take).unwrap_or(value);
            let encoded = detail.to_string();
            if encoded.len() <= limit {
                return ProviderError {
                    detail: Some(detail),
                    message: None,
                    truncated,
                };
            }
            truncated = true;
            encoded
        } else {
            // Invalid JSON can contain recoverable secrets spelled with any
            // Unicode/slash escape, not just serde's canonical spelling. Omit
            // undecodable escaped diagnostics instead of returning those tokens.
            if bytes.contains(&b'\\') {
                return ProviderError {
                    detail: None,
                    message: None,
                    truncated: true,
                };
            }
            // A replacement character at a cut code point would hide the fact
            // that the tail is a prefix of a credential. Remove only incomplete
            // trailing UTF-8; malformed bytes elsewhere still decode lossily.
            let bytes = match bytes.utf8_chunks().last() {
                Some(chunk)
                    if truncated
                        && std::str::from_utf8(chunk.invalid())
                            .is_err_and(|error| error.error_len().is_none()) =>
                {
                    &bytes[..bytes.len() - chunk.invalid().len()]
                }
                _ => bytes,
            };
            self.text(&String::from_utf8_lossy(bytes), truncated)
        };
        // Bound the JSON-encoded message too: control bytes, lossy decoding and
        // redaction can all expand the original provider body.
        let mut remaining = limit.saturating_sub(2); // JSON string quotes.
        let mut end = 0;
        for character in message.chars() {
            let encoded_bytes = match character {
                '"' | '\\' | '\u{0008}' | '\u{000c}' | '\n' | '\r' | '\t' => 2,
                '\u{0000}'..='\u{001f}' => 6,
                _ => character.len_utf8(),
            };
            if encoded_bytes > remaining {
                break;
            }
            remaining -= encoded_bytes;
            end += character.len_utf8();
        }
        truncated |= end < message.len() || limit < 2;
        ProviderError {
            detail: None,
            message: (limit >= 2).then(|| message[..end].to_owned()),
            truncated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ExecutionLimits;
    use judge_contract::{ErrorCode, RequestOptions};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    };
    use std::time::Duration;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Semaphore,
        task::{JoinHandle, JoinSet},
        time::{timeout, Instant},
    };

    struct Reply {
        status: u16,
        headers: String,
        body: Vec<u8>,
        declared_length: Option<usize>,
        delay: Duration,
        body_delay: Duration,
        body_prefix_bytes: usize,
        disconnect: bool,
    }
    impl Reply {
        fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
            Self {
                status,
                headers: String::new(),
                body: body.into(),
                declared_length: None,
                delay: Duration::ZERO,
                body_delay: Duration::ZERO,
                body_prefix_bytes: 0,
                disconnect: false,
            }
        }
    }
    struct Server {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Server {
        async fn start(reply: impl Fn(usize) -> Reply + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/evaluate", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let observed = requests.clone();
            let reply = Arc::new(reply);
            let task = tokio::spawn(async move {
                let mut pending = JoinSet::new();
                loop {
                    tokio::select! {
                        result = listener.accept() => {
                            let (mut stream, _) = result.unwrap();
                            let observed = observed.clone();
                            let reply = reply.clone();
                            pending.spawn(async move {
                                let mut bytes = Vec::new();
                                let mut buf = [0; 4096];
                                loop {
                                    let count = stream.read(&mut buf).await.unwrap_or(0);
                                    if count == 0 { return; }
                                    bytes.extend_from_slice(&buf[..count]);
                                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") { break; }
                                }
                                let index = {
                                    let mut requests = observed.lock().unwrap();
                                    requests.push(String::from_utf8_lossy(&bytes).into_owned());
                                    requests.len() - 1
                                };
                                let response = reply(index);
                                if response.disconnect { return; }
                                tokio::time::sleep(response.delay).await;
                                let length = response.declared_length.unwrap_or(response.body.len());
                                let head = format!("HTTP/1.1 {} Test\r\nContent-Length: {length}\r\nConnection: close\r\n{}\r\n", response.status, response.headers);
                                if stream.write_all(head.as_bytes()).await.is_err() { return; }
                                let prefix = response.body_prefix_bytes.min(response.body.len());
                                if stream.write_all(&response.body[..prefix]).await.is_err() { return; }
                                tokio::time::sleep(response.body_delay).await;
                                let _ = stream.write_all(&response.body[prefix..]).await;
                            });
                        }
                        _ = pending.join_next(), if !pending.is_empty() => {}
                    }
                }
            });
            Self {
                url,
                requests,
                task,
            }
        }
    }
    fn client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .unwrap()
    }
    fn options() -> RequestOptions {
        let mut options = RequestOptions::default();
        options.retry.max_retries = 0;
        options.retry.backoff_initial_ms = 1;
        options.retry.backoff_max_ms = 4;
        options.retry.backoff_jitter = 0.0;
        options
    }
    async fn call(
        server: &Server,
        options: &RequestOptions,
        limits: ExecutionLimits,
    ) -> (Result<Vec<u8>, Failure>, usize, bool) {
        let client = client();
        let attempts = AtomicUsize::new(0);
        let unknown = AtomicBool::new(false);
        let result = send_http(
            &client,
            &Semaphore::new(1),
            "test-key",
            limits,
            options,
            Instant::now() + Duration::from_secs(3),
            &attempts,
            &unknown,
            || Ok(client.get(&server.url)),
        )
        .await;
        (
            result,
            attempts.load(Ordering::SeqCst),
            unknown.load(Ordering::SeqCst),
        )
    }

    #[tokio::test]
    async fn transport_preserves_validation_detail_and_redacts_nested_secrets() {
        let server = Server::start(|_| Reply::new(422, json!({"detail":[{"loc":["body","questions","urgent"],"msg":"test-key private-header","input":{"test-key":"private-header","nested":["test-key"]}}]}).to_string())).await;
        let mut options = options();
        options
            .headers
            .insert("X-Correlation-Id".into(), "private-header".into());
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.code, ErrorCode::Http);
        assert_eq!(failure.http_status, Some(422));
        let error = failure.provider_error.unwrap();
        assert!(!error.truncated);
        assert_eq!(
            error.detail.as_ref().unwrap()[0]["loc"],
            json!(["body", "questions", "urgent"])
        );
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(!serialized.contains("test-key"));
        assert!(!serialized.contains("private-header"));
        assert_eq!(attempts, 1);
        assert!(!unknown);
        let requests = server.requests.lock().unwrap();
        assert!(requests[0].contains("x-correlation-id: private-header"));
        assert!(requests[0].contains("authorization: Bearer test-key"));
    }

    #[tokio::test]
    async fn transport_retries_429_and_529_then_succeeds_without_unknown_usage() {
        let server = Server::start(|index| {
            let mut reply = Reply::new(
                match index {
                    0 => 429,
                    1 => 529,
                    _ => 200,
                },
                b"done".to_vec(),
            );
            reply.headers = "retry-after-ms: 0\r\n".into();
            reply
        })
        .await;
        let mut options = options();
        options.retry.max_retries = 2;
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        assert_eq!(result.unwrap(), b"done");
        assert_eq!(attempts, 3);
        assert!(!unknown);
    }

    #[tokio::test]
    async fn transport_exhaustion_keeps_final_error_and_retry_delay() {
        let server = Server::start(|index| {
            let mut reply = Reply::new(503, format!("busy {index}"));
            reply.headers = "retry-after-ms: 0\r\nRetry-After: 90\r\n".into();
            reply
        })
        .await;
        let mut options = options();
        options.retry.max_retries = 2;
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.http_status, Some(503));
        assert_eq!(failure.retry_after_ms, Some(0));
        assert_eq!(
            failure.provider_error.unwrap().message.as_deref(),
            Some("busy 2")
        );
        assert_eq!(attempts, 3);
        assert!(!unknown);
    }

    #[tokio::test]
    async fn transport_dropped_or_interrupted_success_marks_usage_unknown_after_recovery() {
        for disconnect in [false, true] {
            let server = Server::start(move |index| {
                let mut reply = Reply::new(200, b"ok".to_vec());
                if index == 0 {
                    reply.disconnect = disconnect;
                    reply.declared_length = Some(20);
                }
                reply
            })
            .await;
            let mut options = options();
            options.retry.max_retries = 1;
            let (result, attempts, unknown) =
                call(&server, &options, ExecutionLimits::default()).await;
            assert_eq!(result.unwrap(), b"ok");
            assert_eq!(attempts, 2);
            assert!(unknown);
        }
    }

    #[tokio::test]
    async fn transport_rejects_malformed_or_protected_headers_before_dispatch() {
        let server = Server::start(|_| Reply::new(200, "ok")).await;
        for (name, value) in [
            ("Authorization", "evil"),
            ("Proxy-Authorization", "evil"),
            ("Host", "elsewhere"),
            ("Content-Length", "1"),
            ("Transfer-Encoding", "chunked"),
            ("Content-Type", "text/plain"),
            ("Connection", "close"),
            ("Keep-Alive", "1"),
            ("TE", "trailers"),
            ("Trailer", "X-Secret"),
            ("Upgrade", "websocket"),
            ("Proxy-Connection", "close"),
            ("bad name", "ok"),
            ("X-Test", "hello\r\nInjected: yes"),
        ] {
            let mut options = options();
            options.headers.insert(name.into(), value.into());
            let (result, attempts, unknown) =
                call(&server, &options, ExecutionLimits::default()).await;
            assert_eq!(
                result.unwrap_err().code,
                ErrorCode::InvalidRequest,
                "{name}"
            );
            assert_eq!(attempts, 0);
            assert!(!unknown);
        }
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn transport_validates_header_budgets_duplicates_and_retry_settings() {
        let limits = ExecutionLimits::default();
        let mut duplicate = options();
        duplicate.headers.insert("X-Id".into(), "a".into());
        duplicate.headers.insert("x-id".into(), "b".into());
        assert_eq!(
            validate_options(&duplicate, limits),
            Err(ErrorCode::InvalidRequest)
        );
        let mut count = options();
        for n in 0..65 {
            count.headers.insert(format!("x-{n}"), "a".into());
        }
        assert_eq!(
            validate_options(&count, limits),
            Err(ErrorCode::InvalidRequest)
        );
        let mut bytes = options();
        bytes.headers.insert("x-test".into(), "a".repeat(16 * 1024));
        assert_eq!(
            validate_options(&bytes, limits),
            Err(ErrorCode::InvalidRequest)
        );
        for change in [
            |o: &mut RequestOptions| o.retry.max_retries = 11,
            |o: &mut RequestOptions| o.retry.backoff_initial_ms = 0,
            |o: &mut RequestOptions| o.retry.backoff_max_ms = 0,
            |o: &mut RequestOptions| o.retry.backoff_initial_ms = u64::MAX,
            |o: &mut RequestOptions| o.retry.backoff_max_ms = u64::MAX,
            |o: &mut RequestOptions| o.retry.backoff_jitter = f64::NAN,
            |o: &mut RequestOptions| o.retry.backoff_jitter = -0.1,
            |o: &mut RequestOptions| o.retry.backoff_jitter = 1.01,
            |o: &mut RequestOptions| o.retry.max_retry_after_ms = 0,
            |o: &mut RequestOptions| o.retry.max_retry_after_ms = u64::MAX,
            |o: &mut RequestOptions| o.retry.http_statuses = vec![99],
            |o: &mut RequestOptions| o.retry.http_statuses = vec![600],
            |o: &mut RequestOptions| o.attempt_timeout_ms = Some(0),
            |o: &mut RequestOptions| o.attempt_timeout_ms = Some(u64::MAX),
        ] {
            let mut options = options();
            change(&mut options);
            assert_eq!(
                validate_options(&options, limits),
                Err(ErrorCode::InvalidRequest)
            );
        }
        assert_eq!(validate_options(&RequestOptions::default(), limits), Ok(()));
    }

    #[tokio::test]
    async fn transport_bounds_error_and_success_bodies_independently() {
        for status in [200, 422] {
            let server = Server::start(move |_| Reply::new(status, "x".repeat(200))).await;
            let limits = ExecutionLimits {
                max_response_bytes: 64,
                ..ExecutionLimits::default()
            };
            let (result, attempts, unknown) = call(&server, &options(), limits).await;
            let failure = result.unwrap_err();
            assert_eq!(attempts, 1);
            if status == 200 {
                assert_eq!(failure.code, ErrorCode::InvalidResponse);
                assert!(unknown);
            } else {
                assert_eq!(failure.code, ErrorCode::Http);
                let error = failure.provider_error.unwrap();
                assert!(error.truncated);
                assert!(error.message.unwrap().len() <= 64);
                assert!(!unknown);
            }
        }
    }

    #[tokio::test]
    async fn transport_attempt_timeout_covers_body_and_is_retryable() {
        let server = Server::start(|index| {
            let mut reply = Reply::new(200, "ok");
            if index == 0 {
                reply.body_delay = Duration::from_millis(200);
            }
            reply
        })
        .await;
        let mut options = options();
        options.retry.max_retries = 1;
        options.attempt_timeout_ms = Some(40);
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        assert_eq!(result.unwrap(), b"ok");
        assert_eq!(attempts, 2);
        assert!(unknown);
    }

    #[tokio::test]
    async fn transport_permit_wait_obeys_total_deadline_without_dispatch() {
        let client = client();
        let permits = Semaphore::new(0);
        let attempts = AtomicUsize::new(0);
        let unknown = AtomicBool::new(false);
        let result = timeout(
            Duration::from_secs(1),
            send_http(
                &client,
                &permits,
                "key",
                ExecutionLimits::default(),
                &options(),
                Instant::now() + Duration::from_millis(20),
                &attempts,
                &unknown,
                || panic!("no permit: no request"),
            ),
        )
        .await
        .expect("transport enforces its deadline");
        assert_eq!(result.unwrap_err().code, ErrorCode::Deadline);
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert!(!unknown.load(Ordering::SeqCst));
    }

    #[test]
    fn transport_error_message_budget_includes_json_escaping() {
        let redactor = Redactor::new("test-key", &options());
        let error = redactor.provider_error(&[0; 64], false, 64);
        assert!(
            serde_json::to_string(&error.message.unwrap())
                .unwrap()
                .len()
                <= 64
        );
        assert!(error.truncated);
    }

    #[test]
    fn transport_redacts_a_credential_cut_inside_a_utf8_character() {
        let redactor = Redactor::new("token-éclair", &options());
        for bytes in [
            b"prefix token-\xc3".as_slice(),
            b"\xff prefix token-\xc3".as_slice(),
        ] {
            let error = redactor.provider_error(bytes, true, 64);
            let message = error.message.unwrap();
            assert!(!message.contains("token"), "{message}");
            assert!(message.ends_with("[REDACTED]"), "{message}");
        }
    }

    #[test]
    fn transport_retry_after_handles_precedence_dates_invalid_and_saturation() {
        let now = std::time::UNIX_EPOCH + Duration::from_secs(784111777);
        for (ms, seconds, expected) in [
            (None, Some("12"), Some(12000)),
            (None, Some("0"), Some(0)),
            (Some("0"), Some("12"), Some(0)),
            (Some(" 25 "), Some("12"), Some(25)),
            (Some("invalid"), Some("12"), Some(12000)),
            (Some("-1"), Some("12"), Some(12000)),
            (Some("184467440737095516160000"), Some("12"), Some(u64::MAX)),
            (None, Some("18446744073709551615"), Some(u64::MAX)),
            (None, Some("Sun, 06 Nov 1994 08:49:39 GMT"), Some(2000)),
            (None, Some("Sun, 06 Nov 1994 08:49:30 GMT"), Some(0)),
            (None, Some("invalid"), None),
            (None, Some("-1"), None),
            (None, Some("NaN"), None),
            (None, Some("1second"), None),
        ] {
            let mut headers = HeaderMap::new();
            if let Some(value) = ms {
                headers.insert("retry-after-ms", HeaderValue::from_str(value).unwrap());
            }
            if let Some(value) = seconds {
                headers.insert("retry-after", HeaderValue::from_str(value).unwrap());
            }
            assert_eq!(
                retry_after(&headers, now),
                expected,
                "ms={ms:?}, seconds={seconds:?}"
            );
        }
    }

    #[test]
    fn transport_backoff_caps_before_subtractive_jitter_and_ignores_long_retry_after() {
        let mut policy = RetryPolicy::default();
        assert_eq!(retry_delay(&policy, 0, None, 0.0), 500);
        assert_eq!(retry_delay(&policy, 0, None, 1.0), 375);
        assert_eq!(retry_delay(&policy, 1, None, 0.0), 1000);
        assert_eq!(retry_delay(&policy, 10, None, 1.0), 3750);
        assert_eq!(retry_delay(&policy, 1000, None, 0.0), 5000);
        assert_eq!(retry_delay(&policy, 0, Some(60_000), 0.0), 60_000);
        assert_eq!(retry_delay(&policy, 0, Some(60_001), 0.0), 500);
        assert_eq!(retry_delay(&policy, 0, Some(u64::MAX), 0.0), 500);
        assert_eq!(retry_delay(&policy, 0, Some(0), 0.0), 0);
        policy.respect_retry_after = false;
        assert_eq!(retry_delay(&policy, 0, Some(0), 0.0), 500);
        policy.backoff_jitter = 1.0;
        assert_eq!(retry_delay(&policy, 0, None, 1.0), 0);
        policy.backoff_initial_ms = i64::MAX as u64;
        policy.backoff_max_ms = 5000;
        assert_eq!(retry_delay(&policy, 10, None, 0.0), 5000);
    }

    #[tokio::test]
    async fn transport_interrupted_http_error_keeps_status_without_unknown_usage() {
        let server = Server::start(|_| {
            let mut reply = Reply::new(422, "invalid request");
            reply.declared_length = Some(500);
            reply
        })
        .await;
        let (result, attempts, unknown) =
            call(&server, &options(), ExecutionLimits::default()).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.code, ErrorCode::Http);
        assert_eq!(failure.http_status, Some(422));
        assert!(failure.provider_error.unwrap().truncated);
        assert_eq!(attempts, 1);
        assert!(!unknown);
    }

    #[tokio::test]
    async fn transport_error_ceiling_utf8_and_truncated_secret_are_bounded_and_redacted() {
        for body in [
            vec![0xff; 100_000],
            "test-key".repeat(20_000).into_bytes(),
            b"provider echoed test-key".to_vec(),
        ] {
            let server = Server::start(move |_| Reply::new(422, body.clone())).await;
            let limits = ExecutionLimits {
                max_response_bytes: 21,
                ..ExecutionLimits::default()
            };
            let (result, _, _) = call(&server, &options(), limits).await;
            let error = result.unwrap_err().provider_error.unwrap();
            assert!(error.truncated);
            let message = error.message.unwrap();
            assert!(message.len() <= 21);
            assert!(!message.contains("test"));
        }
        let server = Server::start(|_| Reply::new(422, "x".repeat(100_000))).await;
        let (result, _, _) = call(&server, &options(), ExecutionLimits::default()).await;
        let error = result.unwrap_err().provider_error.unwrap();
        assert!(error.truncated);
        assert!(
            serde_json::to_string(&error.message.unwrap())
                .unwrap()
                .len()
                <= 64 * 1024
        );
    }

    #[tokio::test]
    async fn transport_disabling_retry_categories_preserves_one_attempt() {
        for category in [0, 1, 2] {
            let server = Server::start(move |_| {
                let mut reply = Reply::new(if category == 0 { 503 } else { 200 }, "ok");
                reply.disconnect = category == 1;
                if category == 2 {
                    reply.delay = Duration::from_millis(200);
                }
                reply
            })
            .await;
            let mut options = options();
            options.retry.max_retries = 2;
            options.retry.http_statuses.clear();
            options.retry.api_connection_error = false;
            options.retry.api_timeout_error = false;
            options.attempt_timeout_ms = Some(40);
            let (result, attempts, unknown) =
                call(&server, &options, ExecutionLimits::default()).await;
            assert_eq!(
                result.unwrap_err().code,
                [
                    ErrorCode::Http,
                    ErrorCode::Transport,
                    ErrorCode::AttemptTimeout
                ][category]
            );
            assert_eq!(attempts, 1);
            assert_eq!(unknown, category != 0);
        }
    }

    #[tokio::test]
    async fn transport_releases_permit_during_backoff_and_total_deadline_stops_retry() {
        let server = Server::start(|_| {
            let mut reply = Reply::new(429, "busy");
            reply.headers = "retry-after-ms: 500\r\n".into();
            reply
        })
        .await;
        let unrelated = Server::start(|_| Reply::new(200, "available")).await;
        let http = client();
        let permits = Semaphore::new(1);
        let attempts = AtomicUsize::new(0);
        let unknown = AtomicBool::new(false);
        let mut options = options();
        options.retry.max_retries = 2;
        let future = send_http(
            &http,
            &permits,
            "key",
            ExecutionLimits::default(),
            &options,
            Instant::now() + Duration::from_millis(100),
            &attempts,
            &unknown,
            || Ok(http.get(&server.url)),
        );
        tokio::pin!(future);
        tokio::select! {
            result = &mut future => panic!("must enter backoff first: {result:?}"),
            _ = async {
                while server.requests.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
                let other_attempts = AtomicUsize::new(0);
                let other_unknown = AtomicBool::new(false);
                let result = send_http(&http, &permits, "key", ExecutionLimits::default(), &RequestOptions::default(), Instant::now() + Duration::from_millis(70), &other_attempts, &other_unknown, || Ok(http.get(&unrelated.url))).await;
                assert_eq!(result.unwrap(), b"available");
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
                assert_eq!(other_attempts.load(Ordering::SeqCst), 1);
            } => {}
        }
        assert_eq!(future.await.unwrap_err().code, ErrorCode::Deadline);
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(!unknown.load(Ordering::SeqCst));
        assert_eq!(permits.available_permits(), 1);
    }

    #[tokio::test]
    async fn transport_total_deadline_interrupts_body_and_drop_releases_permit() {
        for cancel in [false, true] {
            let server = Server::start(|_| {
                let mut reply = Reply::new(200, "ok");
                reply.body_delay = Duration::from_secs(5);
                reply
            })
            .await;
            let http = client();
            let permits = Semaphore::new(1);
            let attempts = AtomicUsize::new(0);
            let unknown = AtomicBool::new(false);
            let deadline = Instant::now() + Duration::from_millis(if cancel { 5000 } else { 40 });
            let options = options();
            let result = timeout(
                Duration::from_millis(100),
                send_http(
                    &http,
                    &permits,
                    "key",
                    ExecutionLimits::default(),
                    &options,
                    deadline,
                    &attempts,
                    &unknown,
                    || Ok(http.get(&server.url)),
                ),
            )
            .await;
            if cancel {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap().unwrap_err().code, ErrorCode::Deadline);
            }
            assert_eq!(attempts.load(Ordering::SeqCst), 1);
            assert!(unknown.load(Ordering::SeqCst));
            assert_eq!(permits.available_permits(), 1);
        }
    }

    #[tokio::test]
    async fn transport_invalid_built_requests_do_not_count_as_dispatches() {
        let http = client();
        let attempts = AtomicUsize::new(0);
        let unknown = AtomicBool::new(false);
        let result = send_http(
            &http,
            &Semaphore::new(1),
            "test-key",
            ExecutionLimits::default(),
            &options(),
            Instant::now() + Duration::from_secs(1),
            &attempts,
            &unknown,
            || Ok(http.get("://invalid")),
        )
        .await;
        assert_eq!(result.unwrap_err().code, ErrorCode::InvalidRequest);
        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert!(!unknown.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn transport_malformed_escaped_json_omits_diagnostics_but_keeps_http_metadata() {
        let server = Server::start(|_| {
            let mut reply = Reply::new(422, r#"{"detail":"\u0074est-key","broken":"#);
            reply.headers = "retry-after-ms: 1000\r\n".into();
            reply
        })
        .await;
        let (result, attempts, unknown) =
            call(&server, &options(), ExecutionLimits::default()).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.code, ErrorCode::Http);
        assert_eq!(failure.http_status, Some(422));
        assert_eq!(failure.retry_after_ms, Some(1000));
        let error = failure.provider_error.unwrap();
        assert!(error.detail.is_none());
        assert!(error.message.is_none());
        assert!(error.truncated);
        assert_eq!(attempts, 1);
        assert!(!unknown);
    }

    #[test]
    fn transport_malformed_escape_variants_cannot_expose_keys_or_headers() {
        let mut options = options();
        options
            .headers
            .insert("X-Private".into(), "private/header".into());
        let redactor = Redactor::new("test/key", &options);
        for body in [
            r#"{"detail":"\u0074est/key","broken":"#,
            r#"{"detail":"test\u002Fkey","broken":"#,
            r#"{"detail":"test\/key","broken":"#,
            r#"{"detail":"\u0070rivate/header","broken":"#,
            r#"{"detail":"private\u002fheader","broken":"#,
            r#"{"detail":"private\/header","broken":"#,
            r#"{"detail":"test\/key""#,
            r#"{"detail":"private\/header""#,
            r#"{"detail":"\u0074est/ke"#,
            r#"{"detail":"private\u002fhea"#,
        ] {
            for truncated in [false, true] {
                let error = redactor.provider_error(body.as_bytes(), truncated, 1024);
                assert!(error.detail.is_none(), "{body}");
                assert!(error.message.is_none(), "{body}");
                assert!(error.truncated, "{body}");
            }
        }
        for body in [
            r#"{"detail":"\u0074est\/key"}"#,
            r#"{"detail":"\u0070rivate\u002Fheader"}"#,
        ] {
            let error = redactor.provider_error(body.as_bytes(), false, 1024);
            assert_eq!(error.detail, Some(json!("[REDACTED]")));
            assert!(error.message.is_none());
            assert!(!error.truncated);
        }
    }

    #[test]
    fn transport_plaintext_diagnostics_preserve_paths_and_redact_complete_and_partial_secrets() {
        let mut options = options();
        options
            .headers
            .insert("X-Private".into(), "private-header".into());
        let redactor = Redactor::new("test-key", &options);
        for (body, truncated, expected) in [
            (
                "Invalid path /questions/urgent: expected integer",
                false,
                "Invalid path /questions/urgent: expected integer",
            ),
            (
                "<html>Service unavailable</html>",
                false,
                "<html>Service unavailable</html>",
            ),
            (
                "Rejected test-key and private-header",
                false,
                "Rejected [REDACTED] and [REDACTED]",
            ),
            ("Rejected test-ke", true, "Rejected [REDACTED]"),
            ("Rejected private-hea", true, "Rejected [REDACTED]"),
            (
                r#"{"detail":"test-key","broken":"#,
                false,
                r#"{"detail":"[REDACTED]","broken":"#,
            ),
        ] {
            let error = redactor.provider_error(body.as_bytes(), truncated, 1024);
            assert!(error.detail.is_none());
            assert_eq!(error.message.as_deref(), Some(expected));
            assert_eq!(error.truncated, truncated);
        }
    }

    #[tokio::test]
    async fn transport_slow_422_body_keeps_safe_partial_diagnostics_without_retrying() {
        let server = Server::start(|_| {
            let mut reply = Reply::new(422, "invalid /questions/urgent: test-key private-header");
            reply.headers = "retry-after-ms: 1000\r\n".into();
            reply.body_prefix_bytes = reply.body.len() - 3;
            reply.body_delay = Duration::from_millis(300);
            reply
        })
        .await;
        let mut options = options();
        options
            .headers
            .insert("X-Private".into(), "private-header".into());
        options.retry.max_retries = 1;
        options.attempt_timeout_ms = Some(50);
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        assert_eq!(attempts, 1);
        let failure = result.unwrap_err();
        assert_eq!(failure.code, ErrorCode::Http);
        assert_eq!(failure.http_status, Some(422));
        assert_eq!(failure.retry_after_ms, Some(1000));
        let error = failure.provider_error.unwrap();
        assert_eq!(
            error.message.as_deref(),
            Some("invalid /questions/urgent: [REDACTED] [REDACTED]")
        );
        assert!(error.truncated);
        assert!(!unknown);
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn transport_slow_429_body_respects_observed_retry_hint() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let arrivals = observed.clone();
        let server = Server::start(move |index| {
            arrivals.lock().unwrap().push(Instant::now());
            let mut reply = Reply::new(if index == 0 { 429 } else { 200 }, "ok");
            if index == 0 {
                reply.headers = "retry-after-ms: 1000\r\n".into();
                reply.body_delay = Duration::from_millis(300);
            }
            reply
        })
        .await;
        let mut options = options();
        options.retry.max_retries = 1;
        options.attempt_timeout_ms = Some(50);
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        assert_eq!(result.unwrap(), b"ok");
        assert_eq!(attempts, 2);
        assert!(!unknown);
        let observed = observed.lock().unwrap();
        assert!(
            observed[1].duration_since(observed[0]) >= Duration::from_millis(1000),
            "server delay was ignored: {observed:?}"
        );
    }

    #[tokio::test]
    async fn transport_slow_429_body_keeps_final_http_metadata_when_retries_exhaust() {
        let server = Server::start(|_| {
            let mut reply = Reply::new(429, "busy");
            reply.headers = "retry-after-ms: 0\r\n".into();
            reply.body_delay = Duration::from_millis(300);
            reply
        })
        .await;
        let mut options = options();
        options.retry.max_retries = 1;
        options.retry.api_timeout_error = false;
        options.attempt_timeout_ms = Some(50);
        let (result, attempts, unknown) = call(&server, &options, ExecutionLimits::default()).await;
        let failure = result.unwrap_err();
        assert_eq!(failure.code, ErrorCode::Http);
        assert_eq!(failure.http_status, Some(429));
        assert_eq!(failure.retry_after_ms, Some(0));
        assert!(failure.provider_error.unwrap().truncated);
        assert_eq!(attempts, 2);
        assert!(!unknown);
    }

    #[tokio::test]
    async fn transport_slow_error_body_and_backoff_obey_total_deadline_and_drop() {
        for during_body in [false, true] {
            for cancel in [false, true] {
                let server = Server::start(|_| {
                    let mut reply = Reply::new(429, "busy");
                    reply.headers = "retry-after-ms: 1000\r\n".into();
                    reply.body_delay = Duration::from_secs(5);
                    reply
                })
                .await;
                let http = client();
                let permits = Semaphore::new(1);
                let attempts = AtomicUsize::new(0);
                let unknown = AtomicBool::new(false);
                let mut options = options();
                options.retry.max_retries = 1;
                options.attempt_timeout_ms = Some(if during_body { 1000 } else { 50 });
                let deadline =
                    Instant::now() + Duration::from_millis(if cancel { 2000 } else { 150 });
                let result = timeout(
                    Duration::from_millis(250),
                    send_http(
                        &http,
                        &permits,
                        "key",
                        ExecutionLimits::default(),
                        &options,
                        deadline,
                        &attempts,
                        &unknown,
                        || Ok(http.get(&server.url)),
                    ),
                )
                .await;
                if cancel {
                    assert!(result.is_err(), "dropping must interrupt {during_body}");
                } else {
                    assert_eq!(result.unwrap().unwrap_err().code, ErrorCode::Deadline);
                }
                assert_eq!(attempts.load(Ordering::SeqCst), 1);
                assert!(!unknown.load(Ordering::SeqCst));
                assert_eq!(permits.available_permits(), 1);
            }
        }
    }
}
