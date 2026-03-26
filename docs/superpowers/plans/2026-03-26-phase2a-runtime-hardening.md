# Phase 2A: Runtime Hardening — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Harden managed Docker workers with manifest resource enforcement, a health check loop with exponential backoff restarts, and graceful shutdown via SDK invocation draining.

**Architecture:** Three independent features touching two repos. Resource enforcement is a wiring change in the CLI and launcher. Health checks are a new background loop in the launcher. Graceful shutdown adds an in-flight invocation counter to the SDK and a configurable stop timeout to the Docker adapter.

**Tech Stack:** Rust, tokio, iii-sdk 0.9.0, Docker CLI, chrono

---

## Repos

- **Engine repo:** `/Users/andersonleal/projetos/motia/motia`
- **Workers repo:** `/Users/andersonleal/projetos/motia/workers`

## File Structure

### Workers Repo — Modified Files

| File | Changes |
|------|---------|
| `iii-launcher/src/state.rs` | Add `restart_count`, `last_failure`, `backoff_until` fields to `ManagedWorker` |
| `iii-launcher/src/adapter.rs` | Add `stop_timeout_secs` to `ContainerSpec`. Change `RuntimeAdapter::stop()` signature to accept timeout. |
| `iii-launcher/src/docker.rs` | Pass `--time` flag to `docker stop` |
| `iii-launcher/src/functions/start.rs` | Read `memory_limit`/`cpu_limit` from manifest resources if not explicitly provided |
| `iii-launcher/src/functions/status.rs` | Return `restart_count`, `last_failure`, `status` in response |
| `iii-launcher/src/functions/stop.rs` | Pass stop timeout to adapter |
| `iii-launcher/src/main.rs` | Spawn health check background loop |

### Workers Repo — New Files

| File | Responsibility |
|------|---------------|
| `iii-launcher/src/health.rs` | Health check loop: interval timer, container + engine checks, restart with backoff |

### Engine Repo — Modified Files

| File | Changes |
|------|---------|
| `engine/src/cli/managed.rs` | Extract manifest resources and pass to start request. Add `RESTARTS` column to status output. |
| `sdk/packages/rust/iii/src/iii.rs` | Add `in_flight_count: AtomicUsize` to `IIIInner`. Increment/decrement around handler invocations. Drain on shutdown. |

---

## Task 1: State Enrichment

**Files:**
- Modify: `workers/iii-launcher/src/state.rs`

Add restart tracking fields to `ManagedWorker` and a helper to record the original container spec for restarts.

- [ ] **Step 1: Add restart fields to ManagedWorker**

In `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/state.rs`, add three fields to the `ManagedWorker` struct after the existing `config` field:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWorker {
    pub image: String,
    pub container_id: String,
    pub runtime: String,
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub config: serde_json::Value,
    #[serde(default)]
    pub restart_count: u32,
    #[serde(default)]
    pub last_failure: Option<String>,
    #[serde(default)]
    pub backoff_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default)]
    pub cpu_limit: Option<String>,
    #[serde(default)]
    pub engine_url: Option<String>,
    #[serde(default)]
    pub auth_token: Option<String>,
}
```

The `memory_limit`, `cpu_limit`, `engine_url`, and `auth_token` fields are stored so the health check can reconstruct the `ContainerSpec` for restarts without needing the original request.

`#[serde(default)]` ensures backward compatibility with existing `launcher-state.json` files — new fields deserialize as their default values.

- [ ] **Step 2: Verify compilation**

Run: `cd /Users/andersonleal/projetos/motia/workers/iii-launcher && cargo check`
Expected: Compilation errors in `start.rs` and `status.rs` because they construct `ManagedWorker` without the new fields. That's expected — we fix those in later tasks.

- [ ] **Step 3: Fix start.rs — add new fields to ManagedWorker construction**

In `iii-launcher/src/functions/start.rs`, find the `ManagedWorker` construction (around line 87) and add the new fields:

