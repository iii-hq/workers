// Copyright Motia LLC and/or licensed to Motia LLC under one or more
// contributor license agreements. Licensed under the Elastic License 2.0;
// you may not use this file except in compliance with the Elastic License 2.0.
// This software is patent protected. We welcome discussions - reach out at team@iii.dev
// See LICENSE and PATENTS files for details.

//! libkrun VM runtime for worker VMs and sandboxes.
//!
//! Provides VM-based isolated execution using libkrun (Apple Hypervisor.framework
//! on macOS, KVM on Linux). The VM runs in a separate helper process
//! for crash isolation.

use anyhow::Result;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Forward optional VM-boot tuning flags to the `__vm-boot` child
/// based on opt-in environment variables. Keeps the public API of
/// `build_vm_command` unchanged while giving
/// operators a single place to enable hyperthreading, nested
/// virtualization, virtiofs DAX window tuning, and the worker-side
/// NOFILE rlimit for perf experiments.
///
/// Variables read (all optional; omit to take each CLI default):
///   - `III_VM_HYPERTHREADING=1|true|on|yes` → `--hyperthreading`
///   - `III_VM_NESTED_VIRT=1|true|on|yes`   → `--nested-virt`
///   - `III_VM_VIRTIOFS_SHM_SIZE_MIB=<int>` → `--virtiofs-shm-size-mib <n>` (0 = skip)
///   - `III_VM_NOFILE_LIMIT=<int>`     → `--nofile-limit <n>` (0 = let iii-init own it)
///
/// Bad values are ignored silently (a typo in an opt-in perf flag
/// should never fail a worker boot). Appending args to both
/// `std::process::Command` and `tokio::process::Command` needs the
/// same logic, so we take `&mut CommandArgsExt` via the closure
/// rather than committing to either concrete type.
fn apply_vm_tuning_env(mut push: impl FnMut(&str, Option<&str>)) {
    let parse_bool =
        |v: &str| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "on" | "yes");

    if std::env::var("III_VM_HYPERTHREADING")
        .ok()
        .filter(|v| parse_bool(v))
        .is_some()
    {
        push("--hyperthreading", None);
    }
    if std::env::var("III_VM_NESTED_VIRT")
        .ok()
        .filter(|v| parse_bool(v))
        .is_some()
    {
        push("--nested-virt", None);
    }
    if let Some(n) = std::env::var("III_VM_VIRTIOFS_SHM_SIZE_MIB")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
    {
        let owned = n.to_string();
        push("--virtiofs-shm-size-mib", Some(&owned));
    }
    if let Some(n) = std::env::var("III_VM_NOFILE_LIMIT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
    {
        let owned = n.to_string();
        push("--nofile-limit", Some(&owned));
    }
}

/// msb_krun (the VMM) is compiled into the binary; this checks for libkrunfw.
pub fn libkrun_available() -> bool {
    crate::cli::firmware::resolve::resolve_libkrunfw_dir().is_some()
}

/// Build the VM boot env. Launcher wins: `III_ISOLATION=libkrun` is written
/// after caller env so an OCI image `ENV III_ISOLATION=docker` cannot override it.
pub(crate) fn build_vm_env(caller_env: HashMap<String, String>) -> HashMap<String, String> {
    let mut merged = HashMap::with_capacity(caller_env.len() + 1);
    for (key, value) in caller_env {
        merged.insert(key, value);
    }
    merged.insert("III_ISOLATION".to_string(), "libkrun".to_string());
    merged
}

/// A `__vm-boot` invocation, built but not spawned.
///
/// The caller spawns it and keeps the handle, to stop the VM and to see it
/// crash.
pub struct VmCommand {
    pub command: tokio::process::Command,
    /// The worker env, as the `__vm-boot --env-stdin` frame. The command
    /// carries no env values in argv, where `ps` would show them to every
    /// local user (MOT-5492): whoever spawns it must write this to the
    /// child's stdin and close it (see [`feed_env_stdin`]).
    pub env_stdin: Vec<u8>,
    /// Cooperative normal-mode guards, retained by the owning worker waiter.
    /// They are not compatibility authority and cannot boot a retained root.
    pub(crate) lifecycle_leases: Vec<crate::worker_admission::Lease>,
    /// Exact prepared writable paths; activate only after host control writes.
    pub(crate) runtime_paths: Vec<(PathBuf, crate::worker_admission::RuntimePathKind)>,
    /// This builder currently launches no shell relay. Future relay flags must
    /// be added together with their exact declared endpoint, never by scanning.
    pub(crate) runtime_endpoints: Vec<PathBuf>,
}

