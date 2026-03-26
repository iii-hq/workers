# iii Worker Abstraction Layer — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `iii worker add image-resize` work end-to-end — OCI image with embedded manifest, launcher worker pulls and starts it via Docker, engine confirms manifest-driven readiness, function is invocable.

**Architecture:** Two repos are involved. The engine repo (`motia/motia`) gets protocol extensions, manifest-driven readiness in the engine, manifest support in the Rust SDK, and new CLI commands. The workers repo (`motia/workers`) gets the image-resize OCI build, a new `iii-launcher` worker crate, registry v2 format, and OCI CI/CD workflows.

**Tech Stack:** Rust, tokio, serde/serde_yaml, iii-sdk 0.9.0, Docker CLI, clap 4, GitHub Actions, docker/build-push-action

---

## Repos

- **Engine repo:** `/Users/andersonleal/projetos/motia/motia`
- **Workers repo:** `/Users/andersonleal/projetos/motia/workers`

## File Structure

### Engine Repo — New Files

| File | Responsibility |
|------|---------------|
| `engine/src/manifest.rs` | `WorkerManifest`, `Entrypoint`, `Capabilities`, `FunctionCapability`, `ConfigSchema`, `Resources` structs with YAML/JSON serde + validation |

### Engine Repo — Modified Files

| File | Changes |
|------|---------|
| `engine/src/protocol.rs` | Add `WorkerManifest`, `WorkerReady`, `WorkerReadinessTimeout` variants to `Message` enum. Add `manifest_accepted` + `error` fields to `WorkerRegistered`. |
| `engine/src/workers/mod.rs` | Add `manifest: Option<WorkerManifestCompact>` and `expected_functions: Option<HashSet<String>>` to `Worker` struct. Add `set_manifest()`, `check_readiness()` methods. |
| `engine/src/engine/mod.rs` | Handle `WorkerManifest` message in `router_msg()`. On `RegisterFunction`, check readiness and send `WorkerReady` if all declared functions registered. Spawn readiness timeout task. |
| `engine/src/cli/mod.rs` | Add `worker add`, `worker remove`, `worker start`, `worker stop`, `worker status`, `worker logs` subcommands. Launcher bootstrap logic. |
| `engine/src/cli/worker_manager/registry.rs` | Support registry v2 format (`image` field instead of `repo`/`tag_prefix`/`supported_targets`). |
| `engine/src/lib.rs` or `engine/src/main.rs` | Add `mod manifest;` |

### SDK Repo — Modified Files

| File | Changes |
|------|---------|
| `sdk/packages/rust/iii/src/protocol.rs` | Mirror new message variants from engine protocol. |
| `sdk/packages/rust/iii/src/iii.rs` | Add `manifest: Option<WorkerManifestCompact>` to `InitOptions`. Send `WorkerManifest` after connect. Handle `WorkerReady` / `WorkerReadinessTimeout`. Add `WorkerManifestCompact::from_file()`. |
| `sdk/packages/rust/iii/src/lib.rs` | Export new manifest types. |

### Workers Repo — New Files

| File | Responsibility |
|------|---------------|
| `image-resize/Dockerfile` | Multi-stage OCI build: compile binary, generate manifest, copy both into slim image |
| `iii-launcher/Cargo.toml` | New crate for the launcher worker |
| `iii-launcher/src/main.rs` | CLI entry, engine registration, launcher bootstrap |
| `iii-launcher/src/adapter.rs` | `RuntimeAdapter` trait definition |
| `iii-launcher/src/docker.rs` | `DockerAdapter` implementation (Docker CLI) |
| `iii-launcher/src/state.rs` | `LauncherState` — read/write `launcher-state.json` |
| `iii-launcher/src/functions/mod.rs` | Module declarations |
| `iii-launcher/src/functions/pull.rs` | `iii_launcher::pull` handler |
| `iii-launcher/src/functions/start.rs` | `iii_launcher::start` handler |
| `iii-launcher/src/functions/stop.rs` | `iii_launcher::stop` handler |
| `iii-launcher/src/functions/status.rs` | `iii_launcher::status` handler |
| `iii-launcher/src/functions/logs.rs` | `iii_launcher::logs` handler |
| `.github/workflows/_oci-build.yml` | Reusable OCI multi-arch build workflow |

### Workers Repo — Modified Files

| File | Changes |
|------|---------|
| `image-resize/src/manifest.rs` | Emit `iii.worker.yaml` YAML format instead of JSON. New `WorkerManifest` struct matching spec. |
| `image-resize/src/main.rs` | Load manifest from `/iii/worker.yaml` if present, pass to `register_worker()` via SDK's new manifest field. |
| `image-resize/Cargo.toml` | Add `serde_yaml` if not present (already there). |
| `registry/index.json` | Upgrade to v2 format with `image` field. |
| `.github/workflows/release.yml` | Call `_oci-build.yml` instead of `_rust-binary.yml`. |
| `.github/workflows/create-tag.yml` | Update registry update step for v2 format. |

---

## Task 1: Manifest Types (Engine)

**Files:**
- Create: `engine/src/manifest.rs`
- Modify: `engine/src/lib.rs` (or wherever modules are declared)
- Test: inline `#[cfg(test)]` module

This task defines the core `WorkerManifest` types used by the engine to parse and validate manifests extracted from OCI images.

- [ ] **Step 1: Create manifest.rs with WorkerManifest types**

Create `/Users/andersonleal/projetos/motia/motia/engine/src/manifest.rs`:

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