```rust
            let worker = ManagedWorker {
                image: image.clone(),
                container_id: container_id.clone(),
                runtime: "docker".to_string(),
                started_at: chrono::Utc::now(),
                status: "running".to_string(),
                config,
                restart_count: 0,
                last_failure: None,
                backoff_until: None,
                memory_limit: spec.memory_limit.clone(),
                cpu_limit: spec.cpu_limit.clone(),
                engine_url: Some(engine_url.clone()),
                auth_token: Some(auth_token.clone()),
            };
```

Where `engine_url` and `auth_token` are the values already extracted from the payload earlier in the same function.

- [ ] **Step 4: Verify compilation**

Run: `cd /Users/andersonleal/projetos/motia/workers/iii-launcher && cargo check`
Expected: Compiles cleanly.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/src/state.rs iii-launcher/src/functions/start.rs
git commit -m "feat(launcher): add restart tracking fields to ManagedWorker state"
```

---

## Task 2: Docker Stop Timeout

**Files:**
- Modify: `workers/iii-launcher/src/adapter.rs`
- Modify: `workers/iii-launcher/src/docker.rs`
- Modify: `workers/iii-launcher/src/functions/stop.rs`

Add configurable stop timeout so Docker sends SIGTERM and waits before SIGKILL.

- [ ] **Step 1: Update RuntimeAdapter::stop() signature**

In `iii-launcher/src/adapter.rs`, change the `stop` method to accept a timeout:

```rust
#[async_trait::async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn pull(&self, image: &str) -> Result<ImageInfo>;
    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>>;
    async fn start(&self, spec: &ContainerSpec) -> Result<String>;
    async fn stop(&self, container_id: &str, timeout_secs: u32) -> Result<()>;
    async fn status(&self, container_id: &str) -> Result<ContainerStatus>;
    async fn logs(&self, container_id: &str, follow: bool) -> Result<String>;
    async fn remove(&self, container_id: &str) -> Result<()>;
}
```

- [ ] **Step 2: Update DockerAdapter::stop()**

In `iii-launcher/src/docker.rs`, update the `stop` implementation to pass `--time`:

```rust
    async fn stop(&self, container_id: &str, timeout_secs: u32) -> Result<()> {
        tracing::info!(container_id = %container_id, timeout_secs = timeout_secs, "stopping container");
        let timeout_str = timeout_secs.to_string();
        Self::run_cmd(&["stop", "--time", &timeout_str, container_id]).await?;
        Ok(())
    }
```

- [ ] **Step 3: Update all callers of adapter.stop()**

Find all calls to `adapter.stop()` in the codebase and add the timeout argument.

In `iii-launcher/src/functions/stop.rs`, update the stop call:

```rust
                    adapter
                        .stop(&w.container_id, 30)
                        .await
                        .map_err(|e| IIIError::Handler(format!("stop failed: {e}")))?;
```

In `iii-launcher/src/functions/start.rs`, update the pre-start cleanup:

```rust
            let _ = adapter.stop(&name, 10).await;
```

The pre-start cleanup uses a shorter timeout (10s) since we're just cleaning up a stale container before starting a new one.

- [ ] **Step 4: Verify compilation**

Run: `cd /Users/andersonleal/projetos/motia/workers/iii-launcher && cargo check`
Expected: Compiles cleanly.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/src/adapter.rs iii-launcher/src/docker.rs iii-launcher/src/functions/stop.rs iii-launcher/src/functions/start.rs
git commit -m "feat(launcher): add configurable stop timeout to Docker adapter"
```

---

## Task 3: Resource Enforcement

**Files:**
- Modify: `engine/src/cli/managed.rs`

Wire manifest resources from pull response into the start request.

- [ ] **Step 1: Extract resources from manifest and pass to start**

In `/Users/andersonleal/projetos/motia/motia/engine/src/cli/managed.rs`, find the `handle_managed_add` function. After the pull succeeds and the manifest is displayed (around line 206), extract the resources and include them in the start payload.

