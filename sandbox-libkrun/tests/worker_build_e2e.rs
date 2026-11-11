// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! `worker::build` and `worker::start` with `build` on real VMs: an image is
//! built from a local worker and from a bundle and booted without an install,
//! the S-codes of a build, a build cancelled by its reader, the data disk across
//! restarts and rebuilds, and the disks an earlier adapter left. Needs KVM and
//! the cached `iiidev/python` and `iiidev/node` base images (pulled otherwise).

#![cfg(feature = "integration-vm")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use iii_sdk::IIIClient;
use iii_sdk::channels::{ChannelDirection, StreamChannelRef};
use sandbox_libkrun::sandbox_daemon::SandboxError;
use sandbox_libkrun::wire::{
    LogRecord, WorkerBuildRef, WorkerBuildRequest, WorkerBuildResponse, WorkerDestroyRequest,
    WorkerSource, WorkerStartRequest, WorkerStartResponse, WorkerStopRequest,
};
use sandbox_libkrun::worker_vm::{WorkerVms, vm_id};
use serial_test::serial;
use sha2::{Digest, Sha256};
use tokio::sync::{Notify, mpsc};
use tokio_tungstenite::tungstenite::Message;

const PYTHON: &str = "docker.io/iiidev/python:latest";

/// False, with a note, when this host cannot boot a VM.
fn kvm() -> bool {
    let kvm = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm");
    if let Err(e) = kvm {
        eprintln!("[skip] worker_build_e2e: /dev/kvm is not read-write: {e}");
        return false;
    }
    // SAFETY: the tests of this binary run one at a time (#[serial]).
    unsafe {
        std::env::set_var(
            "III_TEST_VM_BOOT_BIN",
            env!("CARGO_BIN_EXE_sandbox-libkrun"),
        )
    };
    true
}

/// A stand-in for the engine's channel endpoint: it passes on every frame any
/// writer sends, and `leave` closes the writers connected now, as the engine
/// does once their reader has left.
async fn channel_engine() -> (IIIClient, mpsc::UnboundedReceiver<Message>, Arc<Notify>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let iii = IIIClient::new(&format!("ws://{}", listener.local_addr().unwrap()));
    let (frames, received) = mpsc::unbounded_channel();
    let leave = Arc::new(Notify::new());
    let left = leave.clone();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let frames = frames.clone();
            let left = left.clone();
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                loop {
                    tokio::select! {
                        frame = ws.next() => match frame {
                            Some(Ok(frame)) => { let _ = frames.send(frame); }
                            _ => break,
                        },
                        () = left.notified() => {
                            let _ = ws.close(None).await;
                            break;
                        }
                    }
                }
            });
        }
    });
    (iii, received, leave)
}

/// Text frames as records, binary frames as bytes, of what arrived so far.
fn drain(frames: &mut mpsc::UnboundedReceiver<Message>) -> (Vec<LogRecord>, Vec<Vec<u8>>) {
    let (mut records, mut binary) = (Vec::new(), Vec::new());
    while let Ok(frame) = frames.try_recv() {
        match frame {
            Message::Text(text) => records.push(
                serde_json::from_str(&text)
                    .unwrap_or_else(|e| panic!("a text frame is no LogRecord ({e}): {text}")),
            ),
            Message::Binary(bytes) => binary.push(bytes.to_vec()),
            _ => {}
        }
    }
    (records, binary)
}

/// Only keepalives (one 0 byte) and the empty frame that opens or closes a
/// stream travel as binary: no record ever does.
fn assert_binary_frames_are_keepalives(binary: &[Vec<u8>]) {
    for frame in binary {
        assert!(
            frame.is_empty() || frame == &[0],
            "a binary frame is not a keepalive: {frame:?}"
        );
    }
}

