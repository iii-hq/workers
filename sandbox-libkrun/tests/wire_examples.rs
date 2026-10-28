// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Pins the adapter's copies of the contract's types to the contract's own wire
//! fixtures: every info and worker-VM example must survive a deserialize and
//! re-serialize unchanged, every `sandbox::*` request and response must have
//! the contract's schema, and every reply must carry the fields the contract
//! requires, so a field, rename, tag, default or optional flag that drifts
//! fails here.

use iii_sdk::channels::StreamChannelRef;
use iii_shell_proto::{FsEntry, FsMatch, FsSedFileResult};
use sandbox_libkrun::sandbox_daemon::{catalog, create, exec, fs, list, run, stop};
use sandbox_libkrun::wire::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/wire_examples.json");
const OPERATION_SCHEMAS: &str = include_str!("fixtures/operation_schemas.json");

/// Every type the adapter copies; `RouterInfo` is answered only by the router.
const COPIED: &[&str] = &[
    "ContractVersion",
    "AdapterInfoRequest",
    "AdapterInfo",
    "WorkerPrepareRequest",
    "WorkerPrepareResponse",
    "WorkerStartRequest",
    "WorkerSource",
    "WorkerStartResponse",
    "WorkerStatusRequest",
    "WorkerStatus",
    "State",
    "WorkerLogsRequest",
    "WorkerLogsResponse",
    "LogRecord",
    "LogStream",
    "WorkerStopRequest",
    "WorkerDestroyRequest",
    "WorkerDestroyResponse",
];

fn round_trips<T: Serialize + DeserializeOwned>(name: &str, example: &Value) {
    let typed: T = serde_json::from_value(example.clone())
        .unwrap_or_else(|e| panic!("{name}: example does not deserialize: {e}"));
    assert_eq!(&serde_json::to_value(typed).unwrap(), example, "{name}");
}

#[test]
fn wire_examples_round_trip() {
    let fixture: serde_json::Map<String, Value> = serde_json::from_str(FIXTURE).unwrap();
    let mut seen = Vec::new();
    for (name, example) in &fixture {
        match name.as_str() {
            "RouterInfo" => continue,
            "ContractVersion" => round_trips::<ContractVersion>(name, example),
            "AdapterInfoRequest" => round_trips::<AdapterInfoRequest>(name, example),
            "AdapterInfo" => round_trips::<AdapterInfo>(name, example),
            "WorkerPrepareRequest" => round_trips::<WorkerPrepareRequest>(name, example),
            "WorkerPrepareResponse" => round_trips::<WorkerPrepareResponse>(name, example),
            "WorkerStartRequest" => round_trips::<WorkerStartRequest>(name, example),
            "WorkerSource" => round_trips::<WorkerSource>(name, example),
            "WorkerStartResponse" => round_trips::<WorkerStartResponse>(name, example),
            "WorkerStatusRequest" => round_trips::<WorkerStatusRequest>(name, example),
            "WorkerStatus" => round_trips::<WorkerStatus>(name, example),
            "State" => round_trips::<State>(name, example),
            "WorkerLogsRequest" => round_trips::<WorkerLogsRequest>(name, example),
            "WorkerLogsResponse" => round_trips::<WorkerLogsResponse>(name, example),
            "LogRecord" => round_trips::<LogRecord>(name, example),
            "LogStream" => round_trips::<LogStream>(name, example),
            "WorkerStopRequest" => round_trips::<WorkerStopRequest>(name, example),
            "WorkerDestroyRequest" => round_trips::<WorkerDestroyRequest>(name, example),
            "WorkerDestroyResponse" => round_trips::<WorkerDestroyResponse>(name, example),
            other => panic!("fixture type {other} has no copy in sandbox_libkrun::wire"),
        }
        seen.push(name.as_str());
    }
    for name in COPIED {
        assert!(seen.contains(name), "fixture has no example for {name}");
    }
}