Find the start payload construction (around line 211):

```rust
    let start_payload = serde_json::json!({
        "name": name,
        "image": image_ref,
        "engine_url": engine_url,
    });
```

Replace with:

```rust
    // Extract resource limits from manifest (defaults)
    let memory_limit = pull_result
        .get("manifest")
        .and_then(|m| m.get("resources"))
        .and_then(|r| r.get("memory"))
        .and_then(|v| v.as_str());
    let cpu_limit = pull_result
        .get("manifest")
        .and_then(|m| m.get("resources"))
        .and_then(|r| r.get("cpu"))
        .and_then(|v| v.as_str());

    let mut start_payload = serde_json::json!({
        "name": name,
        "image": image_ref,
        "engine_url": engine_url,
    });
    if let Some(mem) = memory_limit {
        start_payload["memory_limit"] = serde_json::Value::String(mem.to_string());
    }
    if let Some(cpu) = cpu_limit {
        start_payload["cpu_limit"] = serde_json::Value::String(cpu.to_string());
    }
```

- [ ] **Step 2: Verify compilation**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii --lib`
Expected: Compiles.

- [ ] **Step 3: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/managed.rs
git commit -m "feat(cli): propagate manifest resources to container start request"
```

---

## Task 4: Health Check Loop

**Files:**
- Create: `workers/iii-launcher/src/health.rs`
- Modify: `workers/iii-launcher/src/main.rs`

This is the core feature. A background loop that checks managed workers and restarts unhealthy ones.

- [ ] **Step 1: Create health.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/health.rs`:

```rust
use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Mutex;

use crate::adapter::{ContainerSpec, RuntimeAdapter};
use crate::state::{LauncherState, ManagedWorker};

const CHECK_INTERVAL_SECS: u64 = 15;
const MAX_RETRIES: u32 = 5;

fn backoff_secs(restart_count: u32) -> i64 {
    let secs = 5 * 2_i64.pow(restart_count.saturating_sub(1));
    secs.min(60)
}

pub async fn run_health_loop(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
    iii: iii_sdk::III,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;

        let workers: Vec<(String, ManagedWorker)> = {
            let st = state.lock().await;
            st.managed_workers
                .iter()
                .filter(|(_, w)| w.status != "failed")
                .map(|(name, w)| (name.clone(), w.clone()))
                .collect()
        };

        if workers.is_empty() {
            continue;
        }

        // Get connected workers from engine (best-effort)
        let connected_workers = match iii.list_workers().await {
            Ok(list) => list
                .iter()
                .filter_map(|w| Some(w.name.clone()?))
                .collect::<Vec<String>>(),
            Err(e) => {
                tracing::debug!(error = %e, "failed to list workers from engine, skipping engine check");
                Vec::new()
            }
        };

        for (name, worker) in workers {
            check_worker(
                &name,
                &worker,
                &adapter,
                &state,
                &connected_workers,
            )
            .await;
        }
    }
}