fn lines(records: &[LogRecord]) -> String {
    records
        .iter()
        .map(|r| r.line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn logs() -> StreamChannelRef {
    StreamChannelRef {
        channel_id: uuid::Uuid::new_v4().to_string(),
        access_key: "k".to_string(),
        direction: ChannelDirection::Write,
    }
}

fn local(dir: &Path) -> WorkerSource {
    WorkerSource::Local {
        host_path: dir.display().to_string(),
        run_override: None,
    }
}

fn build_req(state_key: &str, source: WorkerSource, out: &Path) -> WorkerBuildRequest {
    WorkerBuildRequest {
        auth: "t".into(),
        state_key: state_key.into(),
        worker_name: "w".into(),
        source,
        output_dir: out.display().to_string(),
        env: BTreeMap::from([("BUILD_ENV".to_string(), "from-guest-env".to_string())]),
        logs: logs(),
    }
}

async fn build(
    vms: &WorkerVms,
    iii: &IIIClient,
    state_key: &str,
    source: WorkerSource,
    out: &Path,
) -> Result<WorkerBuildResponse, SandboxError> {
    let started = Instant::now();
    let built = vms.build(iii, build_req(state_key, source, out)).await;
    eprintln!(
        "[build] {state_key}: {:?} in {:?}",
        built.as_ref().map(|b| &b.build_id),
        started.elapsed()
    );
    built
}

async fn start(
    vms: &WorkerVms,
    state_key: &str,
    source: WorkerSource,
    build: Option<(&str, &Path)>,
) -> Result<WorkerStartResponse, SandboxError> {
    vms.start(WorkerStartRequest {
        auth: "t".into(),
        state_key: state_key.into(),
        worker_name: "w".into(),
        source,
        env: Default::default(),
        engine_url: "ws://127.0.0.1:49134".into(),
        build: build.map(|(build_id, dir)| WorkerBuildRef {
            build_id: build_id.into(),
            build_dir: dir.display().to_string(),
        }),
    })
    .await
}

async fn stop(vms: &WorkerVms, started: &WorkerStartResponse) {
    vms.stop(WorkerStopRequest {
        auth: "t".into(),
        vm_id: started.vm_id.clone(),
        instance: started.instance,
        grace_ms: 3000,
    })
    .await
    .unwrap();
}

fn destroy(vms: &WorkerVms, state_key: &str) {
    let destroyed = vms
        .destroy(WorkerDestroyRequest {
            auth: "t".into(),
            state_key: state_key.into(),
        })
        .unwrap();
    assert!(destroyed.destroyed);
}

/// The text of the instance's console log once it holds `needle`.
async fn wait_for(console: &Path, needle: &str, secs: u64) -> String {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let text = std::fs::read_to_string(console).unwrap_or_default();
        let lines: Vec<String> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .map(|r| r["line"].as_str().unwrap_or_default().to_string())
            .collect();
        // What `__vm-boot` writes is plain text, whatever colour the env asks
        // for: its prefix before `init_or_kernel: `, or the whole of a line of
        // its own. The guest's text after the prefix is the guest's to colour.
        if let Some(line) = lines.iter().find(|l| vm_boot_prefix(l).contains('\u{1b}')) {
            panic!("ANSI escapes in {}: {line:?}", console.display());
        }
        let log: String = lines
            .iter()
            .map(|line| format!("{}\n", guest_line(line)))
            .collect();
        if log.contains(needle) {
            return log;
        }
        assert!(
            Instant::now() < deadline,
            "no {needle:?} in {} within {secs} s:\n{log}",
            console.display()
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// `__vm-boot` logs what the guest prints as `<time> ERROR init_or_kernel: <line>`.
fn guest_line(line: &str) -> &str {
    line.split_once("init_or_kernel: ")
        .map_or(line, |(_, guest)| guest)
}

/// What `__vm-boot` wrote of a console line: the prefix of a guest line, or
/// the whole of a line of its own.
fn vm_boot_prefix(line: &str) -> &str {
    line.split_once("init_or_kernel: ")
        .map_or(line, |(prefix, _)| prefix)
}

fn console(t: &Path, state_key: &str) -> PathBuf {
    t.join("workers").join(vm_id(state_key)).join("console.log")
}

/// Streamed: an image is hundreds of MB.
fn sha256(path: &Path) -> String {
    use std::io::Read;
    let mut file = std::fs::File::open(path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A local worker on the python image whose manifest runs `install` and starts
/// `sh run.sh`.
fn python_worker(dir: &Path, install: &str, run: &str) {
    write(
        &dir.join("iii.worker.yaml"),
        &format!(
            "name: e2e\nruntime:\n  base_image: {PYTHON}\nscripts:\n  install: {}\n  start: sh run.sh\n",
            serde_json::to_string(install).unwrap()
        ),
    );
    write(&dir.join("run.sh"), run);
}

/// Acceptance 1, 2 and 6, for `path://.`: the worker's dir is the project, so
/// `.build/`, `.workers/` and `.iii/` are in it.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_local_worker_builds_an_image_and_boots_it_without_install() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "echo INSTALL-RAN env=$BUILD_ENV; mkdir -p /opt/deps node_modules/dep && echo dep-v1 > /opt/deps/version && echo in-image > node_modules/dep/index.js",
        r#"echo "VERSION=$(cat /opt/deps/version)"
echo "DEPS=$(ls /workspace/node_modules | tr '\n' ' ')"
echo "TOP=$(ls -A /workspace | tr '\n' ' ')"
echo READY
while sleep 1; do
  [ -f /workspace/late.txt ] && echo "LATE=$(cat /workspace/late.txt)"
  [ -e /workspace/.build/late ] && echo LEAK-BUILD
  [ -e /workspace/.workers/late ] && echo LEAK-WORKERS
  [ -e /workspace/node_modules/late ] && echo LEAK-DEPS
done
"#,
    );
    write(&project.join("node_modules/hostdep/index.js"), "host");
    write(&project.join(".workers/pkg/index.js"), "package");
    write(&project.join(".iii/token"), "secret");
    let out = project.join(".build/e2e");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, mut frames, _) = channel_engine().await;

    let built = build(&vms, &iii, "local", local(&project), &out)
        .await
        .unwrap();
    eprintln!("[build] response: {built:?}");
    eprintln!("[build] {}: {:?}", out.display(), names(&out));
    assert_eq!(names(&out), ["image.erofs", "image.id"]);
    let id = std::fs::read_to_string(out.join("image.id")).unwrap();
    assert_eq!(id.trim(), built.build_id);
    assert_eq!(sha256(&out.join("image.erofs")), built.build_id);
    assert_eq!(built.base_image.reference, PYTHON);
    let (records, binary) = drain(&mut frames);
    let log = lines(&records);
    assert!(log.contains("INSTALL-RAN env=from-guest-env"), "{log}");
    assert!(log.contains("iii-init: exported"), "{log}");
    assert_binary_frames_are_keepalives(&binary);
    // Nothing of the build is left in the VM's dir.
    let vm_dir = t.join("workers").join(vm_id("local"));
    assert!(
        names(&vm_dir).iter().all(|n| !n.starts_with("build-")),
        "{:?}",
        names(&vm_dir)
    );

    let started = start(
        &vms,
        "local",
        local(&project),
        Some((&built.build_id, &out)),
    )
    .await
    .unwrap();
    let console = console(t, "local");
    let log = wait_for(&console, "READY", 120).await;
    eprintln!("[run] {log}");
    assert!(log.contains("VERSION=dep-v1"), "{log}");
    assert!(!log.contains("INSTALL-RAN"), "the run installed: {log}");
    // The image's deps, not the host's; the code, without Compose's dirs.
    assert!(log.contains("DEPS=dep "), "{log}");
    let top = log.lines().find(|l| l.starts_with("TOP=")).unwrap();
    assert!(
        top.contains("run.sh") && top.contains("iii.worker.yaml"),
        "{top}"
    );
    for hidden in [".build", ".workers", ".iii"] {
        assert!(!top.split(' ').any(|n| n == hidden), "{hidden} in {top}");
    }

    // Code syncs live; Compose's dirs and dependency dirs do not.
    write(&project.join(".build/late"), "x");
    write(&project.join(".workers/late"), "x");
    write(&project.join("node_modules/late"), "x");
    write(&project.join("late.txt"), "synced-ok\n");
    wait_for(&console, "LATE=synced-ok", 30).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let log = wait_for(&console, "LATE=synced-ok", 1).await;
    for leak in ["LEAK-BUILD", "LEAK-WORKERS", "LEAK-DEPS"] {
        assert!(!log.contains(leak), "{leak}: {log}");
    }
    stop(&vms, &started).await;
    destroy(&vms, "local");
}

/// Acceptance 1 and 2 for a bundle: its code is copied into the image, and a
/// run neither installs nor mounts the package.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_bundle_builds_an_image_and_boots_it_without_install() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let package = t.join("project/.workers/probe");
    write(
        &package.join("iii.worker.yaml"),
        "name: probe\nscripts:\n  install: \"echo BUNDLE-INSTALL; echo built > /workspace/built.txt\"\n  start: \"sh run.sh\"\n",
    );
    write(
        &package.join("run.sh"),
        r#"echo "BUNDLE=$(cat /workspace/built.txt) CODE=$(cat /workspace/code.txt)"
echo "MOUNTED=$(awk '$5 == "/workspace"' /proc/self/mountinfo | wc -l)"
echo READY
exec sleep 600
"#,
    );
    write(&package.join("code.txt"), "bundle-code");
    let source = WorkerSource::Bundle {
        package: "probe".into(),
        host_path: package.display().to_string(),
    };
    let out = t.join("project/.build/probe");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, mut frames, _) = channel_engine().await;

    let built = build(&vms, &iii, "bundle", source.clone(), &out)
        .await
        .unwrap();
    eprintln!("[build] response: {built:?}");
    eprintln!("[build] {}: {:?}", out.display(), names(&out));
    assert_eq!(names(&out), ["image.erofs", "image.id"]);
    assert_eq!(
        std::fs::read_to_string(out.join("image.id"))
            .unwrap()
            .trim(),
        built.build_id
    );
    assert_eq!(sha256(&out.join("image.erofs")), built.build_id);
    assert_eq!(built.base_image.reference, "docker.io/iiidev/node:latest");
    let (records, binary) = drain(&mut frames);
    assert!(lines(&records).contains("BUNDLE-INSTALL"));
    assert_binary_frames_are_keepalives(&binary);

    let started = start(&vms, "bundle", source, Some((&built.build_id, &out)))
        .await
        .unwrap();
    let log = wait_for(&console(t, "bundle"), "READY", 120).await;
    eprintln!("[run] {log}");
    assert!(log.contains("BUNDLE=built CODE=bundle-code"), "{log}");
    assert!(log.contains("MOUNTED=0"), "the package is mounted: {log}");
    assert!(!log.contains("BUNDLE-INSTALL"), "the run installed: {log}");
    stop(&vms, &started).await;
    destroy(&vms, "bundle");
}

/// Acceptance 2: S508 for a missing or another image id, S509 for a second
/// build of one state key, S507 for an install that fails.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_build_codes() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "if [ -f slow ]; then echo SLOW-STARTED; sleep 20; fi; if [ -f fail ]; then echo before-fail; exit 3; fi; echo ok",
        "echo READY; exec sleep 600\n",
    );
    let out = project.join(".build/e2e");
    let vms = Arc::new(WorkerVms::new(
        t.join("workers"),
        Some("t".into()),
        vec![t.into()],
    ));
    let (iii, mut frames, _) = channel_engine().await;

    // S508: no image.id yet.
    std::fs::create_dir_all(&out).unwrap();
    let missing = start(&vms, "codes", local(&project), Some(("sha256:00", &out))).await;
    let missing = missing.unwrap_err();
    eprintln!("[S508 missing] {missing}");
    assert_eq!(missing.code().as_str(), "S508");

    build(&vms, &iii, "codes", local(&project), &out)
        .await
        .unwrap();
    // S508: another build's id.
    let other = format!("sha256:{}", "0".repeat(64));
    let mismatch = start(&vms, "codes", local(&project), Some((&other, &out)))
        .await
        .unwrap_err();
    eprintln!("[S508 mismatch] {mismatch}");
    assert_eq!(mismatch.code().as_str(), "S508");

    // S509: a second build of the key while the first one runs.
    write(&project.join("slow"), "");
    let first = {
        let (vms, iii, project, out) = (vms.clone(), iii.clone(), project.clone(), out.clone());
        tokio::spawn(async move { build(&vms, &iii, "codes", local(&project), &out).await })
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seen = Vec::new();
    while !lines(&seen).contains("SLOW-STARTED") {
        assert!(Instant::now() < deadline, "the slow build never started");
        tokio::time::sleep(Duration::from_millis(200)).await;
        seen.extend(drain(&mut frames).0);
    }
    let busy = build(&vms, &iii, "codes", local(&project), &out)
        .await
        .unwrap_err();
    eprintln!("[S509] {busy}");
    assert_eq!(busy.code().as_str(), "S509");
    let first = first.await.unwrap().unwrap();
    std::fs::remove_file(project.join("slow")).unwrap();

    // S507: the install fails; the output tail comes along, the image stays.
    write(&project.join("fail"), "");
    let failed = build(&vms, &iii, "codes", local(&project), &out)
        .await
        .unwrap_err();
    eprintln!("[S507] {failed}");
    assert_eq!(failed.code().as_str(), "S507");
    let message = failed.to_string();
    assert!(
        message.contains("exited 3") && message.contains("before-fail"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(out.join("image.id"))
            .unwrap()
            .trim(),
        first.build_id
    );
    assert_eq!(names(&out), ["image.erofs", "image.id"]);

    // The last good build still boots.
    std::fs::remove_file(project.join("fail")).unwrap();
    let started = start(
        &vms,
        "codes",
        local(&project),
        Some((&first.build_id, &out)),
    )
    .await
    .unwrap();
    wait_for(&console(t, "codes"), "READY", 120).await;
    stop(&vms, &started).await;
    destroy(&vms, "codes");
}

/// Acceptance 3: the reader of `logs` leaves mid-build. The build VM stops, the
/// temporary image goes, the previous image and id stay, and the key is free.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn closing_the_logs_reader_cancels_the_build() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "if [ -f slow ]; then echo SLOW-STARTED; sleep 120; fi; echo ok",
        "echo READY; exec sleep 600\n",
    );
    let out = project.join(".build/e2e");
    let vms = Arc::new(WorkerVms::new(
        t.join("workers"),
        Some("t".into()),
        vec![t.into()],
    ));
    let (iii, mut frames, leave) = channel_engine().await;
    let previous = build(&vms, &iii, "cancel", local(&project), &out)
        .await
        .unwrap();
    drain(&mut frames);

    write(&project.join("slow"), "");
    let slow = {
        let (vms, iii, project, out) = (vms.clone(), iii.clone(), project.clone(), out.clone());
        tokio::spawn(async move { build(&vms, &iii, "cancel", local(&project), &out).await })
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    let (mut records, mut binary) = (Vec::new(), Vec::new());
    while !lines(&records).contains("SLOW-STARTED") {
        assert!(Instant::now() < deadline, "the slow build never started");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let (r, b) = drain(&mut frames);
        records.extend(r);
        binary.extend(b);
    }
    // A quiet build still writes: one keepalive within 5 s, no record.
    let quiet = records.len();
    tokio::time::sleep(Duration::from_secs(7)).await;
    let (r, b) = drain(&mut frames);
    records.extend(r);
    binary.extend(b);
    eprintln!(
        "[keepalive] binary frames {binary:?}, records since the install went quiet: {}",
        records.len() - quiet
    );
    assert!(binary.iter().any(|f| f == &[0]), "no keepalive: {binary:?}");
    assert_binary_frames_are_keepalives(&binary);
    assert!(
        records[quiet..].iter().all(|r| !r.line.is_empty()),
        "a keepalive came through as a record"
    );

    let left = Instant::now();
    leave.notify_waiters();
    let cancelled = tokio::time::timeout(Duration::from_secs(30), slow)
        .await
        .expect("the build outlived its reader by 30 s")
        .unwrap()
        .unwrap_err();
    eprintln!("[cancel] after {:?}: {cancelled}", left.elapsed());
    assert_eq!(cancelled.code().as_str(), "S507");
    assert!(cancelled.to_string().contains("cancelled"), "{cancelled}");
    // The previous pair, and nothing else.
    assert_eq!(names(&out), ["image.erofs", "image.id"]);
    assert_eq!(
        std::fs::read_to_string(out.join("image.id"))
            .unwrap()
            .trim(),
        previous.build_id
    );
    assert_eq!(sha256(&out.join("image.erofs")), previous.build_id);
    // The build VM is gone with its scratch dir.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let vm_dir = t.join("workers").join(vm_id("cancel"));
    assert!(
        names(&vm_dir).iter().all(|n| !n.starts_with("build-")),
        "{:?}",
        names(&vm_dir)
    );

    // The key is free: the retry builds at once, no S509.
    std::fs::remove_file(project.join("slow")).unwrap();
    let retried = build(&vms, &iii, "cancel", local(&project), &out)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(out.join("image.id"))
            .unwrap()
            .trim(),
        retried.build_id
    );
}