/// As `wire_shape` in the contract's `tests/wire_parity.rs`: drops prose, the
/// `[]`/`null` defaults schemars only emits for `Serialize` field types (this
/// adapter's request types only deserialize) and a response's `default`. It also
/// drops a response's `required`, which schemars derives from `#[serde(default)]`
/// alone; `replies_carry_required_fields` checks the contract's list instead.
fn wire_shape(v: &mut Value, response: bool) {
    match v {
        Value::Array(a) => a.iter_mut().for_each(|v| wire_shape(v, response)),
        Value::Object(m) => {
            m.remove("description");
            if response
                || m.get("default")
                    .is_some_and(|d| d.is_null() || d == &json!([]))
            {
                m.remove("default");
            }
            if response {
                m.remove("required");
            }
            for (k, v) in m.iter_mut() {
                // These two maps are keyed by field and type names, not schema keywords.
                if k == "properties" || k == "definitions" {
                    v.as_object_mut()
                        .into_iter()
                        .flat_map(|m| m.values_mut())
                        .for_each(|v| wire_shape(v, response));
                } else {
                    wire_shape(v, response);
                }
            }
        }
        _ => {}
    }
}

fn schema<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap()
}

/// `[(suffix, request schema, response schema)]`, one per `sandbox::*` handler.
macro_rules! ops {
    ($($suffix:literal => $req:ty, $res:ty;)+) => {
        [$(($suffix, schema::<$req>(), schema::<$res>())),+]
    };
}

#[test]
fn operation_schemas_match_the_contract() {
    let fixture: serde_json::Map<String, Value> = serde_json::from_str(OPERATION_SCHEMAS).unwrap();
    // The router publishes the contract's schema under `sandbox::<suffix>`.
    let ops = ops! {
        "create" => create::CreateRequest, create::CreateResponse;
        "exec" => exec::ExecRequest, exec::ExecResponse;
        "stop" => stop::StopRequest, stop::StopResponse;
        "list" => list::ListRequest, list::ListResponse;
        "catalog::list" => catalog::CatalogListRequest, catalog::CatalogListResponse;
        "run" => run::RunRequest, run::RunResponse;
        "fs::ls" => fs::ls::LsRequest, fs::ls::LsResponse;
        "fs::stat" => fs::stat::StatRequest, fs::stat::StatResponse;
        "fs::mkdir" => fs::mkdir::MkdirRequest, fs::mkdir::MkdirResponse;
        "fs::rm" => fs::rm::RmRequest, fs::rm::RmResponse;
        "fs::chmod" => fs::chmod::ChmodRequest, fs::chmod::ChmodResponse;
        "fs::mv" => fs::mv::MvRequest, fs::mv::MvResponse;
        "fs::grep" => fs::grep::GrepRequest, fs::grep::GrepResponse;
        "fs::sed" => fs::sed::SedRequest, fs::sed::SedResponse;
        "fs::write" => fs::write::WriteRequest, fs::write::WriteResponse;
        "fs::read" => fs::read::ReadRequest, fs::read::ReadResponse;
    };
    let mut suffixes: Vec<&str> = ops.iter().map(|(s, ..)| *s).collect();
    suffixes.sort();
    assert_eq!(
        suffixes,
        fixture.keys().map(String::as_str).collect::<Vec<_>>()
    );
    for (suffix, mut req, mut res) in ops {
        let mut want = fixture[suffix]["response"].clone();
        wire_shape(&mut req, false);
        wire_shape(&mut res, true);
        wire_shape(&mut want, true);
        assert_eq!(req, fixture[suffix]["request"], "{suffix} request");
        assert_eq!(res, want, "{suffix} response");
    }
}

/// Fails if `value`, at any depth, lacks a field `schema` requires.
fn assert_required_present(defs: &Value, schema: &Value, value: &Value, at: &str) {
    if let Some(name) = schema["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/definitions/"))
    {
        return assert_required_present(defs, &defs[name], value, at);
    }
    // `allOf` wraps a nested struct, `anyOf` is `Option<T>`'s `[T, null]`.
    for sub in ["allOf", "anyOf"]
        .into_iter()
        .filter_map(|k| schema[k].as_array())
        .flatten()
    {
        assert_required_present(defs, sub, value, at);
    }
    match value {
        Value::Object(fields) => {
            for name in schema["required"].as_array().into_iter().flatten() {
                let name = name.as_str().unwrap();
                assert!(
                    fields.contains_key(name),
                    "{at} omits `{name}`, which the contract requires"
                );
            }
            for (name, v) in fields {
                assert_required_present(
                    defs,
                    &schema["properties"][name],
                    v,
                    &format!("{at}.{name}"),
                );
            }
        }
        Value::Array(items) => items
            .iter()
            .for_each(|v| assert_required_present(defs, &schema["items"], v, at)),
        _ => {}
    }
}