async fn check_worker(
    name: &str,
    worker: &ManagedWorker,
    adapter: &Arc<dyn RuntimeAdapter>,
    state: &Arc<Mutex<LauncherState>>,
    connected_workers: &[String],
) {
    let now = Utc::now();

    // Check if in backoff cooldown
    if let Some(backoff_until) = worker.backoff_until {
        if now < backoff_until {
            return;
        }
    }

    // 1. Container check
    let container_running = match adapter.status(&worker.container_id).await {
        Ok(cs) => cs.running,
        Err(_) => false,
    };

    // 2. Engine check — is a worker with this name connected?
    let worker_registered = connected_workers.iter().any(|w| w == name);

    if container_running && worker_registered {
        // Healthy — reset restart state if needed
        if worker.restart_count > 0 {
            let mut st = state.lock().await;
            if let Some(w) = st.managed_workers.get_mut(name) {
                w.restart_count = 0;
                w.last_failure = None;
                w.backoff_until = None;
                tracing::info!(worker = %name, "worker recovered, reset restart count");
            }
            let _ = st.save();
        }
        return;
    }

    // Unhealthy — determine reason
    let reason = if !container_running {
        "container not running".to_string()
    } else {
        "worker not registered with engine".to_string()
    };

    tracing::warn!(worker = %name, reason = %reason, "worker unhealthy");

    let mut st = state.lock().await;
    let Some(w) = st.managed_workers.get_mut(name) else {
        return;
    };

    w.restart_count += 1;
    w.last_failure = Some(reason.clone());

    if w.restart_count > MAX_RETRIES {
        w.status = "failed".to_string();
        tracing::error!(
            worker = %name,
            restart_count = w.restart_count,
            reason = %reason,
            "max restarts exceeded, marking worker as failed"
        );
        let _ = st.save();
        return;
    }

    let backoff = backoff_secs(w.restart_count);
    w.backoff_until = Some(now + chrono::Duration::seconds(backoff));

    tracing::warn!(
        worker = %name,
        restart_count = w.restart_count,
        backoff_secs = backoff,
        reason = %reason,
        "restarting unhealthy worker"
    );

    // Clone what we need before dropping the lock
    let image = w.image.clone();
    let engine_url = w.engine_url.clone().unwrap_or_default();
    let auth_token = w.auth_token.clone().unwrap_or_default();
    let config = w.config.clone();
    let memory_limit = w.memory_limit.clone();
    let cpu_limit = w.cpu_limit.clone();
    let old_container_id = w.container_id.clone();

    // Drop lock before async operations
    drop(st);

    // Stop and remove old container
    let _ = adapter.stop(&old_container_id, 30).await;
    let _ = adapter.remove(&old_container_id).await;

    // Build env for new container
    let mut env = HashMap::new();
    env.insert("III_ENGINE_URL".to_string(), engine_url.clone());
    env.insert("III_AUTH_TOKEN".to_string(), auth_token.clone());
    let config_json = serde_json::to_string(&config).unwrap_or_else(|_| "{}".to_string());
    env.insert(
        "III_WORKER_CONFIG".to_string(),
        data_encoding::BASE64.encode(config_json.as_bytes()),
    );

    let spec = ContainerSpec {
        name: name.to_string(),
        image: image.clone(),
        env,
        memory_limit,
        cpu_limit,
    };

    match adapter.start(&spec).await {
        Ok(new_container_id) => {
            let mut st = state.lock().await;
            if let Some(w) = st.managed_workers.get_mut(name) {
                w.container_id = new_container_id;
                w.started_at = Utc::now();
                w.status = "running".to_string();
            }
            let _ = st.save();
            tracing::info!(worker = %name, "worker restarted successfully");
        }
        Err(e) => {
            tracing::error!(worker = %name, error = %e, "failed to restart worker");
        }
    }
}
```

- [ ] **Step 2: Register the module and spawn the loop in main.rs**

In `iii-launcher/src/main.rs`, add `mod health;` after the existing module declarations:

```rust
mod adapter;
mod docker;
mod functions;
mod health;
mod state;
```

Then, after all functions are registered and the `"all launcher functions registered"` log line (around line 179), spawn the health loop:

```rust
    tracing::info!("all launcher functions registered, waiting for invocations");

    // Spawn health check background loop
    let health_adapter = adapter.clone();
    let health_state = launcher_state.clone();
    let health_iii = iii.clone();
    tokio::spawn(async move {
        health::run_health_loop(health_adapter, health_state, health_iii).await;
    });

    tokio::signal::ctrl_c().await?;
```

Note: `iii.clone()` works because `III` derives `Clone` (it wraps `Arc<IIIInner>`).

- [ ] **Step 3: Add data-encoding dependency**

Check if `data-encoding` is already in `iii-launcher/Cargo.toml`. If so, skip this step. If not, add `data-encoding = "2"`.

- [ ] **Step 4: Verify compilation**

Run: `cd /Users/andersonleal/projetos/motia/workers/iii-launcher && cargo check`
Expected: Compiles. There may be a warning about `list_workers()` return type — check what it returns and adjust the field access for `name` accordingly. The SDK's `WorkerInfo` struct may use `.name` as `Option<String>` or `String`.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/src/health.rs iii-launcher/src/main.rs
git commit -m "feat(launcher): add health check loop with exponential backoff restarts"
```

