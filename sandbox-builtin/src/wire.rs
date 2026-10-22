// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! The adapter's own copies of the sandbox contract's info and worker-VM types
//! (`iii-sandbox-contract`, `info.rs` and `worker.rs`). They are copies, not a
//! dependency; `tests/wire_examples.rs` pins their JSON to the contract's
//! `wire_examples.json`. `RouterInfo` is not copied: only the router answers it.

use iii_sdk::channels::StreamChannelRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CONTRACT_MAJOR: u32 = 1;
pub const CONTRACT_MINOR: u32 = 0;

/// Worker-VM function suffixes, called as `<adapter>::<suffix>`.
pub const WORKER_PREPARE: &str = "worker::prepare";
pub const WORKER_START: &str = "worker::start";
pub const WORKER_STATUS: &str = "worker::status";
pub const WORKER_LOGS: &str = "worker::logs";
pub const WORKER_STOP: &str = "worker::stop";
pub const WORKER_DESTROY: &str = "worker::destroy";

/// Contract version. A different major is incompatible (S302); a minor only adds fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContractVersion {
    pub major: u32,
    pub minor: u32,
}

impl ContractVersion {
    pub const CURRENT: Self = Self {
        major: CONTRACT_MAJOR,
        minor: CONTRACT_MINOR,
    };
}

/// Request of `<adapter>::adapter::info`; carries no fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdapterInfoRequest {}

/// Reply of `<adapter>::adapter::info`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdapterInfo {
    /// The adapter's name, e.g. `"sandbox-builtin"`.
    pub adapter: String,
    pub adapter_version: String,
    /// The contract version the adapter implements.
    pub contract: ContractVersion,
}

/// Where the worker's code comes from.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerSource {
    Bundle {
        /// Manifest name, not the container key.
        package: String,
        /// The bundle in Compose's package cache.
        host_path: String,
    },
    /// Local sources always sync into the VM.
    Local {
        host_path: String,
        run_override: Option<String>,
    },
}

/// Idempotent image and base-layer pull; long-running.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerPrepareRequest {
    /// Per-project token.
    pub auth: String,
    pub source: WorkerSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerPrepareResponse {}

/// Boots the VM. The adapter reads resources from the worker's manifest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStartRequest {
    /// Per-project token.
    pub auth: String,
    /// Hash of the compose state dir and container key; the same key gets the same persistent upper.
    pub state_key: String,
    /// Container key; becomes `III_WORKER_NAME`.
    pub worker_name: String,
    pub source: WorkerSource,
    /// Complete guest env; carried in the body only, never argv or the kernel cmdline.
    pub env: BTreeMap<String, String>,
    /// The engine's URL as seen from the host.
    pub engine_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStartResponse {
    /// Opaque; stable per `state_key`.
    pub vm_id: String,
    /// Increments on every boot of this `vm_id`.
    pub instance: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStatusRequest {
    pub auth: String,
    pub vm_id: String,
    pub instance: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Preparing,
    Running,
    Exited,
    Stopped,
    Unknown,
}

/// Reply of `worker::status` and `worker::stop`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStatus {
    pub vm_id: String,
    pub instance: u64,
    pub state: State,
    pub exit_code: Option<i32>,
    /// The signal that ended the VM, if one did.
    pub signal: Option<i32>,
    pub started_at_ms: u64,
    pub ended_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerLogsRequest {
    pub auth: String,
    pub vm_id: String,
    /// `LogRecord.cursor` of the last record the caller has; reading resumes right after it.
    /// Opaque to Compose. A cursor past the end of the log (after a rotation) reads from the start.
    pub cursor: Option<u64>,
    /// Keep the channel open while the instance runs.
    pub follow: bool,
    /// Writer end of a channel Compose created; one `LogRecord` JSON per text frame.
    pub channel: StreamChannelRef,
}

/// Returned once the log pump starts; the records travel over the channel.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerLogsResponse {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LogRecord {
    pub stream: LogStream,
    pub line: String,
    pub ts_ms: u64,
    /// Resume point: pass it as `WorkerLogsRequest.cursor` to continue after this record.
    pub cursor: u64,
}

/// Idempotent; `grace_ms: 0` kills. Replies with the final `WorkerStatus`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerStopRequest {
    pub auth: String,
    pub vm_id: String,
    /// Names the boot to stop, so a stale stop cannot hit a newer one.
    pub instance: u64,
    pub grace_ms: u64,
}

/// Refused while the VM is running.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerDestroyRequest {
    pub auth: String,
    pub state_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkerDestroyResponse {
    pub destroyed: bool,
}