fn value(reply: impl Serialize) -> Value {
    serde_json::to_value(reply).unwrap()
}

/// schemars ignores `skip_serializing_if`, so the schema comparison cannot see a
/// reply that leaves a field out. Serialize one reply per operation with every field
/// at the zero value such predicates skip, and one element per list so nested types
/// count too, and require every field the contract needs to read it.
#[test]
fn replies_carry_required_fields() {
    let fixture: serde_json::Map<String, Value> = serde_json::from_str(OPERATION_SCHEMAS).unwrap();
    let entry = || FsEntry {
        name: String::new(),
        is_dir: false,
        size: 0,
        mode: String::new(),
        mtime: 0,
        is_symlink: false,
    };
    for (suffix, op) in &fixture {
        let reply = match suffix.as_str() {
            "create" => value(create::CreateResponse {
                sandbox_id: String::new(),
                image: String::new(),
            }),
            "exec" => value(exec::ExecResponse {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: false,
                duration_ms: 0,
                success: false,
                stdout_truncated: false,
                stderr_truncated: false,
            }),
            "stop" => value(stop::StopResponse {
                sandbox_id: String::new(),
                stopped: false,
            }),
            "list" => value(list::ListResponse {
                sandboxes: vec![list::SandboxSummary {
                    sandbox_id: String::new(),
                    name: None,
                    image: String::new(),
                    age_secs: 0,
                    exec_in_progress: false,
                    exec_in_flight: 0,
                    exec_slots_free: 0,
                    stopped: false,
                }],
            }),
            "catalog::list" => value(catalog::CatalogListResponse {
                images: vec![catalog::CatalogEntry {
                    name: String::new(),
                    oci_ref: String::new(),
                    kind: catalog::CatalogEntryKind::Preset,
                }],
            }),
            "run" => value(run::RunResponse {
                stdout: String::new(),
                stderr: String::new(),
                exit_code: None,
                timed_out: false,
                duration_ms: 0,
                success: false,
                stdout_truncated: false,
                stderr_truncated: false,
                sandbox_id: None,
            }),
            "fs::ls" => value(fs::ls::LsResponse {
                entries: vec![entry()],
            }),
            "fs::stat" => value(fs::stat::StatResponse::from(entry())),
            "fs::mkdir" => value(fs::mkdir::MkdirResponse { created: false }),
            "fs::rm" => value(fs::rm::RmResponse { removed: false }),
            "fs::chmod" => value(fs::chmod::ChmodResponse { updated: 0 }),
            "fs::mv" => value(fs::mv::MvResponse { moved: false }),
            "fs::grep" => value(fs::grep::GrepResponse {
                matches: vec![FsMatch {
                    path: String::new(),
                    line: 0,
                    content: String::new(),
                }],
                truncated: false,
            }),
            "fs::sed" => value(fs::sed::SedResponse {
                results: vec![FsSedFileResult {
                    path: String::new(),
                    replacements: 0,
                    success: false,
                    error: None,
                }],
                total_replacements: 0,
            }),
            "fs::write" => value(fs::write::WriteResponse {
                bytes_written: 0,
                path: String::new(),
            }),
            "fs::read" => value(fs::read::ReadResponse {
                content: StreamChannelRef::default(),
                body: None,
                size: 0,
                mode: String::new(),
                mtime: 0,
            }),
            other => panic!("no zero-valued reply for sandbox::{other}"),
        };
        let schema = &op["response"];
        assert_required_present(&schema["definitions"], schema, &reply, suffix);
    }
}

#[test]
fn constants_match_the_contract() {
    let fixture: serde_json::Map<String, Value> = serde_json::from_str(FIXTURE).unwrap();
    let version = serde_json::to_value(ContractVersion::CURRENT).unwrap();
    assert_eq!(version, fixture["ContractVersion"]);
    assert_eq!((CONTRACT_MAJOR, CONTRACT_MINOR), (1, 1));
    assert_eq!(
        [
            WORKER_PREPARE,
            WORKER_START,
            WORKER_STATUS,
            WORKER_LOGS,
            WORKER_STOP,
            WORKER_DESTROY
        ],
        [
            "worker::prepare",
            "worker::start",
            "worker::status",
            "worker::logs",
            "worker::stop",
            "worker::destroy"
        ]
    );
}