/// Acceptance 4: what the VM writes survives a restart and a rebuild, which
/// swaps the lower under the kept upper; the overlay has none of the features
/// that forbid that.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn data_survives_restart_and_rebuild() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "v=$(cat version.txt); mkdir -p /opt/deps && echo $v > /opt/deps/version && echo from-build-$v > /opt/deps/shared",
        r#"mkdir -p /var/lib/iii-test
echo boot >> /var/lib/iii-test/boots
echo "BOOTS=$(wc -l < /var/lib/iii-test/boots)"
echo "VERSION=$(cat /opt/deps/version)"
echo "SHARED=$(cat /opt/deps/shared)"
[ -f /var/lib/iii-test/touched ] || { echo from-worker > /opt/deps/shared; touch /var/lib/iii-test/touched; }
sync
echo "MOUNT=$(awk '$2 == "/" {print $4}' /proc/mounts)"
echo READY
exec sleep 600
"#,
    );
    write(&project.join("version.txt"), "v1");
    let out = project.join(".build/e2e");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, _frames, _) = channel_engine().await;
    let console = console(t, "data");

    let v1 = build(&vms, &iii, "data", local(&project), &out)
        .await
        .unwrap();
    let started = start(&vms, "data", local(&project), Some((&v1.build_id, &out)))
        .await
        .unwrap();
    let log = wait_for(&console, "READY", 120).await;
    eprintln!("[boot 1] {log}");
    assert!(
        log.contains("BOOTS=1\nVERSION=v1\nSHARED=from-build-v1"),
        "{log}"
    );
    let mount = log.lines().find(|l| l.starts_with("MOUNT=")).unwrap();
    for on in [
        "index=on",
        "metacopy=on",
        "redirect_dir=on",
        "redirect_dir=follow",
        "xino=on",
        "xino=auto",
    ] {
        assert!(!mount.contains(on), "{on} in {mount}");
    }
    stop(&vms, &started).await;

    // Restart on the same build.
    let started = start(&vms, "data", local(&project), Some((&v1.build_id, &out)))
        .await
        .unwrap();
    let log = wait_for(&console, "READY", 120).await;
    eprintln!("[boot 2] {log}");
    assert!(
        log.contains("BOOTS=2\nVERSION=v1\nSHARED=from-worker"),
        "{log}"
    );
    stop(&vms, &started).await;

    // Rebuild: a new lower under the upper that holds the worker's files.
    write(&project.join("version.txt"), "v2");
    let v2 = build(&vms, &iii, "data", local(&project), &out)
        .await
        .unwrap();
    assert_ne!(v1.build_id, v2.build_id);
    let started = start(&vms, "data", local(&project), Some((&v2.build_id, &out)))
        .await
        .unwrap();
    let log = wait_for(&console, "READY", 120).await;
    eprintln!("[boot 3, rebuilt] {log}");
    assert!(
        log.contains("BOOTS=3\nVERSION=v2\nSHARED=from-worker"),
        "{log}"
    );
    stop(&vms, &started).await;
    destroy(&vms, "data");
}