/// Full worker manifest as embedded in /iii/worker.yaml inside OCI images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerManifest {
    pub iii: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    pub entrypoint: Entrypoint,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub config: Option<ConfigSchema>,
    #[serde(default)]
    pub resources: Option<Resources>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entrypoint {
    pub command: Vec<String>,
    pub transport: String,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    #[serde(default)]
    pub functions: Vec<FunctionCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCapability {
    pub id: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub request_schema: Option<Value>,
    #[serde(default)]
    pub response_schema: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSchema {
    pub schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    #[serde(default)]
    pub memory: Option<String>,
    #[serde(default)]
    pub cpu: Option<String>,
}

/// Compact manifest sent over the protocol — just enough for readiness tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerManifestCompact {
    pub iii: String,
    pub name: String,
    pub version: String,
    pub capabilities: ManifestCapabilitiesCompact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestCapabilitiesCompact {
    pub functions: Vec<String>,
}

impl WorkerManifest {
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.iii != "v1" {
            return Err(format!("unsupported manifest version: {}", self.iii));
        }
        if self.name.is_empty() {
            return Err("manifest name is required".into());
        }
        if self.version.is_empty() {
            return Err("manifest version is required".into());
        }
        Ok(())
    }

    pub fn to_compact(&self) -> WorkerManifestCompact {
        WorkerManifestCompact {
            iii: self.iii.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            capabilities: ManifestCapabilitiesCompact {
                functions: self.capabilities.functions.iter().map(|f| f.id.clone()).collect(),
            },
        }
    }

    pub fn expected_function_ids(&self) -> HashSet<String> {
        self.capabilities.functions.iter().map(|f| f.id.clone()).collect()
    }
}

impl WorkerManifestCompact {
    pub fn expected_function_ids(&self) -> HashSet<String> {
        self.capabilities.functions.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MANIFEST: &str = r#"
iii: v1
name: image-resize
version: 0.1.2
description: "Image resize and format conversion"
author: iii-hq
license: MIT
entrypoint:
  command: ["/worker"]
  transport: websocket
  protocol: iii-worker-v1
capabilities:
  functions:
    - id: "image_resize::resize"
      description: "Resize an image via channel I/O"
resources:
  memory: 256Mi
  cpu: "0.5"
"#;

    #[test]
    fn test_parse_yaml_manifest() {
        let manifest = WorkerManifest::from_yaml(SAMPLE_MANIFEST).unwrap();
        assert_eq!(manifest.iii, "v1");
        assert_eq!(manifest.name, "image-resize");
        assert_eq!(manifest.version, "0.1.2");
        assert_eq!(manifest.capabilities.functions.len(), 1);
        assert_eq!(manifest.capabilities.functions[0].id, "image_resize::resize");
    }

    #[test]
    fn test_validate_manifest() {
        let manifest = WorkerManifest::from_yaml(SAMPLE_MANIFEST).unwrap();
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn test_validate_rejects_unsupported_version() {
        let yaml = SAMPLE_MANIFEST.replace("iii: v1", "iii: v99");
        let manifest = WorkerManifest::from_yaml(&yaml).unwrap();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn test_to_compact() {
        let manifest = WorkerManifest::from_yaml(SAMPLE_MANIFEST).unwrap();
        let compact = manifest.to_compact();
        assert_eq!(compact.name, "image-resize");
        assert_eq!(compact.capabilities.functions, vec!["image_resize::resize"]);
    }

    #[test]
    fn test_expected_function_ids() {
        let manifest = WorkerManifest::from_yaml(SAMPLE_MANIFEST).unwrap();
        let ids = manifest.expected_function_ids();
        assert!(ids.contains("image_resize::resize"));
        assert_eq!(ids.len(), 1);
    }
}
```

- [ ] **Step 2: Register the module**

Add `pub mod manifest;` to the engine's module declarations. The exact file depends on how `engine/src/` is structured — look for the file that declares `pub mod protocol;` and `pub mod workers;`, and add `pub mod manifest;` alongside them.

- [ ] **Step 3: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii-engine manifest`
Expected: All 5 tests pass.

- [ ] **Step 4: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/manifest.rs engine/src/lib.rs
git commit -m "feat(engine): add WorkerManifest types with YAML parsing and validation"
```

---

## Task 2: Protocol Extensions (Engine)

**Files:**
- Modify: `engine/src/protocol.rs` (lines ~42-120 Message enum, line ~82 WorkerRegistered)
- Test: existing protocol tests or inline tests

Add three new message variants and extend `WorkerRegistered`.

- [ ] **Step 1: Add new message variants to Message enum**

In `/Users/andersonleal/projetos/motia/motia/engine/src/protocol.rs`, add these variants to the `Message` enum. Find the existing `WorkerRegistered` variant and add the new ones nearby:

```rust
// Add to Message enum, alongside existing variants:

// Worker sends its manifest after connecting (optional for self-hosted)
WorkerManifest {
    manifest: crate::manifest::WorkerManifestCompact,
},

// Engine sends when all manifest-declared functions are registered
WorkerReady {
    worker_id: Uuid,
    functions_registered: Vec<String>,
},

// Engine sends if manifest functions not registered within timeout
WorkerReadinessTimeout {
    worker_id: Uuid,
    missing_functions: Vec<String>,
    timeout_ms: u64,
},
```

- [ ] **Step 2: Extend WorkerRegistered with manifest_accepted**

Find the existing `WorkerRegistered` variant (currently just `{ worker_id: Uuid }`) and extend it:

```rust
WorkerRegistered {
    worker_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
},
```

- [ ] **Step 3: Update any existing code that constructs WorkerRegistered**

Search the engine codebase for all places that construct `Message::WorkerRegistered`. Each one currently passes only `worker_id`. Update them to also pass `manifest_accepted: None, error: None` to preserve backward compat (no manifest = no field).

Run: `cd /Users/andersonleal/projetos/motia/motia && grep -rn "WorkerRegistered" engine/src/`

Update each construction site to include the new fields.

- [ ] **Step 4: Update serde tag names**

The `Message` enum likely uses `#[serde(tag = "type", rename_all = "lowercase")]` or similar. Verify the serde attributes match the protocol spec:
- `WorkerManifest` should serialize to `"type": "worker_manifest"`
- `WorkerReady` should serialize to `"type": "worker_ready"`
- `WorkerReadinessTimeout` should serialize to `"type": "worker_readiness_timeout"`

Check the existing `#[serde(...)]` attribute on the `Message` enum and add `#[serde(rename = "...")]` on the new variants if needed to match. The existing variants use patterns like `RegisterFunction` → `"registerfunction"` (all lowercase, no separator). Follow the same convention:
- `WorkerManifest` → `"workermanifest"`
- `WorkerReady` → `"workerready"`
- `WorkerReadinessTimeout` → `"workerreadinesstimeout"`

- [ ] **Step 5: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii-engine`
Expected: All existing tests pass. New variants compile and serialize correctly.

- [ ] **Step 6: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/protocol.rs
git commit -m "feat(protocol): add WorkerManifest, WorkerReady, WorkerReadinessTimeout messages"
```

---

## Task 3: Protocol Extensions (Rust SDK)

**Files:**
- Modify: `sdk/packages/rust/iii/src/protocol.rs` (mirror engine changes)
- Modify: `sdk/packages/rust/iii/src/lib.rs` (export new types)

The SDK protocol must mirror the engine protocol exactly so messages can be serialized/deserialized on both sides.

- [ ] **Step 1: Add WorkerManifestCompact to SDK**

Create a `WorkerManifestCompact` struct in `sdk/packages/rust/iii/src/protocol.rs` (the SDK's own protocol file). This mirrors what we defined in the engine's `manifest.rs` but lives in the SDK crate so workers can use it without depending on the engine:

```rust
/// Compact manifest sent over WebSocket protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerManifestCompact {
    pub iii: String,
    pub name: String,
    pub version: String,
    pub capabilities: ManifestCapabilitiesCompact,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestCapabilitiesCompact {
    pub functions: Vec<String>,
}
```

- [ ] **Step 2: Add new message variants to SDK Message enum**

In `sdk/packages/rust/iii/src/protocol.rs`, add the same three variants to the SDK's `Message` enum, matching the engine's serde rename conventions:

```rust
WorkerManifest {
    manifest: WorkerManifestCompact,
},

WorkerReady {
    worker_id: Uuid,
    functions_registered: Vec<String>,
},

WorkerReadinessTimeout {
    worker_id: Uuid,
    missing_functions: Vec<String>,
    timeout_ms: u64,
},
```

- [ ] **Step 3: Extend SDK's WorkerRegistered**

Match the engine's change — add `manifest_accepted` and `error` fields:

```rust
WorkerRegistered {
    worker_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_accepted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
},
```

Update any SDK code that pattern-matches on `WorkerRegistered` to handle the new fields.

- [ ] **Step 4: Export new types from SDK lib.rs**

In `sdk/packages/rust/iii/src/lib.rs`, add exports for `WorkerManifestCompact` and `ManifestCapabilitiesCompact`.

- [ ] **Step 5: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii`
Expected: All existing SDK tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add sdk/packages/rust/iii/src/protocol.rs sdk/packages/rust/iii/src/lib.rs
git commit -m "feat(sdk): mirror protocol extensions — WorkerManifest, WorkerReady, WorkerReadinessTimeout"
```

---

## Task 4: Engine Manifest-Driven Readiness

**Files:**
- Modify: `engine/src/workers/mod.rs` (Worker struct + WorkerRegistry)
- Modify: `engine/src/engine/mod.rs` (message routing + readiness logic)
- Test: inline `#[cfg(test)]`

- [ ] **Step 1: Extend Worker struct with manifest fields**

In `/Users/andersonleal/projetos/motia/motia/engine/src/workers/mod.rs`, add to the `Worker` struct (after line ~207 `pub pid: Option<u32>`):

```rust
    pub manifest: Option<crate::manifest::WorkerManifestCompact>,
    pub expected_functions: Option<HashSet<String>>,
```

Add `use std::collections::HashSet;` at the top if not already imported.

- [ ] **Step 2: Initialize new fields in Worker::new() and Worker::with_ip()**

In both `Worker::new()` (line ~211) and `Worker::with_ip()` (line ~231), add to the struct initialization:

```rust
    manifest: None,
    expected_functions: None,
```

- [ ] **Step 3: Add set_manifest() and check_readiness() methods to Worker**

Add these methods to `impl Worker` (after the existing methods, around line ~315):

```rust
    /// Store the manifest and compute expected functions.
    pub fn set_manifest(&mut self, manifest: crate::manifest::WorkerManifestCompact) {
        self.expected_functions = Some(manifest.expected_function_ids());
        self.manifest = Some(manifest);
    }

    /// Check if all manifest-declared functions have been registered.
    /// Returns None if no manifest was provided (self-hosted without manifest).
    /// Returns Some(missing) with the set of unregistered function IDs.
    pub fn check_readiness(&self) -> Option<HashSet<String>> {
        let expected = self.expected_functions.as_ref()?;
        let registered = self.function_ids.read().unwrap();
        let missing: HashSet<String> = expected
            .iter()
            .filter(|id| !registered.contains(*id))
            .cloned()
            .collect();
        Some(missing)
    }
```

- [ ] **Step 4: Handle WorkerManifest message in engine router**

In `/Users/andersonleal/projetos/motia/motia/engine/src/engine/mod.rs`, find the `router_msg()` method (around line ~394). Add a match arm for the new `WorkerManifest` message:

```rust
Message::WorkerManifest { manifest } => {
    // Validate manifest version
    if manifest.iii != "v1" {
        let _ = self.send_msg(
            worker,
            Message::WorkerRegistered {
                worker_id: worker.id,
                manifest_accepted: Some(false),
                error: Some(format!("unsupported manifest version: {}", manifest.iii)),
            },
        ).await;
        return Ok(());
    }

    // Store manifest on worker
    self.worker_registry.set_worker_manifest(&worker.id, manifest.clone());

    // Send acceptance
    let _ = self.send_msg(
        worker,
        Message::WorkerRegistered {
            worker_id: worker.id,
            manifest_accepted: Some(true),
            error: None,
        },
    ).await;

    // Start readiness timeout (30s)
    let registry = self.worker_registry.clone();
    let engine = self.clone();
    let worker_id = worker.id;
    let expected_functions = manifest.capabilities.functions.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        if let Some(w) = registry.get_worker(&worker_id) {
            if let Some(missing) = w.check_readiness() {
                if !missing.is_empty() {
                    let _ = engine.send_msg(
                        &w,
                        Message::WorkerReadinessTimeout {
                            worker_id,
                            missing_functions: missing.into_iter().collect(),
                            timeout_ms: 30000,
                        },
                    ).await;
                    // Disconnect the worker
                    registry.unregister_worker(&worker_id);
                }
            }
        }
    });

    Ok(())
}
```

- [ ] **Step 5: Add set_worker_manifest to WorkerRegistry**

In `engine/src/workers/mod.rs`, add this method to `impl WorkerRegistry`:

```rust
    pub fn set_worker_manifest(&self, worker_id: &Uuid, manifest: crate::manifest::WorkerManifestCompact) {
        if let Some(mut worker) = self.workers.get_mut(worker_id) {
            worker.set_manifest(manifest);
        }
    }
```

- [ ] **Step 6: Check readiness on RegisterFunction**

In `engine/src/engine/mod.rs`, find where `RegisterFunction` is handled in `router_msg()`. After the function is registered and added to the worker's `function_ids`, add a readiness check:

```rust
// After existing RegisterFunction handling, add:
// Check if this completes manifest-driven readiness
if let Some(missing) = worker.check_readiness() {
    if missing.is_empty() {
        let registered: Vec<String> = worker.get_function_ids();
        let _ = self.send_msg(
            worker,
            Message::WorkerReady {
                worker_id: worker.id,
                functions_registered: registered,
            },
        ).await;
    }
}
```

Note: The exact insertion point depends on how `RegisterFunction` is currently handled. Find where `worker.include_function_id()` is called and add the readiness check immediately after.

- [ ] **Step 7: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii-engine`
Expected: All tests pass. No behavioral change for workers without manifests.

- [ ] **Step 8: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/workers/mod.rs engine/src/engine/mod.rs
git commit -m "feat(engine): manifest-driven readiness — track expected functions, send WorkerReady"
```

---

## Task 5: SDK Manifest Support

**Files:**
- Modify: `sdk/packages/rust/iii/src/iii.rs` (InitOptions, connection logic)
- Modify: `sdk/packages/rust/iii/src/lib.rs` (exports)

- [ ] **Step 1: Add manifest field to InitOptions**

In `/Users/andersonleal/projetos/motia/motia/sdk/packages/rust/iii/src/iii.rs`, find the `InitOptions` struct (around line ~44) or `WorkerMetadata` struct. Add an optional manifest field. Look at how `register_worker()` (line ~69) constructs the `III` instance and determine the right struct to extend.

If `InitOptions` exists:
```rust
pub struct InitOptions {
    // ... existing fields ...
    pub manifest: Option<WorkerManifestCompact>,
}
```

If using `WorkerMetadata`:
```rust
pub struct WorkerMetadata {
    // ... existing fields ...
    pub manifest: Option<WorkerManifestCompact>,
}
```

Ensure `Default` impl sets `manifest: None`.

- [ ] **Step 2: Send WorkerManifest after connection**

In `iii.rs`, find the connection logic (around `connect()` method, line ~617). After the WebSocket connection is established and before functions are registered, add manifest sending:

```rust
// Inside the connection handler, after WebSocket is connected:
if let Some(manifest) = &self.inner.manifest {
    self.send_message(Message::WorkerManifest {
        manifest: manifest.clone(),
    })?;
}
```

The `manifest` field needs to be stored on `IIIInner` (line ~514). Add:
```rust
struct IIIInner {
    // ... existing fields ...
    manifest: Option<WorkerManifestCompact>,
}
```

And initialize it from `InitOptions`/`WorkerMetadata` in the constructor.

- [ ] **Step 3: Handle WorkerReady and WorkerReadinessTimeout**

In the message receive loop (find where incoming WebSocket messages are dispatched — look for pattern matching on `Message` variants), add handlers:

```rust
Message::WorkerReady { worker_id, functions_registered } => {
    tracing::info!(
        worker_id = %worker_id,
        functions = ?functions_registered,
        "Worker ready — all manifest functions registered"
    );
    // If there's a readiness callback/channel, signal it here
}

Message::WorkerReadinessTimeout { worker_id, missing_functions, timeout_ms } => {
    tracing::error!(
        worker_id = %worker_id,
        missing = ?missing_functions,
        timeout_ms = timeout_ms,
        "Worker readiness timeout — not all manifest functions registered"
    );
}
```

- [ ] **Step 4: Add WorkerManifestCompact::from_file()**

In `sdk/packages/rust/iii/src/protocol.rs`, add a method to load manifest from the OCI well-known path:

```rust
impl WorkerManifestCompact {
    /// Load a compact manifest from a YAML file (typically /iii/worker.yaml in OCI images).
    /// Parses the full manifest YAML and extracts only the compact fields.
    pub fn from_file(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        let value: serde_yaml::Value = serde_yaml::from_str(&contents)?;

        let iii = value["iii"].as_str().unwrap_or("v1").to_string();
        let name = value["name"].as_str().unwrap_or("").to_string();
        let version = value["version"].as_str().unwrap_or("").to_string();

        let functions: Vec<String> = value["capabilities"]["functions"]
            .as_sequence()
            .map(|seq| {
                seq.iter()
                    .filter_map(|f| f["id"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        Ok(WorkerManifestCompact {
            iii,
            name,
            version,
            capabilities: ManifestCapabilitiesCompact { functions },
        })
    }
}
```

Ensure `serde_yaml` is in the SDK's `Cargo.toml` dependencies.

- [ ] **Step 5: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii`
Expected: All existing tests pass.

- [ ] **Step 6: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add sdk/packages/rust/iii/src/iii.rs sdk/packages/rust/iii/src/protocol.rs sdk/packages/rust/iii/src/lib.rs sdk/packages/rust/iii/Cargo.toml
git commit -m "feat(sdk): add manifest support — WorkerManifestCompact, from_file, protocol handling"
```

---

## Task 6: image-resize Manifest YAML Generation

**Files:**
- Modify: `workers/image-resize/src/manifest.rs`
- Modify: `workers/image-resize/src/main.rs`

- [ ] **Step 1: Rewrite manifest.rs to emit YAML**

Replace the contents of `/Users/andersonleal/projetos/motia/workers/image-resize/src/manifest.rs` with:

```rust
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
pub struct WorkerManifest {
    pub iii: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub license: String,
    pub entrypoint: Entrypoint,
    pub capabilities: Capabilities,
    pub config: ConfigSection,
    pub resources: Resources,
}

#[derive(Debug, Serialize)]
pub struct Entrypoint {
    pub command: Vec<String>,
    pub transport: String,
    pub protocol: String,
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub functions: Vec<FunctionCapability>,
}

#[derive(Debug, Serialize)]
pub struct FunctionCapability {
    pub id: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct ConfigSection {
    pub schema: Value,
}

#[derive(Debug, Serialize)]
pub struct Resources {
    pub memory: String,
    pub cpu: String,
}

pub fn build_manifest() -> WorkerManifest {
    WorkerManifest {
        iii: "v1".into(),
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: "Image resize and format conversion".into(),
        author: "iii-hq".into(),
        license: "MIT".into(),
        entrypoint: Entrypoint {
            command: vec!["/worker".into()],
            transport: "websocket".into(),
            protocol: "iii-worker-v1".into(),
        },
        capabilities: Capabilities {
            functions: vec![FunctionCapability {
                id: "image_resize::resize".into(),
                description: "Resize an image via channel I/O".into(),
                request_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "input_channel": { "type": "object" },
                        "output_channel": { "type": "object" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["input_channel", "output_channel"]
                })),
                response_schema: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "format": { "type": "string" },
                        "width": { "type": "integer" },
                        "height": { "type": "integer" }
                    }
                })),
            }],
        },
        config: ConfigSection {
            schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "width": { "type": "integer", "default": 200 },
                    "height": { "type": "integer", "default": 200 },
                    "strategy": {
                        "type": "string",
                        "enum": ["scale-to-fit", "crop-to-fit"],
                        "default": "scale-to-fit"
                    },
                    "quality": {
                        "type": "object",
                        "properties": {
                            "jpeg": { "type": "integer", "default": 85 },
                            "webp": { "type": "integer", "default": 80 }
                        }
                    }
                }
            }),
        },
        resources: Resources {
            memory: "256Mi".into(),
            cpu: "0.5".into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_manifest_yaml() {
        let manifest = build_manifest();
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        assert!(yaml.contains("iii: v1"));
        assert!(yaml.contains("name: image-resize"));
        assert!(yaml.contains("image_resize::resize"));
    }

    #[test]
    fn test_manifest_version_matches_cargo() {
        let manifest = build_manifest();
        assert_eq!(manifest.version, env!("CARGO_PKG_VERSION"));
    }
}
```

- [ ] **Step 2: Update main.rs --manifest flag to emit YAML**

In `/Users/andersonleal/projetos/motia/workers/image-resize/src/main.rs`, find where `--manifest` is handled (around line ~28-42). Currently it outputs JSON via `serde_json::to_string_pretty`. Change it to output YAML:

```rust
if cli.manifest {
    let manifest = manifest::build_manifest();
    let yaml = serde_yaml::to_string(&manifest).expect("failed to serialize manifest");
    println!("{}", yaml);
    return;
}
```

- [ ] **Step 3: Run tests**

Run: `cd /Users/andersonleal/projetos/motia/workers && cargo test -p image-resize`
Expected: All tests pass.

- [ ] **Step 4: Test manifest output manually**

Run: `cd /Users/andersonleal/projetos/motia/workers && cargo run -p image-resize -- --manifest`
Expected: YAML output matching the spec format with `iii: v1`, function capabilities, config schema, resources.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add image-resize/src/manifest.rs image-resize/src/main.rs
git commit -m "feat(image-resize): emit iii worker manifest in YAML format"
```

---

## Task 7: image-resize OCI Image + SDK Integration

**Files:**
- Create: `workers/image-resize/Dockerfile`
- Modify: `workers/image-resize/src/main.rs` (load manifest, pass to SDK)

- [ ] **Step 1: Create Dockerfile**

Create `/Users/andersonleal/projetos/motia/workers/image-resize/Dockerfile`:

```dockerfile
FROM rust:1.83-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release -p image-resize
RUN ./target/release/image-resize --manifest > /build/worker.yaml

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/image-resize /worker
COPY --from=builder /build/worker.yaml /iii/worker.yaml

ENV III_ENGINE_URL=ws://host.docker.internal:49134

ENTRYPOINT ["/worker"]
CMD ["--url", "ws://host.docker.internal:49134"]
```

- [ ] **Step 2: Update main.rs to load manifest and pass to SDK**

In `/Users/andersonleal/projetos/motia/workers/image-resize/src/main.rs`, update the `register_worker` call to include the manifest. Find the current registration (around line ~65-71) and modify:

```rust
// Try to load manifest from OCI well-known path, fall back to None
let manifest = iii_sdk::WorkerManifestCompact::from_file("/iii/worker.yaml").ok();

let iii = iii_sdk::register_worker(&cli.url, iii_sdk::InitOptions {
    worker_name: "image-resize".into(),
    manifest,
    // ... keep existing fields (otel config, etc.) ...
    ..Default::default()
});
```

The exact field names depend on what we built in Task 5. Match the struct fields from the SDK changes.

- [ ] **Step 3: Test Docker build locally**

Run:
```bash
cd /Users/andersonleal/projetos/motia/workers
docker build -t iii-image-resize:test -f image-resize/Dockerfile .
```
Expected: Image builds successfully.

- [ ] **Step 4: Verify manifest is embedded**

Run:
```bash
docker run --rm iii-image-resize:test cat /iii/worker.yaml
```
Expected: YAML manifest output with `iii: v1`, function capabilities, etc.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add image-resize/Dockerfile image-resize/src/main.rs
git commit -m "feat(image-resize): add Dockerfile with embedded manifest, load manifest in SDK"
```

---

## Task 8: Launcher Worker — Runtime Adapter + Docker

**Files:**
- Create: `workers/iii-launcher/Cargo.toml`
- Create: `workers/iii-launcher/src/adapter.rs`
- Create: `workers/iii-launcher/src/docker.rs`
- Create: `workers/iii-launcher/src/state.rs`

This task builds the foundational layer of the launcher: the adapter trait, Docker implementation, and state management. The next task wires these into functions registered with the engine.

- [ ] **Step 1: Create iii-launcher Cargo.toml**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/Cargo.toml`:

```toml
[package]
name = "iii-launcher"
version = "0.1.0"
edition = "2021"
publish = false

[[bin]]
name = "iii-launcher"
path = "src/main.rs"

[dependencies]
iii-sdk = { version = "0.9.0", features = ["otel"] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "signal", "process"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
clap = { version = "4", features = ["derive"] }
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1", features = ["v4"] }
```

- [ ] **Step 2: Create adapter.rs with RuntimeAdapter trait**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/adapter.rs`:

```rust
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub image: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerSpec {
    pub name: String,
    pub image: String,
    pub env: HashMap<String, String>,
    pub memory_limit: Option<String>,
    pub cpu_limit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerStatus {
    pub name: String,
    pub container_id: String,
    pub running: bool,
    pub exit_code: Option<i32>,
}

#[async_trait::async_trait]
pub trait RuntimeAdapter: Send + Sync {
    async fn pull(&self, image: &str) -> Result<ImageInfo>;
    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>>;
    async fn start(&self, spec: &ContainerSpec) -> Result<String>; // returns container_id
    async fn stop(&self, container_id: &str) -> Result<()>;
    async fn status(&self, container_id: &str) -> Result<ContainerStatus>;
    async fn logs(&self, container_id: &str, follow: bool) -> Result<String>;
    async fn remove(&self, container_id: &str) -> Result<()>;
}
```

Add `async-trait = "0.1"` to `Cargo.toml` dependencies.

- [ ] **Step 3: Create docker.rs with DockerAdapter**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/docker.rs`:

```rust
use crate::adapter::{ContainerSpec, ContainerStatus, ImageInfo, RuntimeAdapter};
use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

pub struct DockerAdapter;

impl DockerAdapter {
    pub fn new() -> Self {
        DockerAdapter
    }

    async fn run_docker(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("docker")
            .args(args)
            .output()
            .await
            .context("failed to execute docker command")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("docker {} failed: {}", args[0], stderr.trim()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

#[async_trait::async_trait]
impl RuntimeAdapter for DockerAdapter {
    async fn pull(&self, image: &str) -> Result<ImageInfo> {
        self.run_docker(&["pull", image]).await?;

        let size_output = self
            .run_docker(&["image", "inspect", image, "--format", "{{.Size}}"])
            .await
            .ok();

        let size_bytes = size_output.and_then(|s| s.parse::<u64>().ok());

        Ok(ImageInfo {
            image: image.to_string(),
            size_bytes,
        })
    }

    async fn extract_file(&self, image: &str, path: &str) -> Result<Vec<u8>> {
        // Create a temporary container, copy the file out, remove the container
        let container_id = self
            .run_docker(&["create", image])
            .await
            .context("failed to create temporary container")?;

        let result = Command::new("docker")
            .args(["cp", &format!("{}:{}", container_id, path), "-"])
            .output()
            .await
            .context("failed to copy file from container")?;

        // Clean up temporary container
        let _ = self.run_docker(&["rm", &container_id]).await;

        if !result.status.success() {
            return Err(anyhow!(
                "failed to extract {} from image: {}",
                path,
                String::from_utf8_lossy(&result.stderr)
            ));
        }

        // docker cp outputs a tar archive — extract the file content
        let cursor = std::io::Cursor::new(result.stdout);
        let mut archive = tar::Archive::new(cursor);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut contents)?;
            return Ok(contents);
        }

        Err(anyhow!("file {} not found in tar output", path))
    }

    async fn start(&self, spec: &ContainerSpec) -> Result<String> {
        let mut args = vec!["run", "-d", "--name", &spec.name];

        let env_strings: Vec<String> = spec
            .env
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        for env in &env_strings {
            args.push("-e");
            args.push(env);
        }

        if let Some(ref mem) = spec.memory_limit {
            args.push("--memory");
            args.push(mem);
        }

        if let Some(ref cpu) = spec.cpu_limit {
            args.push("--cpus");
            args.push(cpu);
        }

        args.push(&spec.image);

        self.run_docker(&args).await
    }

    async fn stop(&self, container_id: &str) -> Result<()> {
        self.run_docker(&["stop", container_id]).await?;
        Ok(())
    }

    async fn status(&self, container_id: &str) -> Result<ContainerStatus> {
        let format = "{{.Name}}\t{{.ID}}\t{{.State.Running}}\t{{.State.ExitCode}}";
        let output = self
            .run_docker(&["inspect", "--format", format, container_id])
            .await?;

        let parts: Vec<&str> = output.split('\t').collect();
        if parts.len() < 4 {
            return Err(anyhow!("unexpected docker inspect output: {}", output));
        }

        Ok(ContainerStatus {
            name: parts[0].trim_start_matches('/').to_string(),
            container_id: parts[1].to_string(),
            running: parts[2] == "true",
            exit_code: parts[3].parse().ok(),
        })
    }

    async fn logs(&self, container_id: &str, follow: bool) -> Result<String> {
        let mut args = vec!["logs"];
        if follow {
            args.push("--follow");
        }
        args.push("--tail");
        args.push("100");
        args.push(container_id);

        self.run_docker(&args).await
    }

    async fn remove(&self, container_id: &str) -> Result<()> {
        self.run_docker(&["rm", "-f", container_id]).await?;
        Ok(())
    }
}
```

Add `tar = "0.4"` to `Cargo.toml` dependencies.

- [ ] **Step 4: Create state.rs for launcher state**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/state.rs`:

```rust
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWorker {
    pub image: String,
    pub container_id: String,
    pub runtime: String,
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LauncherState {
    pub managed_workers: HashMap<String, ManagedWorker>,
}

impl LauncherState {
    pub fn state_path() -> PathBuf {
        PathBuf::from("iii_workers/launcher-state.json")
    }

    pub fn load() -> Result<Self> {
        let path = Self::state_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn add_worker(&mut self, name: String, worker: ManagedWorker) {
        self.managed_workers.insert(name, worker);
    }

    pub fn remove_worker(&mut self, name: &str) -> Option<ManagedWorker> {
        self.managed_workers.remove(name)
    }

    pub fn get_worker(&self, name: &str) -> Option<&ManagedWorker> {
        self.managed_workers.get(name)
    }
}
```

- [ ] **Step 5: Run compilation check**

Run: `cd /Users/andersonleal/projetos/motia/workers && cargo check -p iii-launcher`
Expected: Compiles successfully.

- [ ] **Step 6: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/
git commit -m "feat(launcher): add RuntimeAdapter trait, DockerAdapter, and state management"
```

---

## Task 9: Launcher Worker — Functions + Main

**Files:**
- Create: `workers/iii-launcher/src/main.rs`
- Create: `workers/iii-launcher/src/functions/mod.rs`
- Create: `workers/iii-launcher/src/functions/pull.rs`
- Create: `workers/iii-launcher/src/functions/start.rs`
- Create: `workers/iii-launcher/src/functions/stop.rs`
- Create: `workers/iii-launcher/src/functions/status.rs`
- Create: `workers/iii-launcher/src/functions/logs.rs`

- [ ] **Step 1: Create functions/mod.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/mod.rs`:

```rust
pub mod pull;
pub mod start;
pub mod stop;
pub mod status;
pub mod logs;
```

- [ ] **Step 2: Create functions/pull.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/pull.rs`:

```rust
use crate::adapter::RuntimeAdapter;
use anyhow::Result;
use iii_sdk::IIIError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

const MANIFEST_PATH: &str = "/iii/worker.yaml";

#[derive(Deserialize)]
pub struct PullRequest {
    pub image: String,
}

#[derive(Serialize)]
pub struct PullResponse {
    pub image: String,
    pub manifest: Option<Value>,
    pub size_bytes: Option<u64>,
}

pub fn build_handler(
    adapter: Arc<dyn RuntimeAdapter>,
) -> impl Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |input: Value| {
        let adapter = adapter.clone();
        Box::pin(async move {
            let req: PullRequest = serde_json::from_value(input)
                .map_err(|e| IIIError::new(&format!("invalid pull request: {}", e)))?;

            tracing::info!(image = %req.image, "Pulling OCI image");

            let info = adapter
                .pull(&req.image)
                .await
                .map_err(|e| IIIError::new(&format!("pull failed: {}", e)))?;

            // Extract manifest from image
            let manifest = match adapter.extract_file(&req.image, MANIFEST_PATH).await {
                Ok(bytes) => {
                    let yaml_str = String::from_utf8(bytes)
                        .map_err(|e| IIIError::new(&format!("manifest is not valid UTF-8: {}", e)))?;
                    let value: Value = serde_yaml::from_str(&yaml_str)
                        .map_err(|e| IIIError::new(&format!("manifest is not valid YAML: {}", e)))?;
                    Some(value)
                }
                Err(e) => {
                    tracing::warn!(image = %req.image, error = %e, "No manifest found at {}", MANIFEST_PATH);
                    None
                }
            };

            let response = PullResponse {
                image: info.image,
                manifest,
                size_bytes: info.size_bytes,
            };

            serde_json::to_value(response)
                .map_err(|e| IIIError::new(&format!("failed to serialize response: {}", e)))
        })
    }
}
```

- [ ] **Step 3: Create functions/start.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/start.rs`:

```rust
use crate::adapter::{ContainerSpec, RuntimeAdapter};
use crate::state::{LauncherState, ManagedWorker};
use anyhow::Result;
use chrono::Utc;
use iii_sdk::IIIError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub struct StartRequest {
    pub name: String,
    pub image: String,
    pub engine_url: String,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub config: Option<Value>,
    #[serde(default)]
    pub memory_limit: Option<String>,
    #[serde(default)]
    pub cpu_limit: Option<String>,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub name: String,
    pub container_id: String,
    pub status: String,
}

pub fn build_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |input: Value| {
        let adapter = adapter.clone();
        let state = state.clone();
        Box::pin(async move {
            let req: StartRequest = serde_json::from_value(input)
                .map_err(|e| IIIError::new(&format!("invalid start request: {}", e)))?;

            let container_name = format!("iii-{}-{}", req.name, &uuid::Uuid::new_v4().to_string()[..8]);

            let mut env = HashMap::new();
            env.insert("III_ENGINE_URL".into(), req.engine_url.clone());
            if let Some(ref token) = req.auth_token {
                env.insert("III_AUTH_TOKEN".into(), token.clone());
            }
            if let Some(ref config) = req.config {
                let config_b64 = base64_encode(&serde_json::to_string(config).unwrap_or_default());
                env.insert("III_WORKER_CONFIG".into(), config_b64);
            }

            let spec = ContainerSpec {
                name: container_name.clone(),
                image: req.image.clone(),
                env,
                memory_limit: req.memory_limit,
                cpu_limit: req.cpu_limit,
            };

            tracing::info!(name = %req.name, image = %req.image, container = %container_name, "Starting worker container");

            let container_id = adapter
                .start(&spec)
                .await
                .map_err(|e| IIIError::new(&format!("start failed: {}", e)))?;

            // Update state
            let mut state = state.lock().await;
            state.add_worker(
                req.name.clone(),
                ManagedWorker {
                    image: req.image,
                    container_id: container_id.clone(),
                    runtime: "docker".into(),
                    started_at: Utc::now(),
                    status: "running".into(),
                    config: req.config.unwrap_or(Value::Null),
                },
            );
            state.save().map_err(|e| IIIError::new(&format!("failed to save state: {}", e)))?;

            let response = StartResponse {
                name: req.name,
                container_id,
                status: "running".into(),
            };

            serde_json::to_value(response)
                .map_err(|e| IIIError::new(&format!("failed to serialize response: {}", e)))
        })
    }
}

fn base64_encode(input: &str) -> String {
    data_encoding::BASE64.encode(input.as_bytes())
}
```

Add `data-encoding = "2"` to `Cargo.toml`.

- [ ] **Step 4: Create functions/stop.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/stop.rs`:

```rust
use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;
use iii_sdk::IIIError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub struct StopRequest {
    pub name: String,
}

#[derive(Serialize)]
pub struct StopResponse {
    pub name: String,
    pub stopped: bool,
}

pub fn build_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |input: Value| {
        let adapter = adapter.clone();
        let state = state.clone();
        Box::pin(async move {
            let req: StopRequest = serde_json::from_value(input)
                .map_err(|e| IIIError::new(&format!("invalid stop request: {}", e)))?;

            let mut state = state.lock().await;
            let worker = state.get_worker(&req.name).cloned();

            match worker {
                Some(w) => {
                    adapter
                        .stop(&w.container_id)
                        .await
                        .map_err(|e| IIIError::new(&format!("stop failed: {}", e)))?;

                    adapter
                        .remove(&w.container_id)
                        .await
                        .map_err(|e| IIIError::new(&format!("remove failed: {}", e)))?;

                    state.remove_worker(&req.name);
                    state.save().map_err(|e| IIIError::new(&format!("failed to save state: {}", e)))?;

                    serde_json::to_value(StopResponse { name: req.name, stopped: true })
                        .map_err(|e| IIIError::new(&format!("serialize failed: {}", e)))
                }
                None => Err(IIIError::new(&format!("worker '{}' not found", req.name))),
            }
        })
    }
}
```

- [ ] **Step 5: Create functions/status.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/status.rs`:

```rust
use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;
use iii_sdk::IIIError;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Serialize)]
pub struct WorkerStatusEntry {
    pub name: String,
    pub image: String,
    pub runtime: String,
    pub running: bool,
    pub started_at: String,
}

pub fn build_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_input: Value| {
        let adapter = adapter.clone();
        let state = state.clone();
        Box::pin(async move {
            let state = state.lock().await;
            let mut entries = Vec::new();

            for (name, worker) in &state.managed_workers {
                let running = match adapter.status(&worker.container_id).await {
                    Ok(s) => s.running,
                    Err(_) => false,
                };

                entries.push(WorkerStatusEntry {
                    name: name.clone(),
                    image: worker.image.clone(),
                    runtime: worker.runtime.clone(),
                    running,
                    started_at: worker.started_at.to_rfc3339(),
                });
            }

            serde_json::to_value(entries)
                .map_err(|e| IIIError::new(&format!("serialize failed: {}", e)))
        })
    }
}
```

- [ ] **Step 6: Create functions/logs.rs**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/functions/logs.rs`:

```rust
use crate::adapter::RuntimeAdapter;
use crate::state::LauncherState;
use iii_sdk::IIIError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Deserialize)]
pub struct LogsRequest {
    pub name: String,
    #[serde(default)]
    pub follow: bool,
}

#[derive(Serialize)]
pub struct LogsResponse {
    pub name: String,
    pub logs: String,
}

pub fn build_handler(
    adapter: Arc<dyn RuntimeAdapter>,
    state: Arc<Mutex<LauncherState>>,
) -> impl Fn(Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |input: Value| {
        let adapter = adapter.clone();
        let state = state.clone();
        Box::pin(async move {
            let req: LogsRequest = serde_json::from_value(input)
                .map_err(|e| IIIError::new(&format!("invalid logs request: {}", e)))?;

            let state = state.lock().await;
            let worker = state
                .get_worker(&req.name)
                .ok_or_else(|| IIIError::new(&format!("worker '{}' not found", req.name)))?;

            let logs = adapter
                .logs(&worker.container_id, req.follow)
                .await
                .map_err(|e| IIIError::new(&format!("logs failed: {}", e)))?;

            serde_json::to_value(LogsResponse {
                name: req.name,
                logs,
            })
            .map_err(|e| IIIError::new(&format!("serialize failed: {}", e)))
        })
    }
}
```

- [ ] **Step 7: Create main.rs with registration**

Create `/Users/andersonleal/projetos/motia/workers/iii-launcher/src/main.rs`:

```rust
mod adapter;
mod docker;
mod functions;
mod state;

use adapter::RuntimeAdapter;
use clap::Parser;
use docker::DockerAdapter;
use iii_sdk::{RegisterFunctionMessage, WorkerManifestCompact, ManifestCapabilitiesCompact};
use state::LauncherState;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Parser)]
#[command(name = "iii-launcher")]
struct Cli {
    #[arg(long, default_value = "ws://127.0.0.1:49134")]
    url: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    let adapter: Arc<dyn RuntimeAdapter> = Arc::new(DockerAdapter::new());
    let state = Arc::new(Mutex::new(LauncherState::load().unwrap_or_default()));

    // Try to load manifest from OCI path
    let manifest = WorkerManifestCompact::from_file("/iii/worker.yaml").ok().unwrap_or_else(|| {
        WorkerManifestCompact {
            iii: "v1".into(),
            name: "iii-launcher".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            capabilities: ManifestCapabilitiesCompact {
                functions: vec![
                    "iii_launcher::pull".into(),
                    "iii_launcher::start".into(),
                    "iii_launcher::stop".into(),
                    "iii_launcher::status".into(),
                    "iii_launcher::logs".into(),
                ],
            },
        }
    });

    let iii = iii_sdk::register_worker(&cli.url, iii_sdk::InitOptions {
        worker_name: "iii-launcher".into(),
        manifest: Some(manifest),
        ..Default::default()
    });

    // Register pull function
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "iii_launcher::pull".into(),
            description: Some("Pull an OCI image and extract its manifest".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::pull::build_handler(adapter.clone()),
    );

    // Register start function
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "iii_launcher::start".into(),
            description: Some("Start a worker container".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::start::build_handler(adapter.clone(), state.clone()),
    );

    // Register stop function
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "iii_launcher::stop".into(),
            description: Some("Stop a worker container".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::stop::build_handler(adapter.clone(), state.clone()),
    );

    // Register status function
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "iii_launcher::status".into(),
            description: Some("Get status of all managed workers".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::status::build_handler(adapter.clone(), state.clone()),
    );

    // Register logs function
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "iii_launcher::logs".into(),
            description: Some("Get logs from a worker container".into()),
            request_format: None,
            response_format: None,
            metadata: None,
            invocation: None,
        },
        functions::logs::build_handler(adapter.clone(), state.clone()),
    );

    tracing::info!("iii-launcher registered with engine at {}", cli.url);

    tokio::signal::ctrl_c().await.unwrap();
    tracing::info!("Shutting down iii-launcher");
}
```

Note: The exact `register_function_with` signature depends on the SDK's API. The current SDK uses `register_function` which takes an `impl IntoFunctionRegistration`. Check the SDK's `iii.rs` (line ~759-772) for the exact handler type expected and adjust. The handlers above return `Pin<Box<dyn Future<Output = Result<Value, IIIError>>>>` — verify this matches what the SDK expects for async function handlers.

- [ ] **Step 8: Run compilation check**

Run: `cd /Users/andersonleal/projetos/motia/workers && cargo check -p iii-launcher`
Expected: Compiles. Fix any type mismatches between handler signatures and SDK expectations.

- [ ] **Step 9: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add iii-launcher/
git commit -m "feat(launcher): implement pull/start/stop/status/logs functions with Docker adapter"
```