---

## Task 5: Enriched Status Output

**Files:**
- Modify: `workers/iii-launcher/src/functions/status.rs`
- Modify: `engine/src/cli/managed.rs`

Return and display restart info in `iii worker status`.

- [ ] **Step 1: Update status handler to include restart fields**

In `iii-launcher/src/functions/status.rs`, update the JSON response for each worker to include the new fields:

```rust
                results.push(serde_json::json!({
                    "name": name,
                    "image": worker.image,
                    "runtime": worker.runtime,
                    "running": running,
                    "started_at": worker.started_at.to_rfc3339(),
                    "status": worker.status,
                    "restart_count": worker.restart_count,
                    "last_failure": worker.last_failure,
                }));
```

- [ ] **Step 2: Update CLI status display**

In `/Users/andersonleal/projetos/motia/motia/engine/src/cli/managed.rs`, find `handle_managed_status` and update the table header and row rendering to include `RESTARTS`:

Update the header:

```rust
                eprintln!(
                    "  {:20} {:40} {:10} {:10} {}",
                    "NAME".bold(),
                    "IMAGE".bold(),
                    "STATUS".bold(),
                    "RESTARTS".bold(),
                    "STARTED".bold()
                );
                eprintln!(
                    "  {:20} {:40} {:10} {:10} {}",
                    "----".dimmed(),
                    "-----".dimmed(),
                    "------".dimmed(),
                    "--------".dimmed(),
                    "-------".dimmed()
                );
```

Update each row:

```rust
                for w in workers {
                    let name = w.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let image = w.get("image").and_then(|v| v.as_str()).unwrap_or("?");
                    let status_raw = w.get("status").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let running = w.get("running").and_then(|v| v.as_bool()).unwrap_or(false);
                    let restarts = w.get("restart_count").and_then(|v| v.as_u64()).unwrap_or(0);
                    let started = w
                        .get("started_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("-");
                    let last_failure = w.get("last_failure").and_then(|v| v.as_str());

                    let status_str = if status_raw == "failed" {
                        "failed".red().to_string()
                    } else if running {
                        "running".green().to_string()
                    } else {
                        "stopped".red().to_string()
                    };

                    eprintln!(
                        "  {:20} {:40} {:10} {:10} {}",
                        name, image, status_str, restarts, started
                    );

                    if let Some(failure) = last_failure {
                        if status_raw == "failed" {
                            eprintln!(
                                "  {}: max restarts exceeded — {}",
                                name.dimmed(),
                                failure.dimmed()
                            );
                        }
                    }
                }
```

- [ ] **Step 3: Verify compilation in both repos**

Run: `cd /Users/andersonleal/projetos/motia/workers/iii-launcher && cargo check`
Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii --lib`

- [ ] **Step 4: Commit (both repos)**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/src/functions/status.rs
git commit -m "feat(launcher): return restart info in status response"

cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/managed.rs
git commit -m "feat(cli): show restart count and failure info in worker status"
```

---

## Task 6: SDK Graceful Shutdown Drain

**Files:**
- Modify: `sdk/packages/rust/iii/src/iii.rs`

Add an in-flight invocation counter and drain logic to the SDK's shutdown path.

- [ ] **Step 1: Add in_flight_count to IIIInner**

In `/Users/andersonleal/projetos/motia/motia/sdk/packages/rust/iii/src/iii.rs`, find the `IIIInner` struct (around line 515). Add:

```rust
    in_flight_count: AtomicUsize,
```

Add `use std::sync::atomic::AtomicUsize;` if not already imported (AtomicBool is already imported, so the import line may just need `AtomicUsize` added).