/// The guest's view of the seal: it mounts its root share again and looks.
const SEAL_PROBE: &str = r#"mkdir -p /mnt/r
if mount -t virtiofs /dev/root /mnt/r 2>/dev/null; then
  echo "ROOTSHARE=$(ls -A /mnt/r | tr '\n' ' ')"
  [ -e /mnt/r/build-mode ] && echo SEAL=visible || echo SEAL=hidden
  umount /mnt/r
else
  echo "ROOTSHARE=unmountable"
  echo SEAL=hidden
fi
echo READY
exec sleep 600
"#;

/// Acceptance 5: the disks a 0.26 adapter left are kept, their paths logged,
/// the seal is out of the guest's reach, and destroy removes everything.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_pre_build_disks_are_kept_and_destroy_removes_them() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(&project, "echo ok", SEAL_PROBE);
    let out = project.join(".build/e2e");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, _frames, _) = channel_engine().await;
    let built = build(&vms, &iii, "overlay-026", local(&project), &out)
        .await
        .unwrap();

    // A 0.26 overlay VM: its disk is root/upper.ext4.
    let vm_dir = t.join("workers").join(vm_id("overlay-026"));
    let old_disk = sandbox_libkrun::cli::upper::ensure_upper_ext4(&vm_dir.join("root")).unwrap();
    // A rename keeps the inode: the kept disk is the very file (16 GiB sparse).
    let old_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&old_disk).unwrap());
    let started = start(
        &vms,
        "overlay-026",
        local(&project),
        Some((&built.build_id, &out)),
    )
    .await
    .unwrap();
    let log = wait_for(&console(t, "overlay-026"), "READY", 120).await;
    eprintln!("[0.26 overlay] {log}");
    let kept = vm_dir.join("upper.ext4.pre-build");
    assert!(
        log.contains(&format!(
            "w: kept the pre-build data disk at {}",
            kept.display()
        )),
        "{log}"
    );
    let kept_ino = std::os::unix::fs::MetadataExt::ino(&std::fs::metadata(&kept).unwrap());
    assert_eq!(kept_ino, old_ino);
    assert!(!vm_dir.join("root/upper.ext4").exists());
    assert!(vm_dir.join("upper.ext4").is_file() && vm_dir.join("build-mode").is_file());
    assert!(log.contains("SEAL=hidden"), "{log}");
    eprintln!("[0.26 overlay] {}: {:?}", vm_dir.display(), names(&vm_dir));
    stop(&vms, &started).await;
    destroy(&vms, "overlay-026");
    assert!(!vm_dir.exists(), "destroy left {:?}", names(&vm_dir));

    // A legacy VM: root/ is the rootfs clone, the guest's data in it.
    let vm_dir = t.join("workers").join(vm_id("legacy-026"));
    for dir in ["root/bin", "root/usr/bin", "root/var/lib/app"] {
        std::fs::create_dir_all(vm_dir.join(dir)).unwrap();
    }
    write(&vm_dir.join("root/var/lib/app/db"), "rows");
    let started = start(
        &vms,
        "legacy-026",
        local(&project),
        Some((&built.build_id, &out)),
    )
    .await
    .unwrap();
    let log = wait_for(&console(t, "legacy-026"), "READY", 120).await;
    eprintln!("[0.26 legacy] {log}");
    let kept = vm_dir.join("root.pre-build");
    assert!(
        log.contains(&format!(
            "w: kept the pre-build data disk at {}",
            kept.display()
        )),
        "{log}"
    );
    assert_eq!(
        std::fs::read_to_string(kept.join("var/lib/app/db")).unwrap(),
        "rows"
    );
    assert!(!vm_dir.join("root/bin").exists());
    assert!(log.contains("SEAL=hidden"), "{log}");
    eprintln!("[0.26 legacy] {}: {:?}", vm_dir.display(), names(&vm_dir));
    stop(&vms, &started).await;
    destroy(&vms, "legacy-026");
    assert!(!vm_dir.exists());
}