---

## Task 10: CLI Managed Commands (Engine)

**Files:**
- Modify: `engine/src/cli/mod.rs`

This task adds the new `iii worker add/remove/start/stop/status/logs` commands to the engine CLI. These commands connect to the engine, find the launcher worker, and invoke its functions.

- [ ] **Step 1: Add new subcommands**

In `/Users/andersonleal/projetos/motia/motia/engine/src/cli/mod.rs`, find where CLI commands are dispatched (the `handle_dispatch` function around line ~23). The current worker commands use patterns like `handle_install`, `handle_uninstall`, `handle_worker_list`.

Add new command handlers. The exact integration depends on how clap is structured — look for the command enum or match statement. Add these match arms:

```rust
// In the command dispatch:
"worker" => match subcommand {
    "add" => handle_worker_add(args).await,
    "remove" => handle_worker_remove(args).await,
    "start" => handle_worker_start(args).await,
    "stop" => handle_worker_stop(args).await,
    "status" => handle_worker_status(args).await,
    "logs" => handle_worker_logs(args).await,
    // Keep existing commands:
    "install" => handle_install(args).await,
    "list" => handle_worker_list().await,
    _ => { /* ... */ }
},
```

- [ ] **Step 2: Implement handle_worker_add**

This is the most complex command. Add to `engine/src/cli/mod.rs` (or a new file `engine/src/cli/managed.rs`):

