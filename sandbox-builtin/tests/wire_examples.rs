// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! Pins the adapter's copies of the contract's info and worker-VM types to the
//! contract's own wire fixture: every example must survive a deserialize and
//! re-serialize unchanged, so a field, rename or tag that drifts fails here.

use sandbox_builtin::wire::*;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/wire_examples.json");

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
            other => panic!("fixture type {other} has no copy in sandbox_builtin::wire"),
        }
        seen.push(name.as_str());
    }
    for name in COPIED {
        assert!(seen.contains(name), "fixture has no example for {name}");
    }
}

#[test]
fn constants_match_the_contract() {
    let fixture: serde_json::Map<String, Value> = serde_json::from_str(FIXTURE).unwrap();
    let version = serde_json::to_value(ContractVersion::CURRENT).unwrap();
    assert_eq!(version, fixture["ContractVersion"]);
    assert_eq!((CONTRACT_MAJOR, CONTRACT_MINOR), (1, 0));
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