/// The build fills dependency dirs beyond the usual few; a run never installs,
/// so they must survive every boot's cleanup, and the host's copies of them
/// never reach the guest. `vendor/` alone goes into the build (Go vendors source
/// there), and still stays out of the run's copy.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn dependency_dirs_the_build_fills_survive_the_boots() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "[ -e venv/bin/marker ] && echo HOST-VENV-REACHED-BUILD; mkdir -p venv/bin vendor/pkg && echo built-venv > venv/bin/marker && echo built-vendor > vendor/pkg/a.php && cat vendor/keep.go > vendor/copied.txt",
        r#"echo "VENV=$(cat /workspace/venv/bin/marker) VENDOR=$(cat /workspace/vendor/pkg/a.php) COPIED=$(cat /workspace/vendor/copied.txt)"
echo READY
exec sleep 600
"#,
    );
    write(&project.join("venv/bin/marker"), "host-venv");
    write(&project.join("vendor/keep.go"), "host-vendored-src");
    write(&project.join("vendor/pkg/a.php"), "host-php");
    let out = project.join(".build/e2e");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, mut frames, _) = channel_engine().await;
    let built = build(&vms, &iii, "deps", local(&project), &out)
        .await
        .unwrap();
    let log = lines(&drain(&mut frames).0);
    assert!(!log.contains("HOST-VENV-REACHED-BUILD"), "{log}");

    let console = console(t, "deps");
    let want = "VENV=built-venv VENDOR=built-vendor COPIED=host-vendored-src";
    for boot in 1..=2 {
        let started = start(&vms, "deps", local(&project), Some((&built.build_id, &out)))
            .await
            .unwrap();
        let log = wait_for(&console, "READY", 120).await;
        eprintln!("[boot {boot}] {log}");
        assert!(log.contains(want), "boot {boot}: {log}");
        stop(&vms, &started).await;
        // A host edit of a dependency dir never reaches the guest.
        write(&project.join("venv/bin/marker"), "host-venv-2");
        write(&project.join("vendor/pkg/a.php"), "host-php-2");
    }
    destroy(&vms, "deps");
}