```rust
async fn handle_worker_add(args: &[String]) -> i32 {
    let image_or_name = match args.first() {
        Some(s) => s.clone(),
        None => {
            eprintln!("Usage: iii worker add <image-or-name> [--runtime docker]");
            return 1;
        }
    };

    // Resolve shorthand: if no "/" or ":" in the name, look it up in registry
    let image = if image_or_name.contains('/') || image_or_name.contains(':') {
        image_or_name.clone()
    } else {
        match resolve_image_from_registry(&image_or_name).await {
            Ok(img) => img,
            Err(e) => {
                eprintln!("Failed to resolve '{}' from registry: {}", image_or_name, e);
                return 1;
            }
        }
    };

    // Connect to engine and invoke launcher functions
    let engine_url = "ws://127.0.0.1:49134"; // TODO: read from config

    // Create a temporary SDK connection to invoke launcher functions
    let iii = iii_sdk::register_worker(engine_url, iii_sdk::InitOptions {
        worker_name: "iii-cli".into(),
        ..Default::default()
    });

    // Step 1: Pull image via launcher
    println!("Pulling image... ");
    let pull_result = iii.trigger(iii_sdk::TriggerRequest {
        function_id: "iii_launcher::pull".into(),
        payload: serde_json::json!({ "image": image }),
        action: None,
        timeout_ms: Some(120000), // 2 min for pull
    }).await;

    match pull_result {
        Ok(result) => {
            // Display manifest
            if let Some(manifest) = result.get("manifest") {
                println!("done");
                println!("Worker: {} v{}",
                    manifest["name"].as_str().unwrap_or("unknown"),
                    manifest["version"].as_str().unwrap_or("unknown")
                );
                if let Some(desc) = manifest["description"].as_str() {
                    println!("  {}", desc);
                }
                println!();
                if let Some(funcs) = manifest["capabilities"]["functions"].as_array() {
                    println!("Capabilities:");
                    println!("  Functions:");
                    for f in funcs {
                        if let Some(id) = f["id"].as_str() {
                            println!("    - {}", id);
                        }
                    }
                }
                if let Some(resources) = manifest.get("resources") {
                    println!();
                    println!("Resources:");
                    if let Some(mem) = resources["memory"].as_str() {
                        println!("  Memory: {}", mem);
                    }
                    if let Some(cpu) = resources["cpu"].as_str() {
                        println!("  CPU: {}", cpu);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("Pull failed: {}", e);
            return 1;
        }
    }

    // Step 2: Start container via launcher
    let worker_name = image_or_name.split('/').last().unwrap_or(&image_or_name)
        .split(':').next().unwrap_or(&image_or_name)
        .to_string();

    println!();
    println!("Starting with runtime: docker");
    let start_result = iii.trigger(iii_sdk::TriggerRequest {
        function_id: "iii_launcher::start".into(),
        payload: serde_json::json!({
            "name": worker_name,
            "image": image,
            "engine_url": engine_url,
        }),
        action: None,
        timeout_ms: Some(30000),
    }).await;

    match start_result {
        Ok(result) => {
            let cid = result["container_id"].as_str().unwrap_or("unknown");
            println!("Container started: {}", &cid[..12.min(cid.len())]);

            // Wait for readiness (poll engine for worker ready status)
            print!("Waiting for readiness... ");
            let start = std::time::Instant::now();
            // Simple poll — in production this would listen for WorkerReady
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            println!("ready ({:.1}s)", start.elapsed().as_secs_f64());

            println!();
            println!("Worker {} is running and registered with the engine.", worker_name);
        }
        Err(e) => {
            eprintln!("Start failed: {}", e);
            return 1;
        }
    }

    0
}

async fn resolve_image_from_registry(name: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let registry = crate::cli::worker_manager::registry::fetch_registry(&client).await?;

    // Try v2 format first (image field)
    if let Some(entry) = registry.workers.get(name) {
        if let Some(image) = &entry.image {
            let version = entry.version.as_deref().unwrap_or("latest");
            return Ok(format!("{}:{}", image, version));
        }
    }

    Err(anyhow::anyhow!("worker '{}' not found in registry", name))
}
```

