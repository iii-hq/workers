//! Its own test binary: tracing's callsite interest is process-wide, so a
//! capturing subscriber misses these events while other tests run alongside.
use judge_contract::{ErrorCode, RequestOptions};
use judge_provider::transport::{send_http, ExecutionLimits, RetryPolicy};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    time::Instant,
};

struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn each_retry_is_logged_without_content_or_credentials() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer = logs.clone();
    let _logging = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || Capture(writer.clone()))
            .finish(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let body = r#"{"error":{"message":"private-content"}}"#;
        let reply = format!(
            "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = socket.read(&mut [0; 8192]).await;
            let _ = socket.write_all(reply.as_bytes()).await;
        }
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let retry = RetryPolicy {
        max_retries: 2,
        backoff_initial_ms: 1,
        backoff_max_ms: 1,
        max_retry_after_ms: 60_000,
    };
    let attempts = AtomicUsize::new(0);
    let failure = send_http(
        &http,
        &Semaphore::new(1),
        "test-key",
        ExecutionLimits::default(),
        &RequestOptions::default(),
        retry,
        Instant::now() + Duration::from_secs(5),
        &attempts,
        &AtomicBool::new(false),
        || Ok(http.post(&url)),
    )
    .await
    .unwrap_err();
    assert_eq!(
        (failure.code, failure.http_status),
        (ErrorCode::Http, Some(503))
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    let logs = String::from_utf8(logs.lock().unwrap().clone()).unwrap();
    assert_eq!(
        logs.matches("provider attempt failed; retrying").count(),
        2,
        "{logs}"
    );
    assert!(
        logs.contains("attempt=1") && logs.contains("attempt=2"),
        "{logs}"
    );
    assert!(logs.contains("http_status=Some(503)"), "{logs}");
    assert!(
        !logs.contains("private-content") && !logs.contains("test-key"),
        "{logs}"
    );
}