/// A start that fails after it moved a 0.3 disk aside still names where the
/// disk went: the seal is written, so no later start will. No VM is needed:
/// the boot fails at the spawn.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn a_boot_that_fails_after_the_migration_still_names_the_kept_disk() {
    if !iii_filesystem::init::has_init() {
        eprintln!("[skip] no embedded iii-init: a build cannot boot");
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(&project, "echo ok", "echo READY\n");
    let out = project.join(".build/e2e");
    let id = format!("sha256:{}", "a".repeat(64));
    write(&out.join("image.erofs"), "not booted");
    write(&out.join("image.id"), &format!("{id}\n"));
    let vm_dir = t.join("workers").join(vm_id("fails"));
    write(&vm_dir.join("root/upper.ext4"), "0.26 disk");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);

    // SAFETY: the tests of this binary run one at a time (#[serial]); the
    // next one sets it back.
    unsafe { std::env::set_var("III_TEST_VM_BOOT_BIN", t.join("no-such-vm-boot")) };
    let failed = start(&vms, "fails", local(&project), Some((&id, &out)))
        .await
        .unwrap_err();
    unsafe {
        std::env::set_var(
            "III_TEST_VM_BOOT_BIN",
            env!("CARGO_BIN_EXE_sandbox-libkrun"),
        )
    };
    eprintln!("[failed boot] {failed}");
    assert_eq!(failed.code().as_str(), "S300");
    let kept = vm_dir.join("upper.ext4.pre-build");
    assert!(
        failed.to_string().contains(&format!(
            "w: kept the pre-build data disk at {}",
            kept.display()
        )),
        "{failed}"
    );
    assert_eq!(std::fs::read_to_string(&kept).unwrap(), "0.26 disk");
    assert!(vm_dir.join("build-mode").is_file());
}