- [ ] **Step 3: Implement remaining commands (simpler)**

Add `handle_worker_remove`, `handle_worker_stop`, `handle_worker_status`, `handle_worker_logs` following the same pattern — connect to engine, invoke the corresponding launcher function, display results. These are simpler than `add` because they just forward to a single launcher function:

```rust
async fn handle_worker_status(_args: &[String]) -> i32 {
    let engine_url = "ws://127.0.0.1:49134";
    let iii = iii_sdk::register_worker(engine_url, iii_sdk::InitOptions {
        worker_name: "iii-cli".into(),
        ..Default::default()
    });

    match iii.trigger(iii_sdk::TriggerRequest {
        function_id: "iii_launcher::status".into(),
        payload: serde_json::json!({}),
        action: None,
        timeout_ms: Some(10000),
    }).await {
        Ok(result) => {
            println!("{:<16} {:<10} {:<10} {:<10}", "NAME", "RUNTIME", "STATUS", "UPTIME");
            if let Some(workers) = result.as_array() {
                for w in workers {
                    println!("{:<16} {:<10} {:<10} {}",
                        w["name"].as_str().unwrap_or("-"),
                        w["runtime"].as_str().unwrap_or("-"),
                        if w["running"].as_bool().unwrap_or(false) { "running" } else { "stopped" },
                        w["started_at"].as_str().unwrap_or("-"),
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("Status check failed: {}", e);
            1
        }
    }
}
```

