// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! A local worker VM through `worker::*`: it boots, syncs its source,
//! restarts as the next instance and is destroyed. Needs KVM.

#![cfg(feature = "integration-vm")]

use std::path::Path;
use std::time::{Duration, Instant};

use sandbox_builtin::wire::{
    State, WorkerDestroyRequest, WorkerPrepareRequest, WorkerSource, WorkerStartRequest,
    WorkerStatusRequest, WorkerStopRequest,
};
use sandbox_builtin::worker_vm::WorkerVms;

const RUN: &str =
    "sh -c 'echo worker-vm-ready; while sleep 1; do cat /workspace/synced.txt 2>/dev/null; done'";

#[test]
fn a_local_worker_vm_boots_restarts_and_is_destroyed() {
    let kvm = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm");
    if let Err(e) = kvm {
        eprintln!("[skip] worker_vm_e2e: /dev/kvm is not read-write: {e}");
        return;
    }
    // SAFETY: set before the runtime starts; this binary's only test.
    unsafe {
        std::env::set_var(
            "III_TEST_VM_BOOT_BIN",
            env!("CARGO_BIN_EXE_sandbox-builtin"),
        )
    };
    tokio::runtime::Runtime::new().unwrap().block_on(cycle());
}

async fn cycle() {
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let src = t.join("w");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(
        src.join("iii.worker.yaml"),
        "name: e2e\nruntime:\n  base_image: docker.io/iiidev/python:latest\nscripts:\n  install: ''\n",
    )
    .unwrap();
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let source = WorkerSource::Local {
        host_path: src.display().to_string(),
        run_override: Some(RUN.into()),
    };
    let start = || WorkerStartRequest {
        auth: "t".into(),
        state_key: "e2e".into(),
        worker_name: "e2e".into(),
        source: source.clone(),
        env: Default::default(),
        engine_url: "ws://127.0.0.1:49134".into(),
    };
    let status = |vm_id: &str, instance| {
        vms.status(WorkerStatusRequest {
            auth: "t".into(),
            vm_id: vm_id.into(),
            instance,
        })
        .unwrap()
    };
    let stop = |vm_id: &str, instance| {
        vms.stop(WorkerStopRequest {
            auth: "t".into(),
            vm_id: vm_id.into(),
            instance,
            grace_ms: 5000,
        })
    };

    vms.prepare(WorkerPrepareRequest {
        auth: "t".into(),
        source: source.clone(),
    })
    .await
    .unwrap();

    let first = vms.start(start()).await.unwrap();
    assert_eq!(first.instance, 1);
    assert!(matches!(status(&first.vm_id, 1).state, State::Running));
    let console = t.join("workers").join(&first.vm_id).join("console.log");
    wait_for(&console, "worker-vm-ready", 120).await;
    std::fs::write(src.join("synced.txt"), "sync-ok").unwrap();
    wait_for(&console, "sync-ok", 30).await;
    let stopped = stop(&first.vm_id, 1).await.unwrap();
    assert!(matches!(stopped.state, State::Stopped), "{stopped:?}");

    let second = vms.start(start()).await.unwrap();
    assert_eq!(
        (second.vm_id.as_str(), second.instance),
        (first.vm_id.as_str(), 2)
    );
    assert!(matches!(status(&second.vm_id, 2).state, State::Running));
    stop(&second.vm_id, 2).await.unwrap();

    let destroyed = vms
        .destroy(WorkerDestroyRequest {
            auth: "t".into(),
            state_key: "e2e".into(),
        })
        .unwrap();
    assert!(destroyed.destroyed);
}

async fn wait_for(console: &Path, needle: &str, secs: u64) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if std::fs::read_to_string(console).is_ok_and(|log| log.contains(needle)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let log = std::fs::read_to_string(console).unwrap_or_default();
    panic!(
        "no {needle:?} in {} within {secs} s:\n{log}",
        console.display()
    );
}
