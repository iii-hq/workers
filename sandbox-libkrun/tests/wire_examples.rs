// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Pins the adapter's copies of the contract's types to the contract's own wire
//! fixtures: every info and worker-VM example must survive a deserialize and
//! re-serialize unchanged, and every `sandbox::*` request and response must have
//! the contract's schema, so a field, rename, tag, default or optional flag that
//! drifts fails here.

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

/// Same as `wire_shape` in the contract's `tests/wire_parity.rs`: drops prose, the
/// `[]`/`null` defaults schemars only emits for `Serialize` field types (this
/// adapter's request types only deserialize), and a response's `required`/`default`
/// (only the adapter writes responses; the contract reads 1.0 replies leniently).
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
        wire_shape(&mut req, false);
        wire_shape(&mut res, true);
        assert_eq!(req, fixture[suffix]["request"], "{suffix} request");
        assert_eq!(res, fixture[suffix]["response"], "{suffix} response");
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