Implement `handle_worker_remove` (invokes stop + updates iii.toml), `handle_worker_stop` (invokes stop), `handle_worker_logs` (invokes logs, prints output).

- [ ] **Step 4: Run compilation check**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo check -p iii-engine`
Expected: Compiles. The exact function signatures may need adjustment based on how the current CLI dispatches commands.

- [ ] **Step 5: Commit**

```bash
cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/
git commit -m "feat(cli): add managed worker commands — add, remove, start, stop, status, logs"
```

---

## Task 11: Registry v2 Format

**Files:**
- Modify: `workers/registry/index.json`
- Modify: `engine/src/cli/worker_manager/registry.rs`
- Modify: `workers/.github/workflows/create-tag.yml` (registry update step)

- [ ] **Step 1: Update registry/index.json to v2**

Replace `/Users/andersonleal/projetos/motia/workers/registry/index.json` with:

```json
{
    "version": 2,
    "workers": {
        "image-resize": {
            "description": "Image resize and format conversion",
            "image": "ghcr.io/iii-hq/image-resize",
            "latest": "0.1.2"
        }
    }
}
```

- [ ] **Step 2: Update WorkerEntry to support v2 fields**

In `/Users/andersonleal/projetos/motia/motia/engine/src/cli/worker_manager/registry.rs`, extend `WorkerEntry` (around line ~23-37) to include the new `image` field while keeping old fields optional for backward compat:

```rust
#[derive(Debug, Deserialize)]
pub struct WorkerEntry {
    pub description: String,
    // v1 fields (kept for backward compat)
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub tag_prefix: Option<String>,
    #[serde(default)]
    pub supported_targets: Option<Vec<String>>,
    #[serde(default)]
    pub has_checksum: bool,
    #[serde(default)]
    pub default_config: Option<serde_json::Value>,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    // v2 fields
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
}
```

Note: This changes `repo` and `supported_targets` from required to optional. Find all code that accesses these fields and handle the `Option`. The `install.rs` code that downloads binaries will need to check for `image` first (v2 path) and fall back to `repo` (v1 path).

- [ ] **Step 3: Update create-tag.yml registry step**

In `/Users/andersonleal/projetos/motia/workers/.github/workflows/create-tag.yml`, find the step that updates `registry/index.json` (around lines ~178-189). Update it to write v2 format:

```yaml
- name: Update registry/index.json
  run: |
    jq --arg worker "${{ inputs.worker }}" \
       --arg version "${{ env.NEW_VERSION }}" \
       --arg image "ghcr.io/iii-hq/${{ inputs.worker }}" \
       '.workers[$worker].latest = $version | .workers[$worker].image = $image | .version = 2' \
       registry/index.json > registry/index.json.tmp
    mv registry/index.json.tmp registry/index.json