/// Writes [`VmCommand::env_stdin`] into a spawned `__vm-boot` and closes the
/// pipe. `__vm-boot` reads the frame before anything else, so this returns
/// as soon as the frame is in the pipe.
pub async fn feed_env_stdin(
    stdin: Option<tokio::process::ChildStdin>,
    frame: &[u8],
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;

    let mut stdin =
        stdin.ok_or_else(|| std::io::Error::other("the VM boot process has no stdin pipe"))?;
    stdin.write_all(frame).await?;
    stdin.shutdown().await
}

/// The binary that answers `__vm-boot`: always the running one, so the VMM,
/// init and firmware come from one build. Searching `PATH` would let a stray
/// older copy boot the VM.
fn vm_boot_program() -> Result<PathBuf, String> {
    // Test-only: an integration test that drives `WorkerVms` in process runs
    // in a test binary, which cannot answer `__vm-boot`.
    #[cfg(feature = "integration-vm")]
    if let Some(bin) = std::env::var_os("III_TEST_VM_BOOT_BIN") {
        return Ok(bin.into());
    }
    std::env::current_exe().map_err(|e| format!("cannot locate the running binary: {e}"))
}

/// Builds the `__vm-boot` command for one VM. No process is started.
#[allow(clippy::too_many_arguments)]
pub fn build_vm_command(
    exec_path: &str,
    args: &[String],
    env: HashMap<String, String>,
    vcpus: u32,
    ram_mib: u32,
    rootfs: PathBuf,
    rootfs_lower: Option<PathBuf>,
    rootfs_mode: String,
    rootfs_upper: Option<PathBuf>,
    mounts: &[(String, String)],
    source_sync: Option<&Path>,
    source_readonly: Option<&Path>,
) -> Result<VmCommand, String> {
    let mount_args: Vec<_> = mounts
        .iter()
        .map(|(host, guest)| format!("{host}:{guest}"))
        .collect();
    let (admission, _) = crate::cli::vm_boot::admit_boot_inputs(
        &rootfs,
        rootfs_lower.as_deref(),
        rootfs_upper.as_deref(),
        &mount_args,
        source_sync,
        source_readonly,
    )?;
    build_vm_command_admitted(
        admission,
        exec_path,
        args,
        env,
        vcpus,
        ram_mib,
        rootfs,
        rootfs_lower,
        rootfs_mode,
        rootfs_upper,
        mounts,
        source_sync,
        source_readonly,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_vm_command_admitted(
    admission: crate::worker_admission::Lease,
    exec_path: &str,
    args: &[String],
    env: HashMap<String, String>,
    vcpus: u32,
    ram_mib: u32,
    rootfs: PathBuf,
    rootfs_lower: Option<PathBuf>,
    rootfs_mode: String,
    rootfs_upper: Option<PathBuf>,
    mounts: &[(String, String)],
    source_sync: Option<&Path>,
    source_readonly: Option<&Path>,
) -> Result<VmCommand, String> {
    // Independent callers get the same parser and fences as __vm-boot,
    // before even platform provisioning (codesigning on macOS).
    let mount_args: Vec<String> = mounts
        .iter()
        .map(|(host, guest)| format!("{host}:{guest}"))
        .collect();
    let (admission, _) = crate::cli::vm_boot::admit_boot_inputs_on(
        admission,
        &rootfs,
        rootfs_lower.as_deref(),
        rootfs_upper.as_deref(),
        &mount_args,
        source_sync,
        source_readonly,
    )?;
    let runtime_paths = runtime_paths(
        &rootfs,
        rootfs_upper.as_deref(),
        &mount_args,
        source_sync,
        source_readonly,
    )?;
    let env = build_vm_env(env);

    let self_exe = vm_boot_program()?;

    #[cfg(target_os = "macos")]
    {
        if let Err(e) = super::platform::ensure_macos_entitlements(&self_exe) {
            eprintln!(
                "warning: failed to codesign for Hypervisor entitlement: {}",
                e
            );
        }
    }

    let env_pairs: Vec<(String, String)> = env.into_iter().collect();
    let env_stdin = iii_supervisor::env_channel::encode_frame(&env_pairs)
        .map_err(|e| format!("cannot pass the worker environment to the VM: {e}"))?;

    let mut cmd = tokio::process::Command::new(&self_exe);
    cmd.arg("__vm-boot");
    for boot_arg in vm_boot_args_dev(
        &rootfs,
        rootfs_lower.as_deref(),
        &rootfs_mode,
        rootfs_upper.as_deref(),
        exec_path,
        vcpus,
        ram_mib,
        source_sync,
        source_readonly,
        mounts,
        args,
    ) {
        cmd.arg(boot_arg);
    }

    apply_vm_tuning_env(|flag, val| {
        cmd.arg(flag);
        if let Some(v) = val {
            cmd.arg(v);
        }
    });

    if let Some(fw_dir) = crate::cli::firmware::resolve::resolve_libkrunfw_dir() {
        cmd.env(
            crate::cli::firmware::resolve::lib_path_env_var(),
            fw_dir.to_string_lossy().as_ref(),
        );
    }

    cmd.stdin(std::process::Stdio::piped());
    attach_child_admission(&mut cmd, &admission)?;

    // This builder does not emit --shell-sock (unlike the sandbox launcher).
    // Refuse accidental drift rather than settling an empty set for a relay.
    if cmd.as_std().get_args().any(|arg| {
        arg == "--shell-sock" || arg.to_str().is_some_and(|s| s.starts_with("--shell-sock="))
    }) {
        return Err("worker command added an undeclared runtime endpoint".into());
    }

    // Deliberately no `setsid` here: how the child is detached belongs to
    // whoever spawns it. A caller that supervises its own children puts them in
    // a process group it can signal, and `setsid` fails once the child is
    // already a group leader, and the spawn fails with it.
    Ok(VmCommand {
        command: cmd,
        env_stdin,
        lifecycle_leases: vec![admission],
        runtime_paths,
        runtime_endpoints: Vec::new(),
    })
}

fn runtime_paths(
    root: &Path,
    upper: Option<&Path>,
    mounts: &[String],
    source_sync: Option<&Path>,
    source_readonly: Option<&Path>,
) -> Result<Vec<(PathBuf, crate::worker_admission::RuntimePathKind)>, String> {
    use crate::worker_admission::RuntimePathKind;
    let mut paths = vec![(root.to_path_buf(), RuntimePathKind::GuestTree)];
    if let Some(upper) = upper {
        paths.push((upper.to_path_buf(), RuntimePathKind::UpperImage));
    }
    for mount in crate::cli::vm_boot::build_virtiofs_mount_plan(mounts)?.entries {
        let host = Path::new(&mount.host_path);
        if Some(host) == source_sync || Some(host) == source_readonly {
            continue;
        }
        // A child of root is already included by the exact root export.
        if !host.starts_with(root) {
            paths.push((host.to_path_buf(), RuntimePathKind::GuestTree));
        }
    }
    Ok(paths)
}

/// Select exactly one CLOEXEC duplicate; clear only its CHILD copy. The command
/// closure owns ChildAdmission until spawn/command disposal; the waiter retains
/// the same Lease through successful wait (or the existing uncertain-writer policy).
pub(crate) fn attach_child_admission(
    cmd: &mut tokio::process::Command,
    lease: &crate::worker_admission::Lease,
) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let child = lease.export_child().map_err(|e| e.to_string())?;
    let fd = child.duplicate_for_exec().map_err(|e| e.to_string())?;
    if fd.as_raw_fd() < 3 {
        return Err("admission duplicate overlaps standard IO descriptors".into());
    }
    cmd.arg("--admission-fd").arg(fd.as_raw_fd().to_string());
    unsafe {
        cmd.pre_exec(move || {
            let _retained = &child;
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFD);
            if flags < 0
                || libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

const VM_BOOT_NETWORK_FLAG: &str = "--network";
/// The env goes on stdin, never argv, where `ps` shows it (MOT-5492).
const VM_BOOT_ENV_STDIN_FLAG: &str = "--env-stdin";

/// Push `--rootfs-lower <sqfs> --rootfs-mode <mode>` when overlay mode is
/// active (i.e. the start path built a base erofs). No-op for legacy.
fn push_overlay_args(
    out: &mut Vec<OsString>,
    rootfs_lower: Option<&Path>,
    rootfs_mode: &str,
    rootfs_upper: Option<&Path>,
) {
    if let Some(lower) = rootfs_lower {
        out.push(OsString::from("--rootfs-lower"));
        out.push(lower.as_os_str().to_owned());
        out.push(OsString::from("--rootfs-mode"));
        out.push(OsString::from(rootfs_mode));
        // The upper is optional even in overlay mode: dev builds without an
        // embedded golden image attach no upper and iii-init falls back to a
        // tmpfs upper.
        if let Some(upper) = rootfs_upper {
            out.push(OsString::from("--rootfs-upper"));
            out.push(upper.as_os_str().to_owned());
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn vm_boot_args_dev(
    rootfs: &Path,
    rootfs_lower: Option<&Path>,
    rootfs_mode: &str,
    rootfs_upper: Option<&Path>,
    exec_path: &str,
    vcpus: u32,
    ram_mib: u32,
    source_sync: Option<&Path>,
    source_readonly: Option<&Path>,
    mounts: &[(String, String)],
    exec_args: &[String],
) -> Vec<OsString> {
    let mut out: Vec<OsString> = Vec::new();
    out.push(OsString::from("--rootfs"));
    out.push(rootfs.as_os_str().to_owned());
    push_overlay_args(&mut out, rootfs_lower, rootfs_mode, rootfs_upper);
    out.push(OsString::from("--exec"));
    out.push(OsString::from(exec_path));
    out.push(OsString::from("--workdir"));
    out.push(OsString::from("/workspace"));
    out.push(OsString::from("--vcpus"));
    out.push(OsString::from(vcpus.to_string()));
    out.push(OsString::from("--ram"));
    out.push(OsString::from(ram_mib.to_string()));
    if let Some(source_sync) = source_sync {
        out.push(OsString::from("--source-sync"));
        out.push(source_sync.as_os_str().to_owned());
    }
    if let Some(source) = source_readonly {
        out.push(OsString::from("--source-readonly"));
        out.push(source.as_os_str().to_owned());
    }
    out.push(OsString::from(VM_BOOT_NETWORK_FLAG));
    out.push(OsString::from(VM_BOOT_ENV_STDIN_FLAG));
    for (host, guest) in mounts {
        out.push(OsString::from("--mount"));
        out.push(OsString::from(format!("{}:{}", host, guest)));
    }
    for arg in exec_args {
        out.push(OsString::from("--arg"));
        out.push(OsString::from(arg.as_str()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_builder_refuses_mount_and_source_exports_before_provisioning() {
        use std::os::unix::fs::DirBuilderExt;
        let t = crate::worker_admission::testing::private_tempdir().unwrap();
        let base = t.path().join("workers");
        let id = "d".repeat(32);
        // The namespace must be exactly 0700, never the umask-derived default.
        let namespace = crate::worker_admission::testing::private_tempdir().unwrap();
        let context =
            crate::worker_admission::testing::provision_context(namespace.path()).unwrap();
        // `base` is a worker base this namespace knows, as a production base is
        // once a worker operation registered it. That operation ends before
        // the retained VM below is planted.
        drop(context.worker(&base, Some("registration")).unwrap());
        let retained = base.join(&id).join("root");
        std::fs::create_dir_all(&retained).unwrap();
        let custody = crate::worker_admission::custody(&base, &id).unwrap();
        // Core reads a custody registry only when it is private (exactly 0700).
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&custody)
            .unwrap();
        std::fs::write(custody.join("active.json"), b"{corrupt").unwrap();
        let root = t.path().join("normal-root");
        for field in ["lower", "upper", "mount", "sync", "readonly"] {
            let mounts = if field == "mount" {
                vec![(retained.display().to_string(), "/workspace".to_string())]
            } else {
                Vec::new()
            };
            let lower = (field == "lower").then(|| retained.join("image.erofs"));
            let upper = (field == "upper").then(|| retained.join("upper.ext4"));
            let mount_args: Vec<_> = mounts
                .iter()
                .map(|(host, guest)| format!("{host}:{guest}"))
                .collect();
            let error = crate::cli::vm_boot::admit_boot_inputs_fresh(
                context.local(&root).unwrap(),
                &root,
                lower.as_deref(),
                upper.as_deref(),
                &mount_args,
                (field == "sync").then_some(retained.as_path()),
                (field == "readonly").then_some(retained.as_path()),
            )
            .err()
            .unwrap();
            assert!(error.contains("compatibility custody"), "{field}: {error}");
            assert!(!root.exists());
        }
    }

    #[test]
    fn child_descriptor_is_cloexec_and_imports_the_same_operation() {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let namespace = t.path().join("catalog");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o700)).unwrap();
        let context = crate::worker_admission::testing::provision_context(&namespace).unwrap();
        let root = t.path().join("root");
        let lease = context.local(&root).unwrap();
        let lease = crate::worker_admission::with_paths(
            lease,
            &[crate::worker_admission::PathRequest {
                path: &root,
                role: crate::worker_admission::PathRole::MutableTree,
            }],
        )
        .unwrap();
        let export = lease.export_child().unwrap();
        let fd = export.duplicate_for_exec().unwrap();
        assert_ne!(
            unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) } & libc::FD_CLOEXEC,
            0
        );
        let imported = context.import_child(fd).unwrap();
        let mut mutation = imported
            .begin(crate::worker_admission::MutationPlan::Create { root: root.clone() })
            .unwrap();
        mutation.create_dir_all(&root, 0o700).unwrap();
        mutation.commit().unwrap();
        lease.revalidate().unwrap();
        imported.revalidate().unwrap();
        drop(export);
        assert!(root.is_dir());
    }

    #[test]
    fn runtime_footprint_matches_writable_exports_and_excludes_sources() {
        use crate::worker_admission::RuntimePathKind;
        let paths = runtime_paths(
            Path::new("/owned/root"),
            Some(Path::new("/owned/upper.ext4")),
            &[
                "/project:/mnt/host-src".into(),
                "/owned/root/runtime:/opt/iii".into(),
                "/owned/data:/data".into(),
            ],
            Some(Path::new("/project")),
            None,
        )
        .unwrap();
        assert_eq!(paths.len(), 3);
        assert!(
            paths
                .iter()
                .any(|(p, k)| p == Path::new("/owned/root")
                    && matches!(k, RuntimePathKind::GuestTree))
        );
        assert!(
            paths
                .iter()
                .any(|(p, k)| p == Path::new("/owned/upper.ext4")
                    && matches!(k, RuntimePathKind::UpperImage))
        );
        assert!(
            paths
                .iter()
                .any(|(p, k)| p == Path::new("/owned/data")
                    && matches!(k, RuntimePathKind::GuestTree))
        );
        assert!(!paths.iter().any(|(p, _)| p == Path::new("/project")));
    }

    #[test]
    fn vm_boot_program_is_the_running_binary() {
        assert_eq!(vm_boot_program().unwrap(), std::env::current_exe().unwrap());
    }

    fn parse_vm_boot_args(boot_args: Vec<OsString>) -> crate::cli::vm_boot::VmBootArgs {
        use clap::Parser;

        #[derive(Parser)]
        struct Wrapper {
            #[command(flatten)]
            args: crate::cli::vm_boot::VmBootArgs,
        }

        let mut argv: Vec<OsString> = vec![OsString::from("test")];
        argv.extend(boot_args);
        Wrapper::parse_from(argv).args
    }

    #[test]
    fn vm_boot_args_dev_enables_network_and_roundtrips() {
        let mounts = vec![("/host/src".to_string(), "/guest/src".to_string())];
        let exec_args = vec!["-c".to_string(), "exec bash /run.sh".to_string()];

        let parsed = parse_vm_boot_args(vm_boot_args_dev(
            Path::new("/tmp/rootfs"),
            None,
            "",
            None,
            "/bin/sh",
            2,
            2048,
            Some(Path::new("/host/src")),
            None,
            &mounts,
            &exec_args,
        ));

        assert!(parsed.network);
        assert_eq!(parsed.rootfs, "/tmp/rootfs");
        assert_eq!(parsed.exec, "/bin/sh");
        assert_eq!(parsed.workdir, "/workspace");
        assert_eq!(parsed.vcpus, 2);
        assert_eq!(parsed.ram, 2048);
        // No shell channel: nothing on the host execs into a worker VM.
        assert_eq!(parsed.shell_sock, None);
        assert_eq!(parsed.source_sync.as_deref(), Some("/host/src"));
        assert_eq!(parsed.source_readonly, None);
        assert!(parsed.env.is_empty());
        assert!(parsed.env_stdin);
        assert_eq!(parsed.mount, vec!["/host/src:/guest/src".to_string()]);
        assert_eq!(parsed.arg, exec_args);
    }

    #[test]
    fn a_read_only_source_reaches_vm_boot_without_a_sync() {
        let parsed = parse_vm_boot_args(vm_boot_args_dev(
            Path::new("/tmp/rootfs"),
            None,
            "",
            None,
            "/bin/sh",
            2,
            2048,
            None,
            Some(Path::new("/host/src")),
            &[("/host/src".to_string(), "/mnt/host-src".to_string())],
            &[],
        ));
        assert_eq!(parsed.source_sync, None);
        assert_eq!(parsed.source_readonly.as_deref(), Some("/host/src"));
    }

    #[tokio::test]
    async fn feed_env_stdin_writes_the_frame_and_closes_the_pipe() {
        let mut child = tokio::process::Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let env = vec![("API_KEY".to_string(), "sk-secret".to_string())];
        let frame = iii_supervisor::env_channel::encode_frame(&env).unwrap();
        feed_env_stdin(child.stdin.take(), &frame).await.unwrap();
        // `cat` only exits once it sees EOF, so this also proves the close.
        let out = child.wait_with_output().await.unwrap();
        assert_eq!(
            iii_supervisor::env_channel::read_frame(&mut out.stdout.as_slice()).unwrap(),
            env
        );
    }

    #[tokio::test]
    async fn feed_env_stdin_without_a_pipe_is_an_error() {
        assert!(feed_env_stdin(None, b"").await.is_err());
    }

    #[test]
    fn test_libkrun_available_returns_bool() {
        let result = libkrun_available();
        let _ = result;
    }

    #[test]
    fn build_vm_env_injects_isolation_marker_into_empty_input() {
        let merged = build_vm_env(HashMap::new());
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn build_vm_env_preserves_caller_vars_and_adds_isolation() {
        let mut caller = HashMap::new();
        caller.insert("NODE_ENV".to_string(), "production".to_string());
        caller.insert("III_URL".to_string(), "ws://127.0.0.1:3111".to_string());
        let merged = build_vm_env(caller);
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
        assert_eq!(merged.get("NODE_ENV"), Some(&"production".to_string()));
        assert_eq!(
            merged.get("III_URL"),
            Some(&"ws://127.0.0.1:3111".to_string())
        );
        assert_eq!(merged.len(), 3);
    }

    #[test]
    fn build_vm_env_launcher_overrides_caller_isolation() {
        let mut caller = HashMap::new();
        caller.insert("III_ISOLATION".to_string(), "docker".to_string());
        let merged = build_vm_env(caller);
        assert_eq!(merged.get("III_ISOLATION"), Some(&"libkrun".to_string()));
    }
}