/// A boot's cleanup removes only what the last copy put in /workspace and the
/// host deleted: what the worker wrote there survives a restart, and what the
/// build installed in a nested `dist/` or `target/` survives every boot. A root
/// `build` script is code: it reaches the build and the run.
#[tokio::test(flavor = "multi_thread")]
#[serial]
async fn the_workspace_keeps_what_the_build_and_the_worker_wrote() {
    if !kvm() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let t = t.path();
    let project = t.join("project");
    python_worker(
        &project,
        "sh build && mkdir -p packages/a/dist tools/cli/target/release && echo built-dist > packages/a/dist/index.js && echo built-linux > tools/cli/target/release/cli",
        r#"mkdir -p /var/lib/iii-test
echo boot >> /var/lib/iii-test/boots
n=$(wc -l < /var/lib/iii-test/boots)
[ -f /workspace/data.db ] || echo "written-at-boot-$n" > /workspace/data.db
echo "DATA=$(cat /workspace/data.db) DIST=$(cat /workspace/packages/a/dist/index.js) CLI=$(cat /workspace/tools/cli/target/release/cli) OLD=$(cat /workspace/old.txt 2>/dev/null || echo gone) SCRIPT=$(head -n 1 /workspace/build)"
echo READY
exec sleep 600
"#,
    );
    write(
        &project.join("build"),
        "#!/bin/sh\necho ROOT-BUILD-SCRIPT-RAN\n",
    );
    write(&project.join("old.txt"), "old-content");
    write(&project.join("tools/cli/target/release/cli"), "host-macos");
    let out = project.join(".build/e2e");
    let vms = WorkerVms::new(t.join("workers"), Some("t".into()), vec![t.into()]);
    let (iii, mut frames, _) = channel_engine().await;
    let built = build(&vms, &iii, "keep", local(&project), &out)
        .await
        .unwrap();
    let log = lines(&drain(&mut frames).0);
    assert!(log.contains("ROOT-BUILD-SCRIPT-RAN"), "{log}");

    let console = console(t, "keep");
    let base = "DIST=built-dist CLI=built-linux";
    for (boot, old) in [(1, "old-content"), (2, "gone")] {
        let started = start(&vms, "keep", local(&project), Some((&built.build_id, &out)))
            .await
            .unwrap();
        let log = wait_for(&console, "READY", 120).await;
        eprintln!("[boot {boot}] {log}");
        let want = format!("DATA=written-at-boot-1 {base} OLD={old} SCRIPT=#!/bin/sh");
        assert!(log.contains(&want), "boot {boot}: {log}");
        assert!(!log.contains("no workspace manifest yet"), "{log}");
        stop(&vms, &started).await;
        // A host deletion leaves the guest at the next boot.
        std::fs::remove_file(project.join("old.txt")).ok();
    }
    destroy(&vms, "keep");
}
