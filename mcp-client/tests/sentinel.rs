//! A known token set as every credential must show up in no log line, no
//! `servers::list` reply and no error text.

mod support;

use std::io;
use std::sync::{Arc, Mutex, OnceLock};

use mcp_client::server::{Manager, State};
use mcp_client::translate::ScrubWriter;
use serde_json::{Value, json};
use support::mcp_mock::McpMock;
use support::{ECHO, FakeHost, WAIT, connect, server, tool};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

// A bare secret value must be unique to its case: the writer scrubs the values
// of every live set, so a value two cases share hides one's leak behind the other.
pub const SENTINEL: &str = "sentinel-7f3a9c-do-not-print";
/// The key inside `keyed`'s URL; no set holds it as a bare value.
const KEYED_KEY: &str = "keyedkey-2b8e41-do-not-print";
/// The key inside `LIVE_URL`; no set holds it as a bare value.
const LIVE_KEY: &str = "livekey-9d05c7-do-not-print";
/// The bearer token and secret header of the redirect case.
const REDIRECT_TOKEN: &str = "redirect-4a61f2-do-not-print";
/// The env secret the stdio case's child prints to its stderr.
#[cfg(feature = "test-fixtures")]
const STDERR_KEY: &str = "stderrkey-5c19e7-do-not-print";

/// Every line the worker's own filter lets through at `RUST_LOG=trace`, as
/// the production writer prints it.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Captured {
    type Writer = Captured;

    fn make_writer(&'a self) -> Captured {
        self.clone()
    }
}