Initialize it in the constructor (find `IIIInner` initialization, around line 580):

```rust
    in_flight_count: AtomicUsize::new(0),
```

- [ ] **Step 2: Increment/decrement around handler invocations**

Find `handle_invoke_function` (around line 1480). The method spawns a `tokio::spawn` for the handler. Wrap the handler execution with counter management:

Right after the `tokio::spawn(async move {` line, before the handler call, add:

```rust
            iii.inner.in_flight_count.fetch_add(1, Ordering::SeqCst);
```

After the handler result is sent (the `InvocationResult` send, near the end of the spawn block), add in a finally-like pattern. The cleanest way is to use a scope guard or just ensure the decrement happens on all paths. Add right before the closing `});` of the spawn block:

```rust
            iii.inner.in_flight_count.fetch_sub(1, Ordering::SeqCst);
```

Make sure this decrement happens on ALL code paths (success, error, function-not-found early return). The function-not-found early return (around line 1532) happens before the `tokio::spawn`, so it doesn't need the counter.

- [ ] **Step 3: Add shutting_down flag**

Add to `IIIInner`:

```rust
    shutting_down: AtomicBool,
```

Initialize as `AtomicBool::new(false)`.

- [ ] **Step 4: Reject invocations when shutting down**

At the start of `handle_invoke_function`, after the function lookup but before spawning the handler task, add:

```rust
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            if let Some(invocation_id) = invocation_id {
                let (resp_tp, resp_bg) = inject_trace_headers();
                let _ = self.send_message(Message::InvocationResult {
                    invocation_id,
                    function_id,
                    result: None,
                    error: Some(ErrorBody {
                        code: "worker_shutting_down".to_string(),
                        message: "Worker is shutting down and not accepting new invocations".to_string(),
                        stacktrace: None,
                    }),
                    traceparent: resp_tp,
                    baggage: resp_bg,
                });
            }
            return;
        }
```

- [ ] **Step 5: Drain in-flight invocations on shutdown**

Find `shutdown_async` (around line 683). Before setting `running` to false, set `shutting_down` and drain:

```rust
    pub async fn shutdown_async(&self) {
        // Signal that we're shutting down — reject new invocations
        self.inner.shutting_down.store(true, Ordering::SeqCst);

        // Wait for in-flight invocations to drain (max 25s)
        let drain_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            let in_flight = self.inner.in_flight_count.load(Ordering::SeqCst);
            if in_flight == 0 {
                tracing::info!("all in-flight invocations drained, shutting down");
                break;
            }
            if tokio::time::Instant::now() >= drain_deadline {
                tracing::warn!(
                    in_flight = in_flight,
                    "drain timeout reached with in-flight invocations, forcing shutdown"
                );
                break;
            }
            tracing::info!(in_flight = in_flight, "waiting for in-flight invocations to drain");
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        self.inner.running.store(false, Ordering::SeqCst);
        let _ = self.inner.outbound.send(Outbound::Shutdown);
        self.set_connection_state(IIIConnectionState::Disconnected);

        #[cfg(feature = "otel")]
        {
            telemetry::shutdown_otel().await;
        }
    }
```

- [ ] **Step 6: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii-sdk --lib`
Expected: All existing tests pass.

- [ ] **Step 7: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add sdk/packages/rust/iii/src/iii.rs
git commit -m "feat(sdk): graceful shutdown — drain in-flight invocations before disconnect"
```

---

## Dependency Graph

```
Task 1 (State enrichment) ──────────┐
Task 2 (Docker stop timeout) ───────┤
                                     ├── Task 4 (Health check loop) ── Task 5 (Status output)
Task 3 (Resource enforcement) ──────┘
Task 6 (SDK shutdown drain) ———— independent
```

Parallelizable:
- Tasks 1 + 2 + 3 + 6 are all independent
- Task 4 depends on 1 + 2 (uses new state fields and stop timeout)
- Task 5 depends on 1 + 4 (displays new state fields)
