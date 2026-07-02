//! Cutover e2e: boots the worker with `III_HTTP_TRIGGER_TYPE=http` and
//! confirms it owns the `http` trigger type cleanly -- boots, registers a
//! route, and serves a request -- when no built-in `iii-http` worker is
//! connected to the engine.
//!
//! Isolated in its own test binary (a separate process from every other
//! `tests/e2e_*.rs` file) so mutating `III_HTTP_TRIGGER_TYPE` here can never
//! bleed into the rest of the suite, which relies on the `http-ng` default.
//!
//! The shared engine at `III_ENGINE_WS_URL` (see `common::engine`) may or may
//! not have the built-in `iii-http` worker connected, depending on how it was
//! started (e.g. `iii --use-default-config` enables it). When it's present,
//! `boot::start` correctly refuses -- that's the guard from
//! `iii_http::boot::start` working as designed; see the
//! `builtin_iii_http_active` unit tests in `src/boot.rs` for pure-logic
//! coverage of the detection that doesn't need an engine at all. This test
//! only asserts the success path (a clean cutover against a builtin-free
//! engine), so it skips with an explanatory message instead of failing when a
//! builtin is detected.

mod common;

use common::engine;
use serial_test::serial;

use iii_http::config::RestApiConfig;

/// Restores an environment variable to its prior value (or removes it if it
/// was unset) on drop, so the mutation doesn't survive a panic or early
/// return.
struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

#[tokio::test]
#[serial]
async fn cutover_to_http_boots_and_serves_when_no_builtin_present() {
    let iii = engine::get_or_init().await;
    let _env_guard = EnvGuard::set("III_HTTP_TRIGGER_TYPE", "http");
    assert_eq!(iii_http::trigger_type(), "http");

    let boot_result = iii_http::boot::start(
        iii.clone(),
        RestApiConfig {
            port: 0,
            host: "127.0.0.1".to_string(),
            ..RestApiConfig::default()
        },
    )
    .await;

    let boot = match boot_result {
        Ok(boot) => boot,
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("iii-http worker is active"),
                "boot failed for a reason other than the builtin guard: {msg}"
            );
            eprintln!(
                "skipping cutover e2e: engine at {} has the built-in iii-http worker \
                 connected, so the guard correctly refused. Point III_ENGINE_WS_URL at a \
                 builtin-free engine to exercise the success path.",
                engine::ws_url()
            );
            return;
        }
    };

    common::backend::register_echo_backend(&iii, "/cutover", "GET").await;
    common::wait_for_route(&boot.routes, "GET", "/cutover").await;

    let url = format!("http://{}/cutover", boot.local_addr);
    let resp = reqwest::Client::new().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["method"], "GET");

    boot.shutdown().await;
}