impl Captured {
    /// The one subscriber of this test binary, shared by every case.
    fn install() -> &'static Captured {
        static CAPTURED: OnceLock<Captured> = OnceLock::new();
        CAPTURED.get_or_init(|| {
            let captured = Captured::default();
            tracing_subscriber::fmt()
                .with_env_filter(EnvFilter::new(mcp_client::log_directives(Some("trace"))))
                .with_ansi(false)
                .with_writer(ScrubWriter(captured.clone()))
                .try_init()
                .expect("the sentinel binary installs the only subscriber");
            captured
        })
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

/// `srv` on `url` with `secret://TOK` as its bearer token and `secret://HDR`
/// as its secret header.
fn guarded(url: &str) -> Value {
    json!({"type": "http", "url": url, "headers": {"X-API-Key": "secret://HDR"},
           "auth": {"type": "bearer", "token": "secret://TOK"}})
}

/// A host whose `TOK` and `HDR` are both `token`.
fn host_with(token: &str) -> Arc<FakeHost> {
    let host = FakeHost::new();
    host.set_secret("secret://TOK", token);
    host.set_secret("secret://HDR", token);
    host
}

async fn settle(manager: &Arc<Manager>, name: &str, state: State) {
    let reached = manager.wait_for(name, state, WAIT).await;
    assert!(reached, "{}", server(manager, name).await);
}

async fn listed(manager: &Arc<Manager>) -> String {
    serde_json::to_string(&manager.list().await).unwrap()
}

#[tokio::test]
async fn sentinel_token_appears_nowhere() {
    let logs = Captured::install();
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    let host = host_with(SENTINEL);
    // A URL that carries its key; nothing listens there.
    host.set_secret(
        "secret://URL",
        &format!("http://127.0.0.1:1/mcp?k={KEYED_KEY}"),
    );
    let config = json!({"mcpServers": {
        "srv": guarded(&mock.url()),
        "keyed": {"type": "http", "url": "secret://URL"},
    }});
    let manager = connect(&host, config).await;
    settle(&manager, "srv", State::Ready).await;
    settle(&manager, "keyed", State::Unavailable).await;
    // The sentinel really is in play.
    let sent = mock.headers_of("initialize").await;
    assert_eq!(sent[0]["authorization"], format!("Bearer {SENTINEL}"));
    assert_eq!(sent[0]["x-api-key"], SENTINEL);

    let mut seen = vec![listed(&manager).await];
    // The server echoes the credential.
    let echoed = format!("bad token {SENTINEL}");
    mock.on_call_error("echo", -32000, &echoed).await;
    seen.push(host.call(ECHO, json!({})).await.unwrap_err());
    mock.http_status("tools/call", 429, &[("retry-after", "3")])
        .await;
    seen.push(host.call(ECHO, json!({})).await.unwrap_err());
    let challenge = r#"Bearer error="invalid_token""#;
    mock.http_status("tools/call", 401, &[("www-authenticate", challenge)])
        .await;
    seen.push(host.call(ECHO, json!({})).await.unwrap_err());
    settle(&manager, "srv", State::AuthFailed).await;
    seen.push(host.call(ECHO, json!({})).await.unwrap_err());
    seen.push(listed(&manager).await);
    manager.shutdown().await;

    for text in &seen {
        assert!(
            !text.contains(SENTINEL) && !text.contains(KEYED_KEY),
            "{text}"
        );
    }
    assert!(
        seen[0].contains("secret://URL"),
        "a URL that is a reference is shown as the reference"
    );
    let log = logs.text();
    assert!(
        log.contains("mcp_client"),
        "the capture saw none of this crate's lines"
    );
    assert!(!log.contains(SENTINEL) && !log.contains(KEYED_KEY), "{log}");
}

#[tokio::test]
async fn a_secret_url_stays_out_of_the_log_when_its_server_goes_away() {
    let logs = Captured::install();
    let mock = McpMock::start().await;
    mock.tools(json!([tool("echo")])).await;
    let host = FakeHost::new();
    host.set_secret("secret://LIVE_URL", &format!("{}?k={LIVE_KEY}", mock.url()));
    let config = json!({"mcpServers": {"live": {"type": "http", "url": "secret://LIVE_URL"}}});
    let manager = connect(&host, config).await;
    settle(&manager, "live", State::Ready).await;
    let mut seen = vec![listed(&manager).await];
    // Its listener closes: the session DELETE of the shutdown fails.
    drop(mock);
    seen.push(listed(&manager).await);
    manager.shutdown().await;

    for text in &seen {
        assert!(!text.contains(LIVE_KEY), "{text}");
    }
    let log = logs.text();
    assert!(!log.contains(LIVE_KEY) && !log.contains(SENTINEL), "{log}");
}

/// Pins rmcp's own redirect policy (same-origin 307 and 308 only), which an
/// rmcp upgrade could change without any other suite noticing.
#[tokio::test]
async fn a_cross_origin_redirect_gets_no_credential() {
    let logs = Captured::install();
    let (first, second) = (McpMock::start().await, McpMock::start().await);
    first.tools(json!([tool("echo")])).await;
    let host = host_with(REDIRECT_TOKEN);
    let manager = connect(&host, json!({"mcpServers": {"srv": guarded(&first.url())}})).await;
    settle(&manager, "srv", State::Ready).await;
    let location = second.url();
    first
        .http_status("tools/call", 307, &[("location", location.as_str())])
        .await;
    assert!(host.call(ECHO, json!({})).await.is_err());
    assert_eq!(second.request_count().await, 0);
    manager.shutdown().await;
    let log = logs.text();
    assert!(
        !log.contains(REDIRECT_TOKEN) && !log.contains(SENTINEL),
        "{log}"
    );
}

#[cfg(feature = "test-fixtures")]
#[tokio::test]
async fn sentinel_is_redacted_from_stderr_and_process_logs() {
    use std::time::Duration;

    use mcp_client::server::Timings;
    use support::stdio_fixture::{cfg, opts};
    use support::{connect_opts, stdio_timings, until};

    let captured = Captured::install();
    let host = FakeHost::new();
    host.set_secret("secret://FAKE_TOKEN", STDERR_KEY);
    let config = cfg(json!({"API_TOKEN": "secret://FAKE_TOKEN"}));
    let held = Timings {
        backoff_min: Duration::from_secs(5),
        backoff_max: Duration::from_secs(5),
        ..stdio_timings()
    };
    let m = connect_opts(&host, &config, opts(held)).await;
    let ready = m
        .wait_for("fake", State::Ready, Duration::from_secs(10))
        .await;
    assert!(ready, "{}", server(&m, "fake").await);
    // The child holds the secret in its env and prints it, as a real server's debug line would.
    let line = format!("token={STDERR_KEY} end");
    host.call("mcp::fake::stderr", json!({"text": line}))
        .await
        .unwrap();
    let redacted = until(Duration::from_secs(2), || async {
        let logs = m.logs("fake").await.unwrap();
        logs.lines.iter().any(|l| l.text == "token=[redacted] end")
    })
    .await;
    assert!(redacted, "{:?}", m.logs("fake").await);
    let exit_error = host
        .call("mcp::fake::exit", json!({"code": 3}))
        .await
        .unwrap_err();
    assert!(m.wait_for("fake", State::Unavailable, WAIT).await);
    let list = listed(&m).await;
    assert!(
        list.contains("exited with code 3: token=[redacted] end"),
        "{list}"
    );
    let unavailable = host
        .call("mcp::fake::echo", json!({"text": "x"}))
        .await
        .unwrap_err();
    let logs = serde_json::to_string(&m.logs("fake").await.unwrap()).unwrap();
    m.shutdown().await;
    let log = captured.text();
    for (what, text) in [
        ("servers::list", &list),
        ("process::logs", &logs),
        ("exit error", &exit_error),
        ("unavailable error", &unavailable),
        ("log", &log),
    ] {
        assert!(
            !text.contains(STDERR_KEY) && !text.contains(SENTINEL),
            "{what} holds the secret: {text}"
        );
    }
}