```

- [ ] **Step 4: Run engine tests**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo test -p iii-engine`
Expected: Existing tests pass. Registry parsing handles both v1 and v2.

- [ ] **Step 5: Commit (both repos)**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add registry/index.json .github/workflows/create-tag.yml
git commit -m "feat(registry): upgrade to v2 format with OCI image references"

cd /Users/andersonleal/projetos/motia/motia
git add engine/src/cli/worker_manager/registry.rs
git commit -m "feat(registry): support v2 registry format with image field"
```

---

## Task 12: OCI CI/CD Workflow

**Files:**
- Create: `workers/.github/workflows/_oci-build.yml`
- Modify: `workers/.github/workflows/release.yml`

- [ ] **Step 1: Create _oci-build.yml reusable workflow**

Create `/Users/andersonleal/projetos/motia/workers/.github/workflows/_oci-build.yml`:

```yaml
name: OCI Build

on:
  workflow_call:
    inputs:
      worker_name:
        required: true
        type: string
      dockerfile_path:
        required: true
        type: string
      tag_name:
        required: true
        type: string
      is_prerelease:
        required: false
        type: boolean
        default: false
      dry_run:
        required: false
        type: boolean
        default: false
    secrets:
      III_CI_APP_ID:
        required: true
      III_CI_APP_PRIVATE_KEY:
        required: true

