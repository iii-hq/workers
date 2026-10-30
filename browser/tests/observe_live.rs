//! Live checks of how the worker's functions meet observers, on a real
//! engine (worker manager only):
//!
//! - `browser::cookies::list` and `browser::cookies::set`, registered by the
//!   worker binary, refuse every observer (`NOT_OBSERVABLE`).
//!
//! Self-skips when `iii` is not on PATH. Needs no Chromium. Scratch files
//! live under cargo's per-target tmp dir; `BROWSER_OBSERVE_LIVE_KEEP=1` keeps
//! them (and the engine/worker logs) for inspection.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use iii_sdk::{
    register_worker, CallRequest, InitOptions, ObserverHandle, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout, Instant};

/// An engine with only the worker manager, and the worker binary when a test
/// starts it.
struct Live {
    iii: Child,
    worker: Option<Child>,
    engine_ws: String,
    dir: PathBuf,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(worker) = &mut self.worker {
            // SAFETY: plain libc call on the child pid we spawned.
            unsafe {
                libc::kill(worker.id() as i32, libc::SIGTERM);
            }
            std::thread::sleep(Duration::from_millis(500));
            let _ = worker.kill();
            let _ = worker.wait();
        }
        let _ = self.iii.kill();
        let _ = self.iii.wait();
        if std::env::var_os("BROWSER_OBSERVE_LIVE_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        } else {
            eprintln!("kept observe live scratch dir: {}", self.dir.display());
        }
    }
}

impl Live {
    async fn boot() -> Option<Live> {
        let iii_bin = which::which("iii").ok()?;
        let port = TcpListener::bind("127.0.0.1:0")
            .ok()?
            .local_addr()
            .ok()?
            .port();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "browser-observe-live-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).ok()?;
        let config_path = dir.join("engine.yaml");
        std::fs::write(
            &config_path,
            format!(
                "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      \
                 port: {port}\n"
            ),
        )
        .ok()?;
        let engine_log = std::fs::File::create(dir.join("engine.log")).ok()?;
        let iii = Command::new(&iii_bin)
            .args(["--config", config_path.to_str()?, "--no-update-check"])
            .current_dir(&dir)
            .stdout(Stdio::from(engine_log.try_clone().ok()?))
            .stderr(Stdio::from(engine_log))
            .spawn()
            .ok()?;
        sleep(Duration::from_millis(1000)).await;
        Some(Live {
            iii,
            worker: None,
            engine_ws: format!("ws://127.0.0.1:{port}"),
            dir,
        })
    }

    /// Start the worker binary against this engine.
    fn spawn_worker(&mut self) -> Option<()> {
        let worker_log = std::fs::File::create(self.dir.join("worker.log")).ok()?;
        let worker = Command::new(env!("CARGO_BIN_EXE_browser"))
            .arg("--url")
            .arg(&self.engine_ws)
            .env("III_COMPOSE_DIR", &self.dir)
            .env("RUST_LOG", "info")
            .stdout(Stdio::from(worker_log.try_clone().ok()?))
            .stderr(Stdio::from(worker_log))
            .spawn()
            .ok()?;
        self.worker = Some(worker);
        Some(())
    }
}

async fn await_active(observer: &ObserverHandle) {
    timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer active in time")
        .expect("observer accepted");
}

#[tokio::test]
async fn cookie_functions_refuse_observers() {
    let Some(mut live) = Live::boot().await else {
        eprintln!("skipping: `iii` not available");
        return;
    };
    live.spawn_worker().expect("worker started");
    let client = register_worker(&live.engine_ws, InitOptions::default());

    // Wait for the worker's registrations.
    let deadline = Instant::now() + Duration::from_secs(30);
    let functions = loop {
        let listed = timeout(
            Duration::from_secs(10),
            client.call(CallRequest {
                function_id: "engine::functions::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(10_000),
            }),
        )
        .await
        .expect("functions list in time")
        .expect("functions list");
        let functions = listed["functions"].as_array().cloned().unwrap_or_default();
        if functions
            .iter()
            .any(|f| f["function_id"] == "browser::cookies::set")
        {
            break functions;
        }
        assert!(Instant::now() < deadline, "the worker registered nothing");
        sleep(Duration::from_millis(250)).await;
    };
    let entry = |id: &str| {
        functions
            .iter()
            .find(|f| f["function_id"] == id)
            .unwrap_or_else(|| panic!("{id} is registered"))
            .clone()
    };
    for id in ["browser::cookies::list", "browser::cookies::set"] {
        assert_eq!(entry(id)["observable"], false, "{id}: {}", entry(id));
    }
    assert_ne!(
        entry("browser::cookies::clear")["observable"],
        false,
        "clear carries no cookie value"
    );

    client.register_function(
        "cookietest::sink",
        RegisterFunction::new(|_: Value| Ok::<_, iii_sdk::errors::Error>(json!({ "ok": true }))),
    );
    for id in ["browser::cookies::list", "browser::cookies::set"] {
        let observer = client.observe(Source::function(id), "cookietest::sink");
        let err = timeout(Duration::from_secs(10), observer.ready())
            .await
            .expect("refusal in time")
            .expect_err("a cookie function refuses observers");
        assert!(format!("{err}").contains("NOT_OBSERVABLE"), "{id}: {err}");
        let _ = observer.dispose().await;
    }
    let clear = client.observe(
        Source::function("browser::cookies::clear"),
        "cookietest::sink",
    );
    await_active(&clear).await;
    clear.dispose().await.expect("dispose");

    client.shutdown_async().await;
}