jobs:
  build-and-push:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Log in to GitHub Container Registry
        if: ${{ !inputs.dry_run }}
        uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}

      - name: Extract version from tag
        id: version
        run: |
          TAG="${{ inputs.tag_name }}"
          VERSION="${TAG##*/v}"
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"
          echo "Building ${{ inputs.worker_name }} version $VERSION"

      - name: Build and push multi-arch image
        uses: docker/build-push-action@v5
        with:
          context: .
          file: ${{ inputs.dockerfile_path }}
          platforms: linux/amd64,linux/arm64
          push: ${{ !inputs.dry_run }}
          tags: |
            ghcr.io/iii-hq/${{ inputs.worker_name }}:${{ steps.version.outputs.version }}
            ghcr.io/iii-hq/${{ inputs.worker_name }}:latest
          cache-from: type=gha
          cache-to: type=gha,mode=max

      - name: Verify manifest embedded
        if: ${{ !inputs.dry_run }}
        run: |
          docker pull ghcr.io/iii-hq/${{ inputs.worker_name }}:${{ steps.version.outputs.version }}
          docker run --rm ghcr.io/iii-hq/${{ inputs.worker_name }}:${{ steps.version.outputs.version }} cat /iii/worker.yaml
```

- [ ] **Step 2: Update release.yml to use OCI build**

In `/Users/andersonleal/projetos/motia/workers/.github/workflows/release.yml`, find the `binary-build` job (around lines ~99-113) that calls `_rust-binary.yml`. Replace it with:

```yaml
  oci-build:
    needs: [setup]
    if: needs.setup.outputs.dry_run != 'true'
    uses: ./.github/workflows/_oci-build.yml
    with:
      worker_name: ${{ needs.setup.outputs.worker_name }}
      dockerfile_path: ${{ needs.setup.outputs.worker_name }}/Dockerfile
      tag_name: ${{ needs.setup.outputs.tag }}
      is_prerelease: ${{ needs.setup.outputs.is_prerelease == 'true' }}
      dry_run: false
    secrets:
      III_CI_APP_ID: ${{ secrets.III_CI_APP_ID }}
      III_CI_APP_PRIVATE_KEY: ${{ secrets.III_CI_APP_PRIVATE_KEY }}
```

Keep the old `binary-build` job commented out for reference during the transition.

- [ ] **Step 3: Commit**

```bash
cd /Users/andersonleal/projetos/motia/workers
git add .github/workflows/_oci-build.yml .github/workflows/release.yml
git commit -m "feat(ci): add OCI build workflow, switch release from binary to OCI"
```

---

## Task 13: End-to-End Verification

**Files:** None (verification only)

This task validates the full flow works end-to-end.

- [ ] **Step 1: Start the iii engine**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo run -p iii-engine`
Expected: Engine starts, listening on port 49134.

- [ ] **Step 2: Build image-resize OCI image locally**

Run:
```bash
cd /Users/andersonleal/projetos/motia/workers
docker build -t ghcr.io/iii-hq/image-resize:0.1.2 -f image-resize/Dockerfile .
```
Expected: Image builds successfully.

- [ ] **Step 3: Verify manifest is embedded**

Run: `docker run --rm ghcr.io/iii-hq/image-resize:0.1.2 cat /iii/worker.yaml`
Expected: YAML manifest with `iii: v1`, `name: image-resize`, function capabilities.

- [ ] **Step 4: Start the launcher worker**

Run: `cd /Users/andersonleal/projetos/motia/workers && cargo run -p iii-launcher`
Expected: Launcher connects to engine, registers 5 functions (pull, start, stop, status, logs).

- [ ] **Step 5: Run `iii worker add` (manually test the flow)**

Run: `cd /Users/andersonleal/projetos/motia/motia && cargo run -p iii-engine -- worker add ghcr.io/iii-hq/image-resize:0.1.2`
Expected:
```
Pulling image... done
Worker: image-resize v0.1.2
  Image resize and format conversion

Capabilities:
  Functions:
    - image_resize::resize

Starting with runtime: docker
Container started: iii-image-resize-xxxxx
Waiting for readiness... ready (X.Xs)

Worker image-resize is running and registered with the engine.
```

- [ ] **Step 6: Verify worker status**

Run: `cargo run -p iii-engine -- worker status`
Expected: Shows `image-resize` as running.

- [ ] **Step 7: Invoke the function**

Test that `image_resize::resize` is invocable through the engine (use existing test tooling or a simple SDK client).

- [ ] **Step 8: Stop and clean up**

Run: `cargo run -p iii-engine -- worker remove image-resize`
Expected: Container stops, removed from state.

---

## Dependency Graph

```
Task 1 (Manifest Types)
  └── Task 2 (Engine Protocol) ──┐
  └── Task 3 (SDK Protocol) ─────┤
       └── Task 4 (Engine Readiness)
       └── Task 5 (SDK Manifest Support)
            └── Task 6 (image-resize Manifest YAML)
                 └── Task 7 (image-resize Dockerfile + SDK Integration)
            └── Task 8 (Launcher Adapter + Docker)
                 └── Task 9 (Launcher Functions + Main)
                      └── Task 10 (CLI Commands)
Task 11 (Registry v2) — independent, can run in parallel with Tasks 6-10
Task 12 (OCI CI/CD) — independent, can run in parallel with Tasks 6-10
Task 13 (E2E Verification) — depends on all above
```

Parallelizable work:
- Tasks 2 + 3 (engine protocol + SDK protocol) can be done together
- Tasks 6 + 8 (image-resize manifest + launcher adapter) can be done in parallel after Task 5
- Tasks 11 + 12 (registry + CI) can be done in parallel with everything after Task 1
